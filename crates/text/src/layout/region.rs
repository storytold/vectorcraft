#[cfg(test)]
use super::PathData;
use super::{BezPath, LayoutOptions, PathEl, Point, Rect, Shape};

/// A flattened text wrap shape: (polygons, offset, invert).
pub(super) type FlatWrap = (Vec<Vec<Point>>, f64, bool);

/// One cell of a flattened area-type frame (the whole frame, or one row/column of it).
pub(super) struct Region {
    /// The cell (frame bounds, or a grid cell of them).
    pub(super) cell: Rect,
    /// Inset applied inside the frame edges.
    inset: f64,
    pub(super) polys: std::sync::Arc<[Vec<Point>]>,
    /// The frame is its own bounding rectangle (spans need no polygon intersection).
    pub(super) rect: bool,
    /// Text Wrap shapes.
    pub(super) wraps: std::sync::Arc<[FlatWrap]>,
    /// Vertical alignment: how far below the top-aligned position the first line starts.
    pub(super) shift: f64,
    /// Vertical justification: extra space added between consecutive lines.
    pub(super) gap: f64,
}

/// The largest magnitude a coordinate of a frame, a text wrap shape or the path of type on a path
/// may have (points), control points included. A frame or path with a larger coordinate, or a
/// coordinate that isn't finite, holds no text, and a text wrap shape with one is ignored.
const MAX_COORD: f64 = 1e9;

/// Are the coordinates of `path`, control points included, finite and within [`MAX_COORD`]?
pub(super) fn in_range(path: &BezPath) -> bool {
    let near = |p: &Point| p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD;
    path.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => near(p),
        PathEl::QuadTo(a, p) => near(a) && near(p),
        PathEl::CurveTo(a, b, p) => near(a) && near(b) && near(p),
        PathEl::ClosePath => true,
    })
}

/// Flattened closed polygons of a path; `None` when it isn't [`in_range`].
fn polygons(path: &BezPath) -> Option<Vec<Vec<Point>>> {
    if !in_range(path) {
        return None;
    }
    let mut polys: Vec<Vec<Point>> = Vec::new();
    kurbo::flatten(path, 0.1, |el| match el {
        PathEl::MoveTo(p) => polys.push(vec![p]),
        PathEl::LineTo(p) => {
            if let Some(v) = polys.last_mut() {
                v.push(p);
            }
        }
        _ => {}
    });
    polys.retain(|p| p.len() >= 3);
    Some(polys)
}

/// Inside intervals (even-odd) of the horizontal line at `y` through `polys`.
fn poly_intervals(polys: &[Vec<Point>], y: f64) -> Vec<(f64, f64)> {
    let mut xs = Vec::new();
    for poly in polys {
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect()
}

/// The part of the horizontal line at `y` closer than `r` to the segment `a`–`b`: the line's
/// intersection with the segment's two end disks and the band between them, which together make
/// a convex shape, so the result is one interval.
fn near_segment(a: Point, b: Point, y: f64, r: f64) -> Option<(f64, f64)> {
    if y <= a.y.min(b.y) - r || y >= a.y.max(b.y) + r {
        return None;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in [a, b] {
        let h = r * r - (y - p.y) * (y - p.y);
        if h > 0.0 {
            lo = lo.min(p.x - h.sqrt());
            hi = hi.max(p.x + h.sqrt());
        }
    }
    let d = b - a;
    let len = d.hypot();
    if len > 0.0 {
        let n = kurbo::Vec2::new(-d.y, d.x) * (r / len);
        let band = [a + n, b + n, b - n, a - n];
        for i in 0..4 {
            let (p, q) = (band[i], band[(i + 1) % 4]);
            if (p.y <= y) != (q.y <= y) {
                let x = p.x + (y - p.y) / (q.y - p.y) * (q.x - p.x);
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
    }
    (hi > lo).then_some((lo, hi))
}

/// The x at which some point of the band `top..bottom` (`top <= bottom`) is closer than `r` to the
/// segment `a`–`b`. The points closer than `r` to the segment make a convex shape, so within the
/// band its leftmost and rightmost points lie on the band's top or bottom or beside an end of the
/// segment inside the band, and the result is one interval.
fn near_segment_in_band(a: Point, b: Point, top: f64, bottom: f64, r: f64) -> Option<(f64, f64)> {
    let ends = [a, b].into_iter().filter(|p| p.y >= top && p.y <= bottom).map(|p| (p.x - r, p.x + r));
    let (lo, hi) = [near_segment(a, b, top, r), near_segment(a, b, bottom, r)]
        .into_iter()
        .flatten()
        .chain(ends)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (c, d)| (lo.min(c), hi.max(d)));
    (hi > lo).then_some((lo, hi))
}

/// [`band_intervals`] less every x at which some point of the band `top..bottom` is closer than
/// `inset` to an edge of `polys` (holes included), measured from the edge itself.
fn band_inset_intervals(polys: &[Vec<Point>], top: f64, bottom: f64, inset: f64) -> Vec<(f64, f64)> {
    let iv = band_intervals(polys, top, bottom);
    if inset <= 0.0 || iv.is_empty() {
        return iv;
    }
    if !inset.is_finite() {
        return vec![];
    }
    let (top, bottom) = (top.min(bottom), top.max(bottom));
    let near = polys.iter().flat_map(|p| (0..p.len()).filter_map(move |i| near_segment_in_band(p[i], p[(i + 1) % p.len()], top, bottom, inset)));
    subtract_intervals(&iv, &union_intervals(near.collect()))
}

/// Union of sorted-or-not intervals.
fn union_intervals(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `a` minus `b` (both unions of disjoint intervals, `b` sorted as [`union_intervals`] gives it).
fn subtract_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(mut s, e) in a {
        // Only the intervals of `b` from the first that ends after `s` to the last that starts
        // before `e` cut into this one.
        for &(c, d) in b[b.partition_point(|&(_, d)| d <= s)..].iter().take_while(|&&(c, _)| c < e) {
            if c > s {
                out.push((s, c));
            }
            s = s.max(d);
        }
        if e > s {
            out.push((s, e));
        }
    }
    out
}

/// `a` ∩ `b` (both sorted left to right, each interval starting no earlier than the one before it
/// ends, as [`union_intervals`] and [`band_intervals`] give them), left to right. It walks both
/// lists once, stepping past whichever current interval ends first (both
/// when they end together), so it looks at no more than `a.len() + b.len()` pairs.
fn intersect_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    let (mut i, mut j) = (0, 0);
    while let (Some(&(s, e)), Some(&(c, d))) = (a.get(i), b.get(j)) {
        #[cfg(test)]
        INTERSECT_PAIRS.set(INTERSECT_PAIRS.get() + 1);
        let (x, y) = (s.max(c), e.min(d));
        if y > x {
            out.push((x, y));
        }
        // Ends that are equal, or not numbers, step past both, so the walk always ends.
        match e.partial_cmp(&d) {
            Some(std::cmp::Ordering::Less) => i += 1,
            Some(std::cmp::Ordering::Greater) => j += 1,
            _ => (i, j) = (i + 1, j + 1),
        }
    }
    out
}

#[cfg(test)]
thread_local! {
    /// How many pairs of intervals [`intersect_intervals`] looked at on this thread.
    static INTERSECT_PAIRS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most pairs of edges [`slant_boundaries`] compares for one band.
const MAX_EDGE_PAIRS: usize = 1 << 14;

/// An edge of a frame that isn't horizontal, within a band: its lower and upper ends, its y range
/// within the band and its x range there.
type Slant = (Point, Point, (f64, f64), (f64, f64));

/// The x of the line through `p` and `q` (not horizontal) at `y`.
fn line_x(p: Point, q: Point, y: f64) -> f64 {
    p.x + (y - p.y) / (q.y - p.y) * (q.x - p.x)
}

/// The x ranges within a band at which the edges `slants` bound the frame. Edges that lie along one
/// line and overlap there (contours that meet along an edge, however each one divides it) count by
/// parity: a stretch of the line that an even number of them cover has the frame on both sides.
/// At most [`MAX_EDGE_PAIRS`] pairs are compared; edges left unpaired still count as boundaries,
/// which can make a line shorter at a seam but never puts it outside the frame. The flag is false
/// when the limit left edges unpaired.
fn slant_boundaries(slants: &mut [Slant]) -> (Vec<(f64, f64)>, bool) {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    // Edges along one line overlap in x, so only edges whose x ranges overlap are compared.
    slants.sort_by(|u, v| u.3.0.total_cmp(&v.3.0));
    let mut parent: Vec<usize> = (0..slants.len()).collect();
    let mut pairs = 0;
    'edges: for (i, &(p, q, _, (_, x1))) in slants.iter().enumerate() {
        // Ends on the line up to rounding (a few units in the last place of the coordinates)
        // count as on it.
        let scale = p.x.abs().max(p.y.abs()).max(q.x.abs()).max(q.y.abs()).max(1.0);
        let (d, tol) = (q - p, 4.0 * f64::EPSILON * scale);
        let on_line = |r: Point| d.cross(r - p).abs() <= tol * d.hypot();
        for (j, &(r, s, ..)) in slants.iter().enumerate().skip(i + 1).take_while(|(_, o)| o.3.0 <= x1 + tol) {
            pairs += 1;
            if pairs > MAX_EDGE_PAIRS {
                break 'edges;
            }
            if on_line(r) && on_line(s) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: Vec<(usize, usize)> = (0..slants.len()).map(|i| (root(&mut parent, i), i)).collect();
    groups.sort_unstable();
    let mut out = Vec::new();
    for run in groups.chunk_by(|a, b| a.0 == b.0) {
        if let [(_, i)] = run {
            out.push(slants[*i].3);
            continue;
        }
        // The stretches an odd number of the edges cover: pairs of their sorted y ends.
        let (p, q, ..) = slants[run[0].1];
        let mut ys: Vec<f64> = run.iter().flat_map(|&(_, i)| [slants[i].2.0, slants[i].2.1]).collect();
        ys.sort_by(f64::total_cmp);
        for c in ys.as_chunks::<2>().0.iter().filter(|c| c[1] > c[0]) {
            let (s, t) = (line_x(p, q, c[0]), line_x(p, q, c[1]));
            out.push((s.min(t), s.max(t)));
        }
    }
    (out, pairs <= MAX_EDGE_PAIRS)
}

/// The most pieces a line's band is cut into to keep it inside an Invert Wrap shape grown by its
/// offset.
const INVERT_WRAP_PIECES: f64 = 64.0;

/// The edges of `polys` that reach into the band `top..bottom` (`top <= bottom`), leaving out those
/// that only touch its top or bottom: the horizontal ones as (y, x0, x1) and the others as
/// [`Slant`]s.
fn band_edges(polys: &[Vec<Point>], top: f64, bottom: f64) -> (Vec<(f64, f64, f64)>, Vec<Slant>) {
    let mut flats: Vec<(f64, f64, f64)> = Vec::new();
    let mut slants: Vec<Slant> = Vec::new();
    for poly in polys {
        for (i, &a) in poly.iter().enumerate() {
            let b = poly[(i + 1) % poly.len()];
            let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
            if hi <= top || lo >= bottom {
                continue;
            }
            if a.y == b.y {
                flats.push((a.y, a.x.min(b.x), a.x.max(b.x)));
            } else {
                let (p, q) = if a.y < b.y { (a, b) } else { (b, a) };
                let ys = (lo.max(top), hi.min(bottom));
                let (s, t) = (line_x(p, q, ys.0), line_x(p, q, ys.1));
                slants.push((p, q, ys, (s.min(t), s.max(t))));
            }
        }
    }
    (flats, slants)
}

/// Inside intervals (even-odd) of `polys` over the whole band `top..bottom`: the intervals at the
/// middle of the band minus every x at which an edge lies within the band. A vertical line at any
/// other x meets no edge inside the band, so it is inside over the whole band exactly when it is
/// inside at the middle. An edge that only touches the top or bottom of the band leaves the band's
/// width alone, and so does a stretch of an edge that contours meet along (one that an even number
/// of edges cover).
fn band_intervals(polys: &[Vec<Point>], top: f64, bottom: f64) -> Vec<(f64, f64)> {
    let (top, bottom) = (top.min(bottom), top.max(bottom));
    // Contours that meet along an edge cross the middle of the band at the same x, up to a few
    // units of rounding: the intervals on either side of it join (an edge that bounds the
    // shape there is still subtracted below). A vertical edge crosses the middle at its own x with
    // no rounding, so a gap at one is never taken for a seam.
    let y = (top + bottom) * 0.5;
    let mut upright: Vec<f64> = polys
        .iter()
        .flat_map(|poly| poly.iter().zip(poly.iter().cycle().skip(1)))
        .filter(|(a, b)| a.x == b.x && (a.y <= y) != (b.y <= y))
        .map(|(a, _)| a.x)
        .collect();
    upright.sort_by(f64::total_cmp);
    let upright = |x: f64| upright.binary_search_by(|v| v.total_cmp(&x)).is_ok();
    let mut mid = poly_intervals(polys, y);
    mid.dedup_by(|next, prev| {
        let gap = next.0 - prev.1;
        let seam = gap <= 8.0 * f64::EPSILON * prev.1.abs().max(next.0.abs()).max(1.0) && (gap <= 0.0 || !(upright(prev.1) || upright(next.0)));
        if seam {
            prev.1 = prev.1.max(next.1);
        }
        seam
    });
    subtract_intervals(&mid, &band_boundaries(polys, top, bottom).0)
}

/// The x at which `polys` (even-odd) is inside somewhere in the band `top..bottom`, sorted and
/// disjoint: the intervals at the middle of the band and every x at which an edge lies within the
/// band. A vertical line at any other x meets no edge inside the band, so it is inside somewhere in
/// the band exactly when it is inside at the middle. Edges that contours meet along, which bound no
/// part of the shape, are left out as in [`band_intervals`]. When [`MAX_EDGE_PAIRS`] leaves edges
/// unpaired, `admit` keeps only the intervals at the middle, which the shape holds; otherwise every
/// edge counts, which can only widen the result.
fn band_cover(polys: &[Vec<Point>], top: f64, bottom: f64, admit: bool) -> Vec<(f64, f64)> {
    let (top, bottom) = (top.min(bottom), top.max(bottom));
    let mut iv = poly_intervals(polys, (top + bottom) * 0.5);
    let (edges, paired) = band_boundaries(polys, top, bottom);
    if paired || !admit {
        iv.extend(edges);
    }
    iv.retain(|e| e.1 > e.0);
    union_intervals(iv)
}

/// [`band_cover`] with `admit`, plus the x of every horizontal edge that lies along the top or
/// bottom of the band (`top < bottom`), counted by parity as in [`band_boundaries`]: the x at which
/// the shape or its outline meets the closed band.
fn band_cover_closed(polys: &[Vec<Point>], top: f64, bottom: f64) -> Vec<(f64, f64)> {
    let mut iv = band_cover(polys, top, bottom, true);
    for y in [top, bottom] {
        let mut xs: Vec<f64> = polys
            .iter()
            .flat_map(|poly| poly.iter().zip(poly.iter().cycle().skip(1)))
            .filter(|(a, b)| a.y == y && b.y == y)
            .flat_map(|(a, b)| [a.x, b.x])
            .collect();
        xs.sort_by(f64::total_cmp);
        iv.extend(xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])));
    }
    iv.retain(|e| e.1 > e.0);
    union_intervals(iv)
}

/// Every x at which an edge of `polys` bounds the shape within the band `top..bottom`
/// (`top <= bottom`), sorted and disjoint, leaving out edges that only touch the top or bottom of
/// the band, stretches of edges that an even number of edges cover, and vertical edges. The flag is
/// false when [`MAX_EDGE_PAIRS`] left edges unpaired, which can only add to the result.
fn band_boundaries(polys: &[Vec<Point>], top: f64, bottom: f64) -> (Vec<(f64, f64)>, bool) {
    let (mut flats, mut slants) = band_edges(polys, top, bottom);
    let (mut edges, paired) = slant_boundaries(&mut slants);
    // The x covered by an odd number of the horizontal edges at one height: pairs of their
    // sorted ends.
    flats.sort_by(|p, q| p.0.total_cmp(&q.0));
    for run in flats.chunk_by(|p, q| p.0 == q.0) {
        let mut xs: Vec<f64> = run.iter().flat_map(|&(_, a, b)| [a, b]).collect();
        xs.sort_by(f64::total_cmp);
        edges.extend(xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])));
    }
    // A vertical edge covers a single x: where it bounds the shape, the band's middle already
    // ends at it or an edge of some width meets it, so it is left out (vertical seams join).
    edges.retain(|e| e.1 > e.0);
    (union_intervals(edges), paired)
}

/// The largest magnitude a text wrap offset may have (points), the bound the Text Wrap commands
/// and `.ai` import keep too.
const MAX_WRAP_OFFSET: f64 = 1000.0;

/// A wrap object's offset as layout uses it: within [`MAX_WRAP_OFFSET`] either way, and 0 when it
/// is not a number.
fn wrap_offset(offset: f64) -> f64 {
    if offset.is_nan() { 0.0 } else { offset.clamp(-MAX_WRAP_OFFSET, MAX_WRAP_OFFSET) }
}

/// The x at which some point of the band `top..bottom` (`top <= bottom`) is inside `polys`
/// (even-odd) and farther than `d` (> 0) from every edge of it, holes included: the cover of the
/// shape shrunk by `d`. The band is cut into pieces (at most [`INVERT_WRAP_PIECES`] even ones, and
/// at every anchor's height and `d` above and below it when there are at most that many of those);
/// in each piece, an x is left out when the shape is nowhere in the piece there, or when the whole
/// piece there lies within `d` of one edge. A piece where several edges together cover it keeps
/// its x, so the result can be wider than the shrunk shape, never narrower, and never wider than
/// the shape's own cover of the band.
fn eroded_cover(polys: &[Vec<Point>], top: f64, bottom: f64, d: f64) -> Vec<(f64, f64)> {
    // A point `d` from an edge, up to rounding, counts as near it, so a band that only touches the
    // shrunk shape is left whole.
    let r = d * (1.0 + 1e-12);
    let span = |p: &[Point]| p.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    // The shrunk shape lies more than `d` below the shape's top and above its bottom, so a band
    // outside that holds none of it; within it, only contours that come within `d` of the band
    // count (the others neither cross it nor bound the shape near it).
    let (lo, hi) = polys.iter().map(|p| span(p)).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (c, e)| (a.min(c), b.max(e)));
    if bottom <= lo + r || top >= hi - r {
        return vec![];
    }
    let near_band: Vec<Vec<Point>> = polys
        .iter()
        .filter(|p| {
            let (lo, hi) = span(p);
            hi >= top - r && lo <= bottom + r
        })
        .cloned()
        .collect();
    let polys = near_band.as_slice();
    let pieces = ((bottom - top) * 4.0 / d).ceil().clamp(1.0, INVERT_WRAP_PIECES);
    let mut cuts: Vec<f64> = (0..=pieces as usize).map(|k| top + (bottom - top) * k as f64 / pieces).collect();
    if bottom > top {
        let mut anchors: Vec<f64> = polys.iter().flatten().flat_map(|p| [p.y - d, p.y, p.y + d]).filter(|&y| y > top && y < bottom).collect();
        anchors.sort_by(f64::total_cmp);
        anchors.dedup();
        if anchors.len() <= INVERT_WRAP_PIECES as usize {
            cuts.extend(anchors);
            cuts.sort_by(f64::total_cmp);
            cuts.dedup();
        }
    }
    let mut out = Vec::new();
    // A band of no height keeps its two equal cuts: one piece of no height, measured at its y.
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let cover = band_cover(polys, a, b, false);
        if cover.is_empty() {
            continue;
        }
        // The points closer than `d` to an edge make a convex shape, so a vertical line within
        // the piece lies in it when both its ends do.
        let near = polys.iter().flat_map(|p| {
            (0..p.len()).filter_map(move |i| {
                let (s, t) = (p[i], p[(i + 1) % p.len()]);
                let (u, v) = (near_segment(s, t, a, r)?, near_segment(s, t, b, r)?);
                let (x, y) = (u.0.max(v.0), u.1.min(v.1));
                (y > x).then_some((x, y))
            })
        });
        out.extend(subtract_intervals(&cover, &union_intervals(near.collect())));
    }
    union_intervals(out)
}

impl Region {
    pub(super) fn cells(path: &BezPath, opts: &LayoutOptions, wrap: &[vectorcraft_doc::WrapShape]) -> Vec<Region> {
        // A frame out of range has no cells, and a wrap shape out of range is left out.
        let Some(polys) = polygons(path) else { return vec![] };
        // Every cell shares one copy of the frame's polygons and one copy of the text wrap shapes.
        let polys: std::sync::Arc<[Vec<Point>]> = polys.into();
        let wraps: std::sync::Arc<[FlatWrap]> = wrap
            .iter()
            .filter_map(|w| Some((polygons(&w.path.to_bezpath())?, wrap_offset(w.wrap.offset), w.wrap.invert)))
            .filter(|w| !w.0.is_empty())
            .collect();
        let bbox = path.bounding_box();
        let rect = polys.len() == 1 && {
            let area: f64 = polys[0].iter().zip(polys[0].iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>().abs() * 0.5;
            (area - bbox.area()).abs() <= bbox.area() * 1e-6 + 1e-9
        };
        let max = vectorcraft_doc::AreaOptions::MAX_ROWS_COLUMNS;
        let (rows, cols) = (opts.rows.clamp(1, max), opts.columns.clamp(1, max));
        let gutter = opts.gutter.max(0.0);
        let row_gutter = opts.row_gutter.unwrap_or(opts.gutter).max(0.0);
        let cw = ((bbox.width() - gutter * (cols - 1) as f64) / cols as f64).max(0.0);
        let rh = ((bbox.height() - row_gutter * (rows - 1) as f64) / rows as f64).max(0.0);
        let mut out = Vec::with_capacity(rows * cols);
        // Text Flow: down each column, then across (by columns, the default), or across each row,
        // then down (by rows).
        let by_rows = opts.flow == vectorcraft_doc::TextFlow::Rows;
        let (outer, inner) = if by_rows { (rows, cols) } else { (cols, rows) };
        for o in 0..outer {
            for i in 0..inner {
                let (r, c) = if by_rows { (o, i) } else { (i, o) };
                let x0 = bbox.x0 + c as f64 * (cw + gutter);
                let y0 = bbox.y0 + r as f64 * (rh + row_gutter);
                let cell = if rows * cols == 1 { bbox } else { Rect::new(x0, y0, x0 + cw, y0 + rh) };
                out.push(Region { cell, inset: opts.inset.max(0.0), polys: polys.clone(), rect, wraps: wraps.clone(), shift: 0.0, gap: 0.0 });
            }
        }
        out
    }

    pub(super) fn top(&self) -> f64 {
        self.cell.y0 + self.inset
    }
    pub(super) fn bottom(&self) -> f64 {
        self.cell.y1 - self.inset
    }

    /// `iv` minus every x at which a wrap object lies somewhere in the band `top..bottom` (or the x
    /// inside it over the whole band, for Invert Wrap). A positive offset grows each wrap shape
    /// vertically and horizontally; a negative one shrinks it by that distance from its edges
    /// (holes included), so text runs that far into the object, or stays that far inside it for
    /// Invert Wrap. Along the line, the text also keeps the inset beside each wrap object
    /// (outside it, or inside it for Invert Wrap); above and below a wrap object only its offset
    /// applies.
    fn wrapped(&self, mut iv: Vec<(f64, f64)>, top: f64, bottom: f64) -> Vec<(f64, f64)> {
        let (top, bottom) = (top.min(bottom), top.max(bottom));
        let inset = self.inset;
        for (polys, off, invert) in self.wraps.iter() {
            if iv.is_empty() {
                break;
            }
            let grow = |v: Vec<(f64, f64)>| union_intervals(v.into_iter().map(|(a, b)| (a - off, b + off)).collect());
            let shrink = |v: Vec<(f64, f64)>| v.into_iter().map(|(a, b)| (a + inset, b - inset)).filter(|(a, b)| b > a).collect::<Vec<_>>();
            let pad = |v: Vec<(f64, f64)>| union_intervals(v.into_iter().map(|(a, b)| (a - inset, b + inset)).collect());
            iv = if *off < 0.0 {
                // Invert Wrap keeps the x at which the shape shrunk by the offset holds the whole
                // band; otherwise the text leaves out the x at which the shrunk shape lies
                // somewhere in the band.
                if *invert {
                    intersect_intervals(&iv, &shrink(band_inset_intervals(polys, top, bottom, -off)))
                } else {
                    subtract_intervals(&iv, &pad(eroded_cover(polys, top, bottom, -off)))
                }
            } else if *invert {
                // The band is cut into pieces. Each height of a piece `y0..y1` is within the offset
                // of some height from `y0` plus the offset to `y1` less the offset, so an x the
                // shape holds at all of those heights is inside the grown shape over the piece. In
                // a piece less than twice the offset tall, each height of the piece is within the
                // offset of every height from `y1` less the offset to `y0` plus the offset, so an x
                // the shape holds at any of them is. Text within the offset of such an x, for every
                // piece, stays inside the grown shape; finer pieces give it more of that room.
                //
                // The band is also cut at every height an offset above or below an anchor of the
                // shape (when there are at most [`INVERT_WRAP_PIECES`] of them). Within a piece
                // between such cuts, no anchor enters or leaves the offset window of any of its
                // heights, so where two parts of the shape bridge a gap together (one within the
                // offset above and one below), the heights from `y1` less the offset to `y0` plus
                // the offset reach from one to the other, and the horizontal edges they end at
                // count.
                let pieces = if *off > 0.0 { ((bottom - top) * 4.0 / off).ceil().clamp(1.0, INVERT_WRAP_PIECES) } else { 1.0 };
                let mut cuts: Vec<f64> = (0..=pieces as usize).map(|k| top + (bottom - top) * k as f64 / pieces).collect();
                // A band of no height stays one piece: no anchor lies strictly inside it, and
                // removing the repeated cut would leave no piece.
                if *off > 0.0 && bottom > top {
                    let mut anchors: Vec<f64> =
                        polys.iter().flatten().flat_map(|p| [p.y - off, p.y + off]).filter(|&y| y > top && y < bottom).collect();
                    anchors.sort_by(f64::total_cmp);
                    anchors.dedup();
                    if anchors.len() <= INVERT_WRAP_PIECES as usize {
                        cuts.extend(anchors);
                        cuts.sort_by(f64::total_cmp);
                        cuts.dedup();
                    }
                }
                for w in cuts.windows(2) {
                    let (a, b) = (w[0] + off, w[1] - off);
                    let held = if a <= b { band_intervals(polys, a, b) } else { band_cover_closed(polys, b, a) };
                    iv = intersect_intervals(&iv, &shrink(grow(held)));
                    if iv.is_empty() {
                        break;
                    }
                }
                iv
            } else {
                subtract_intervals(&iv, &pad(grow(band_cover(polys, top - off, bottom + off, false))))
            };
        }
        iv
    }

    /// Widest horizontal span inside the frame (and the cell) over the band `top..bottom`.
    pub(super) fn span(&self, top: f64, bottom: f64) -> Option<(f64, f64)> {
        self.spans(top, bottom).into_iter().max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))
    }

    /// Every horizontal span inside the frame (and the cell) over the band `top..bottom`, left to
    /// right (text wraps on both sides of an object).
    pub(super) fn spans(&self, top: f64, bottom: f64) -> Vec<(f64, f64)> {
        // The inset from the cell's edges; `band_inset_intervals` already keeps it from the frame's
        // edges and `wrapped` beside the wrap objects.
        let clip = |(a, b): (f64, f64)| {
            let (a, b) = (a.max(self.cell.x0 + self.inset), b.min(self.cell.x1 - self.inset));
            (b > a).then_some((a, b))
        };
        let plain = self.polys.is_empty() || self.rect;
        if plain && self.wraps.is_empty() {
            return (self.cell.width() > 0.0).then_some((self.cell.x0, self.cell.x1)).and_then(clip).into_iter().collect();
        }
        let fb = if top.is_nan() || bottom.is_nan() {
            return vec![];
        } else if plain {
            self.cell
        } else {
            self.polys.iter().flatten().fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p))
        };
        // `max` then `min` rather than `clamp`, which panics on a frame less than 2e-4 tall.
        let clamp = |y: f64| if plain { y } else { y.max(fb.y0 + 1e-4).min(fb.y1 - 1e-4) };
        let free = if plain { vec![(self.cell.x0, self.cell.x1)] } else { band_inset_intervals(&self.polys, clamp(top), clamp(bottom), self.inset) };
        // Wrap objects split lines: keep what is free of them over the whole band.
        let free = if self.wraps.is_empty() { free } else { self.wrapped(free, clamp(top), clamp(bottom)) };
        free.into_iter().filter_map(clip).collect()
    }
}

/// The 10,000 cells of a frame in 100 rows and 100 columns share one copy of the frame's polygons
/// and one copy of the text wrap shapes.
#[cfg(test)]
#[test]
fn the_cells_of_a_frame_share_its_polygons_and_the_wrap_shapes() {
    let frame = kurbo::Circle::new((0.0, 0.0), 1000.0).to_path(0.1);
    let square = PathData::from_bezpath(&Rect::new(-10.0, -10.0, 10.0, 10.0).to_path(0.1));
    let wrap = vectorcraft_doc::WrapShape { path: square, wrap: Default::default() };
    let cells = Region::cells(&frame, &LayoutOptions { rows: 100, columns: 100, ..LayoutOptions::default() }, &[wrap]);
    assert_eq!(cells.len(), 10_000);
    let first = &cells[0];
    assert!(!first.polys.is_empty() && !first.wraps.is_empty());
    assert!(cells.iter().all(|c| c.polys.as_ptr() == first.polys.as_ptr() && c.wraps.as_ptr() == first.wraps.as_ptr()));
}

/// A path with one closed subpath through each list of points.
#[cfg(test)]
fn closed(subpaths: &[&[(f64, f64)]]) -> BezPath {
    let mut path = BezPath::new();
    for pts in subpaths {
        path.move_to(pts[0]);
        for &p in &pts[1..] {
            path.line_to(p);
        }
        path.close_path();
    }
    path
}

/// The spans of every cell of `path` over the band `top..bottom`.
#[cfg(test)]
fn cell_spans(path: &BezPath, opts: &LayoutOptions, wrap: &[vectorcraft_doc::WrapShape], top: f64, bottom: f64) -> Vec<Vec<(f64, f64)>> {
    Region::cells(path, opts, wrap).iter().map(|c| c.spans(top, bottom)).collect()
}

#[cfg(test)]
fn assert_spans(got: &[(f64, f64)], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?} != {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9, "{got:?} != {want:?}");
    }
}

/// A line stays inside a frame whose right edge pinches in to a point between any fixed heights of
/// the line's band, with or without text wrap shapes elsewhere.
#[cfg(test)]
#[test]
fn a_line_stays_inside_a_frame_that_pinches_in_within_its_band() {
    let notch =
        |tip: (f64, f64), (y0, y1): (f64, f64)| closed(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, y0), tip, (100.0, y1), (100.0, 100.0), (0.0, 100.0)]]);
    let far = PathData::from_bezpath(&Rect::new(300.0, 0.0, 310.0, 100.0).to_path(0.1));
    let far = vectorcraft_doc::WrapShape { path: far, wrap: Default::default() };
    for (frame, x) in
        [(notch((60.0, 47.5), (45.0, 50.0)), 60.0), (notch((70.0, 42.0), (41.0, 43.0)), 70.0), (notch((80.0, 59.0), (58.0, 60.0)), 80.0)]
    {
        for wrap in [&[][..], std::slice::from_ref(&far)] {
            let cells = Region::cells(&frame, &LayoutOptions::default(), wrap);
            assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, x)]);
            assert_eq!(cells[0].span(40.0, 60.0), Some((0.0, x)));
        }
    }
}

/// A line stays clear of a wrap object whose tip reaches into its band between any fixed heights of
/// the band, with or without an offset, and inside an Invert Wrap object whose edge pinches in
/// there. Invert Wrap with an offset keeps the lines it kept beside a straight edge.
#[cfg(test)]
#[test]
fn a_line_stays_clear_of_a_wrap_object_that_reaches_into_its_band() {
    let frame = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
    let spans = |shape: &[(f64, f64)], offset: f64, invert: bool| {
        let path = PathData::from_bezpath(&closed(&[shape]));
        let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset, invert } };
        let cells = Region::cells(&frame, &LayoutOptions::default(), &[wrap]);
        assert!(cells[0].rect);
        cells[0].spans(40.0, 60.0)
    };
    assert_spans(&spans(&[(100.0, 45.0), (40.0, 47.5), (100.0, 50.0)], 0.0, false), &[(0.0, 40.0)]);
    assert_spans(&spans(&[(100.0, 47.0), (40.0, 47.5), (100.0, 48.0)], 1.0, false), &[(0.0, 39.0)]);
    // Two contours that cancel each other (even-odd) hold no area and leave the line whole.
    let band: &[(f64, f64)] = &[(0.0, 45.0), (100.0, 45.0), (100.0, 55.0), (0.0, 55.0)];
    let path = PathData::from_bezpath(&closed(&[band, band]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 0.0, invert: false } };
    assert_spans(&Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    let notch = [(0.0, 0.0), (100.0, 0.0), (100.0, 45.0), (60.0, 47.5), (100.0, 50.0), (100.0, 100.0), (0.0, 100.0)];
    assert_spans(&spans(&notch, 0.0, true), &[(0.0, 60.0)]);
    let triangle = [(0.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    assert_spans(&spans(&triangle, 5.0, true), &[(0.0, 50.0)]);
    assert_spans(&spans(&triangle, 15.0, true), &[(0.0, 70.0)]);
    // An Invert Wrap shape that steps sideways within the band keeps the room its offset bridges.
    let upper: &[(f64, f64)] = &[(0.0, 0.0), (40.0, 0.0), (40.0, 50.0), (0.0, 50.0)];
    let lower: &[(f64, f64)] = &[(45.0, 50.0), (85.0, 50.0), (85.0, 100.0), (45.0, 100.0)];
    let path = PathData::from_bezpath(&closed(&[upper, lower]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert_spans(&Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0), &[(40.0, 45.0)]);
    // An Invert Wrap shape of two parts whose offsets meet across the gap between them (45.25 + 5
    // reaches past 55.125 - 5) keeps the whole line.
    let upper: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 0.0), (100.0, 45.25), (0.0, 45.25)];
    let lower: &[(f64, f64)] = &[(0.0, 55.125), (100.0, 55.125), (100.0, 100.0), (0.0, 100.0)];
    let path = PathData::from_bezpath(&closed(&[upper, lower]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert_spans(&Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // A gap the offsets do not bridge (45 + 5 stops short of 55.125 - 5) holds no line.
    let upper: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 0.0), (100.0, 45.0), (0.0, 45.0)];
    let path = PathData::from_bezpath(&closed(&[upper, lower]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert!(Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0).is_empty());
    // A band of no height keeps Invert Wrap and its offset.
    let strip: &[(f64, f64)] = &[(20.0, 0.0), (40.0, 0.0), (40.0, 100.0), (20.0, 100.0)];
    let path = PathData::from_bezpath(&closed(&[strip]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert_spans(&Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(50.0, 50.0), &[(15.0, 45.0)]);
}

/// The spans of a 100 pt square frame over the band `top..bottom` with one wrap object through
/// `contours` (even-odd).
#[cfg(test)]
fn wrap_spans(contours: &[&[(f64, f64)]], offset: f64, invert: bool, top: f64, bottom: f64) -> Vec<(f64, f64)> {
    let frame = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
    let wrap = vectorcraft_doc::WrapShape { path: PathData::from_bezpath(&closed(contours)), wrap: vectorcraft_doc::TextWrap { offset, invert } };
    Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(top, bottom)
}

/// A negative offset lets text run into a wrap object by that distance, across and along the
/// line: beside a square, the line reaches 5 pt into it, and a line whose band reaches no more than
/// 5 pt into its top or bottom runs across it.
#[cfg(test)]
#[test]
fn negative_wrap_offset_insets_rectangle_on_both_axes() {
    let square: &[(f64, f64)] = &[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)];
    assert_spans(&wrap_spans(&[square], -5.0, false, 44.0, 50.0), &[(0.0, 45.0), (55.0, 100.0)]);
    assert_spans(&wrap_spans(&[square], -5.0, false, 30.0, 45.0), &[(0.0, 100.0)]);
    assert_spans(&wrap_spans(&[square], -5.0, false, 55.0, 70.0), &[(0.0, 100.0)]);
    assert_spans(&wrap_spans(&[square], -5.0, false, 30.0, 46.0), &[(0.0, 45.0), (55.0, 100.0)]);
    // The most an offset may take away is 1000 pt, and an offset that is not a number is 0.
    assert_spans(&wrap_spans(&[square], f64::NEG_INFINITY, false, 44.0, 50.0), &[(0.0, 100.0)]);
    assert_spans(&wrap_spans(&[square], f64::NAN, false, 44.0, 50.0), &[(0.0, 40.0), (60.0, 100.0)]);
}

/// A negative offset is measured from a sloped edge itself (10 pt across the edge of a triangle is
/// 10√2 pt along the line), and from the edges of a hole too.
#[cfg(test)]
#[test]
fn negative_wrap_offset_insets_sloped_shape_and_hole() {
    let triangle: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    assert_spans(&wrap_spans(&[triangle], -10.0, false, 40.0, 60.0), &[(0.0, 10.0), (60.0 - 10.0 * std::f64::consts::SQRT_2, 100.0)]);
    let outer: &[(f64, f64)] = &[(20.0, 20.0), (80.0, 20.0), (80.0, 80.0), (20.0, 80.0)];
    let hole: &[(f64, f64)] = &[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)];
    assert_spans(&wrap_spans(&[outer, hole], -5.0, false, 48.0, 52.0), &[(0.0, 25.0), (35.0, 65.0), (75.0, 100.0)]);
}

/// With Invert Wrap, a negative offset keeps the text that far inside the object, along and across
/// the line.
#[cfg(test)]
#[test]
fn negative_invert_wrap_keeps_only_eroded_interior() {
    let square: &[(f64, f64)] = &[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)];
    assert_spans(&wrap_spans(&[square], -5.0, true, 46.0, 54.0), &[(45.0, 55.0)]);
    assert!(wrap_spans(&[square], -5.0, true, 42.0, 48.0).is_empty());
    let triangle: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    assert_spans(&wrap_spans(&[triangle], -10.0, true, 40.0, 60.0), &[(10.0, 40.0 - 10.0 * std::f64::consts::SQRT_2)]);
}

/// A negative offset at least half as large as a wrap object is wide leaves nothing of it: text
/// runs across it, and with Invert Wrap no text fits inside it.
#[cfg(test)]
#[test]
fn negative_offset_erases_narrow_wrap_shape() {
    let strip: &[(f64, f64)] = &[(40.0, 0.0), (48.0, 0.0), (48.0, 100.0), (40.0, 100.0)];
    for offset in [-4.0, -5.0, -1000.0] {
        assert_spans(&wrap_spans(&[strip], offset, false, 40.0, 60.0), &[(0.0, 100.0)]);
        assert!(wrap_spans(&[strip], offset, true, 40.0, 60.0).is_empty(), "offset {offset}");
    }
    // A band of no height.
    assert_spans(&wrap_spans(&[strip], -1.0, false, 50.0, 50.0), &[(0.0, 41.0), (47.0, 100.0)]);
}

/// A negative offset never keeps text farther from a wrap object than an offset of 0 does, on a
/// shape whose edges meet at many angles.
#[cfg(test)]
#[test]
fn negative_offset_never_wraps_more_than_no_offset() {
    let star: Vec<(f64, f64)> = (0..40)
        .map(|k| {
            let (r, a) = (if k % 2 == 0 { 40.0 } else { 15.0 }, f64::from(k) * std::f64::consts::PI / 20.0);
            (50.0 + r * a.cos(), 50.0 + r * a.sin())
        })
        .collect();
    for (top, bottom) in [(10.0, 22.0), (44.0, 56.0), (30.0, 31.0), (70.0, 90.0)] {
        for invert in [false, true] {
            let zero = wrap_spans(&[star.as_slice()], 0.0, invert, top, bottom);
            let neg = wrap_spans(&[star.as_slice()], -3.0, invert, top, bottom);
            let width = |v: &[(f64, f64)]| v.iter().map(|(a, b)| b - a).sum::<f64>();
            if invert {
                assert!(width(&neg) <= width(&zero) + 1e-9, "{top}..{bottom}: {neg:?} {zero:?}");
            } else {
                assert!(width(&neg) >= width(&zero) - 1e-9, "{top}..{bottom}: {neg:?} {zero:?}");
            }
        }
    }
}

/// A hole that starts and ends inside a line's band splits the line, and a band below the hole, or
/// one that only touches it, keeps the whole width.
#[cfg(test)]
#[test]
fn a_hole_inside_a_band_splits_the_line() {
    let frame = closed(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], &[(40.0, 52.0), (60.0, 52.0), (60.0, 58.0), (40.0, 58.0)]]);
    let cells = Region::cells(&frame, &LayoutOptions::default(), &[]);
    assert!(!cells[0].rect);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 40.0), (60.0, 100.0)]);
    assert_spans(&cells[0].spans(60.0, 80.0), &[(0.0, 100.0)]);
    // A band that only touches the hole at its top or bottom keeps the whole width.
    assert_spans(&cells[0].spans(58.0, 78.0), &[(0.0, 100.0)]);
    assert_spans(&cells[0].spans(32.0, 52.0), &[(0.0, 100.0)]);
    // A band given bottom first covers the same heights.
    assert_spans(&cells[0].spans(60.0, 40.0), &[(0.0, 40.0), (60.0, 100.0)]);
    assert!(cells[0].spans(f64::NAN, 60.0).is_empty());
}

/// Lines run across an edge two contours share (a horizontal one over the part both cover) and
/// stop at the part only one covers (a step).
#[cfg(test)]
#[test]
fn lines_cross_an_edge_two_contours_share() {
    let seam = closed(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], &[(0.0, 50.0), (100.0, 50.0), (100.0, 100.0), (0.0, 100.0)]]);
    let cells = Region::cells(&seam, &LayoutOptions::default(), &[]);
    assert!(!cells[0].rect);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    let step = closed(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], &[(0.0, 50.0), (50.0, 50.0), (50.0, 100.0), (0.0, 100.0)]]);
    let cells = Region::cells(&step, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 50.0)]);
    // Contours that meet along a seam that bends within the band, one drawn each way along it.
    let bent = closed(&[
        &[(0.0, 0.0), (50.0, 0.0), (70.0, 47.5), (50.0, 100.0), (0.0, 100.0)],
        &[(50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 100.0), (70.0, 47.5)],
    ]);
    let cells = Region::cells(&bent, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // Contours side by side, meeting along a vertical edge.
    let side = closed(&[&[(0.0, 0.0), (50.0, 0.0), (50.0, 100.0), (0.0, 100.0)], &[(50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 100.0)]]);
    let cells = Region::cells(&side, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // The same, with an extra anchor on one side of the shared edge.
    let split =
        closed(&[&[(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (50.0, 100.0), (0.0, 100.0)], &[(50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 100.0)]]);
    let cells = Region::cells(&split, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // A slanted shared edge with an extra anchor on one side.
    let slanted =
        closed(&[&[(0.0, 0.0), (40.0, 0.0), (50.0, 50.0), (60.0, 100.0), (0.0, 100.0)], &[(40.0, 0.0), (100.0, 0.0), (100.0, 100.0), (60.0, 100.0)]]);
    let cells = Region::cells(&slanted, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // Slanted edges of two contours 1e-7 pt apart are not one edge: the line stops at each.
    let near =
        closed(&[&[(0.0, 0.0), (40.0, 0.0), (60.0, 100.0), (0.0, 100.0)], &[(40.0 + 1e-7, 0.0), (100.0, 0.0), (100.0, 100.0), (60.0 + 1e-7, 100.0)]]);
    let cells = Region::cells(&near, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 48.0), (52.0 + 1e-7, 100.0)]);
    // The same 1e-6 pt apart, 1e8 pt from the origin.
    let (x, y) = (1e8, 1e8);
    let far = closed(&[
        &[(x, y), (x + 40.0, y), (x + 60.0, y + 100.0), (x, y + 100.0)],
        &[(x + 40.0 + 1e-6, y), (x + 100.0, y), (x + 100.0, y + 100.0), (x + 60.0 + 1e-6, y + 100.0)],
    ]);
    let cells = Region::cells(&far, &LayoutOptions::default(), &[]);
    let spans = cells[0].spans(y + 40.0, y + 60.0);
    assert!(spans.len() == 2 && spans[0].1 < x + 48.001 && spans[1].0 > x + 51.999, "{spans:?}");
    // A real gap of 0.01 pt between contours far from the origin stays a gap.
    let x = 1e8;
    let apart = closed(&[
        &[(x, 0.0), (x + 50.0, 0.0), (x + 50.0, 100.0), (x, 100.0)],
        &[(x + 50.01, 0.0), (x + 100.0, 0.0), (x + 100.0, 100.0), (x + 50.01, 100.0)],
    ]);
    let cells = Region::cells(&apart, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(x, x + 50.0), (x + 50.01, x + 100.0)]);
    // So does a gap of 1e-7 pt between vertical edges there, a few units of rounding wide.
    let apart = closed(&[
        &[(x, 0.0), (x + 50.0, 0.0), (x + 50.0, 100.0), (x, 100.0)],
        &[(x + 50.0 + 1e-7, 0.0), (x + 100.0, 0.0), (x + 100.0, 100.0), (x + 50.0 + 1e-7, 100.0)],
    ]);
    let cells = Region::cells(&apart, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(x, x + 50.0), (x + 50.0 + 1e-7, x + 100.0)]);
}

/// A frame of 200 thin slanted strips, whose edges in the band overlap in x far more than
/// [`MAX_EDGE_PAIRS`] times, holds no line in the band (no strip holds a vertical line across it).
#[cfg(test)]
#[test]
fn many_overlapping_edges_in_a_band_hold_no_line_outside_the_frame() {
    let strips: Vec<Vec<(f64, f64)>> =
        (0..200).map(|i| f64::from(i) * 2.0).map(|x| vec![(x, 0.0), (x + 1.0, 0.0), (x + 501.0, 100.0), (x + 500.0, 100.0)]).collect();
    let strips: Vec<&[(f64, f64)]> = strips.iter().map(Vec::as_slice).collect();
    let cells = Region::cells(&closed(&strips), &LayoutOptions::default(), &[]);
    assert!(cells[0].spans(40.0, 60.0).is_empty());
    assert!(!cells[0].spans(50.0, 50.0).is_empty());
}

/// A frame shorter than the band's margin at the frame's top and bottom holds no line and does
/// not panic; a rectangle keeps its inset spans, top and bottom.
#[cfg(test)]
#[test]
fn flat_frames_and_rectangles_keep_their_spans() {
    for height in [0.0, 1e-5] {
        let flat = closed(&[&[(0.0, 0.0), (10.0, 0.0), (0.0, height)], &[(20.0, 0.0), (30.0, 0.0), (20.0, height)]]);
        let cells = Region::cells(&flat, &LayoutOptions::default(), &[]);
        assert!(!cells[0].rect);
        assert!(cells[0].spans(-5.0, 5.0).iter().all(|&(a, b)| b - a < 1e-3));
    }
    let square = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
    let cells = Region::cells(&square, &LayoutOptions { inset: 10.0, ..LayoutOptions::default() }, &[]);
    assert!(cells[0].rect);
    assert_eq!((cells[0].top(), cells[0].bottom()), (10.0, 90.0));
    assert_spans(&cells[0].spans(40.0, 60.0), &[(10.0, 90.0)]);
}

/// On a triangle, the inset keeps the line 10 pt from the sloped edge, measured from the edge
/// itself (x + y <= 100 - 10√2 at the band's bottom), not 10 pt along the line.
#[cfg(test)]
#[test]
fn inset_on_a_shaped_frame_is_measured_from_its_sloped_edge() {
    let triangle = closed(&[&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]]);
    let inset = |inset| cell_spans(&triangle, &LayoutOptions { inset, ..LayoutOptions::default() }, &[], 40.0, 60.0);
    assert_spans(&inset(10.0)[0], &[(10.0, 40.0 - 10.0 * std::f64::consts::SQRT_2)]);
    assert_spans(&inset(0.0)[0], &[(0.0, 40.0)]);
}

/// A rectangular frame keeps its inset from each side, and its first line starts the inset below
/// its top.
#[cfg(test)]
#[test]
fn a_rectangular_frame_keeps_its_inset_from_each_side() {
    let cells = Region::cells(&Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1), &LayoutOptions { inset: 10.0, ..LayoutOptions::default() }, &[]);
    assert!(cells[0].rect);
    assert_eq!((cells[0].top(), cells[0].bottom()), (10.0, 90.0));
    assert_spans(&cells[0].spans(40.0, 60.0), &[(10.0, 90.0)]);
}

/// The inset keeps text clear of a hole's sloped edges too.
#[cfg(test)]
#[test]
fn inset_keeps_text_clear_of_a_hole_s_sloped_edges() {
    let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    let diamond = [(50.0, 30.0), (70.0, 50.0), (50.0, 70.0), (30.0, 50.0)];
    let spans = cell_spans(&closed(&[&square, &diamond]), &LayoutOptions { inset: 5.0, ..LayoutOptions::default() }, &[], 40.0, 44.0);
    let clear = 5.0 * std::f64::consts::SQRT_2;
    assert_spans(&spans[0], &[(5.0, 36.0 - clear), (64.0 + clear, 95.0)]);
}

/// In a frame of columns, the inset is kept once from a column's inner edge and once from the
/// frame's sloped edge.
#[cfg(test)]
#[test]
fn inset_applies_once_at_a_column_s_edge_and_at_the_frame_s_edge() {
    let triangle = closed(&[&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]]);
    let spans = cell_spans(&triangle, &LayoutOptions { inset: 10.0, columns: 2, gutter: 0.0, ..LayoutOptions::default() }, &[], 10.0, 20.0);
    assert_spans(&spans[0], &[(10.0, 40.0)]);
    assert_spans(&spans[1], &[(60.0, 80.0 - 10.0 * std::f64::consts::SQRT_2)]);
}

/// An inset wider than a shaped frame leaves no room for text, whatever its size.
#[cfg(test)]
#[test]
fn an_inset_wider_than_a_shaped_frame_leaves_no_room() {
    let triangle = closed(&[&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]]);
    for inset in [40.0, 1e300, f64::INFINITY] {
        let cells = Region::cells(&triangle, &LayoutOptions { inset, ..LayoutOptions::default() }, &[]);
        assert!(cells[0].spans(40.0, 60.0).is_empty() && cells[0].span(40.0, 60.0).is_none(), "inset {inset}");
    }
}

/// `a` ∩ `b` by comparing every interval of `a` with every interval of `b`: the result
/// [`intersect_intervals`] is checked against.
#[cfg(test)]
fn intersect_every_pair(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    a.iter().flat_map(|&(s, e)| b.iter().map(move |&(c, d)| (s.max(c), e.min(d)))).filter(|(x, y)| y > x).collect()
}

/// The intersection of two lists of intervals, each sorted with no two overlapping, is the one
/// every pair of their intervals gives, in the same order, left to right and of positive width, for
/// every pair of such lists whose ends are among the points 0 to 6: empty lists, intervals that
/// only touch (in one list or across the two), that end together, that hold one another and that
/// overlap several others.
#[cfg(test)]
#[test]
fn intervals_intersect_as_every_pair_of_them_does() {
    // Every list is a shorter list with one more interval after its last.
    let mut lists: Vec<Vec<(f64, f64)>> = vec![vec![]];
    let mut k = 0;
    while let Some(list) = lists.get(k).cloned() {
        let from = list.last().map_or(0, |l: &(f64, f64)| l.1 as u8);
        for s in from..6 {
            for e in s + 1..=6 {
                let mut next = list.clone();
                next.push((f64::from(s), f64::from(e)));
                lists.push(next);
            }
        }
        k += 1;
    }
    assert_eq!(lists.len(), 233);
    for a in &lists {
        for b in &lists {
            let got = intersect_intervals(a, b);
            assert_eq!(got, intersect_every_pair(a, b), "{a:?} ∩ {b:?}");
            assert!(got.iter().all(|(x, y)| y > x) && got.windows(2).all(|w| w[0].1 <= w[1].0), "{got:?}");
        }
    }
}

/// Intersecting two unions of 3,000 and 2,000 intervals that interleave looks at no more than
/// 5,000 pairs of intervals, not every one of the 6,000,000 pairs.
#[cfg(test)]
#[test]
fn intersecting_intervals_looks_at_each_interval_once() {
    let a: Vec<(f64, f64)> = (0..3000).map(|i| f64::from(i) * 4.0).map(|x| (x, x + 3.0)).collect();
    let b: Vec<(f64, f64)> = (0..2000).map(|i| f64::from(i) * 6.0).map(|x| (x + 1.0, x + 5.0)).collect();
    INTERSECT_PAIRS.set(0);
    let got = intersect_intervals(&a, &b);
    assert!(INTERSECT_PAIRS.get() <= a.len() + b.len(), "{} pairs", INTERSECT_PAIRS.get());
    assert_eq!(got, intersect_every_pair(&a, &b));
    assert!(!got.is_empty());
}

/// A line in area type with an Invert Wrap comb of 500 teeth and an offset keeps a span inside
/// each tooth, left to right, through every piece of its band, and intersecting the spans with each
/// piece looks at each span once.
#[cfg(test)]
#[test]
fn a_line_inside_an_invert_wrap_comb_keeps_a_span_in_each_tooth() {
    let frame = Rect::new(0.0, 0.0, 1000.0, 100.0).to_path(0.1);
    let teeth: Vec<Vec<(f64, f64)>> =
        (0..500).map(|k| f64::from(k) * 2.0).map(|x| vec![(x, 0.0), (x + 1.0, 0.0), (x + 1.0, 100.0), (x, 100.0)]).collect();
    let teeth: Vec<&[(f64, f64)]> = teeth.iter().map(Vec::as_slice).collect();
    let path = PathData::from_bezpath(&closed(&teeth));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 0.25, invert: true } };
    let cells = Region::cells(&frame, &LayoutOptions::default(), &[wrap]);
    INTERSECT_PAIRS.set(0);
    let got = cells[0].spans(40.0, 60.0);
    let want: Vec<(f64, f64)> = (0..500).map(|k| f64::from(k) * 2.0).map(|x| ((x - 0.25).max(0.0), x + 1.25)).collect();
    assert_spans(&got, &want);
    // At most 64 pieces, each looking at no more than the spans so far and the tooth spans.
    assert!(INTERSECT_PAIRS.get() <= 64 * (500 + 500), "{} pairs", INTERSECT_PAIRS.get());
}

/// Along the line, text keeps the inset beside a wrap object, around it and, with Invert Wrap,
/// inside it.
#[cfg(test)]
#[test]
fn text_keeps_the_inset_beside_a_wrap_object_along_the_line() {
    let frame = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
    let shape = PathData::from_bezpath(&Rect::new(40.0, 40.0, 60.0, 60.0).to_path(0.1));
    let opts = LayoutOptions { inset: 5.0, ..LayoutOptions::default() };
    for (invert, want) in [(false, vec![(5.0, 35.0), (65.0, 95.0)]), (true, vec![(45.0, 55.0)])] {
        let wrap = vectorcraft_doc::WrapShape { path: shape.clone(), wrap: vectorcraft_doc::TextWrap { offset: 0.0, invert } };
        assert_spans(&cell_spans(&frame, &opts, &[wrap], 45.0, 55.0)[0], &want);
    }
}
