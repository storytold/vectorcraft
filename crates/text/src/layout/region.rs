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

/// `a` ∩ `b`.
fn intersect_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(s, e) in a {
        for &(c, d) in b {
            let (x, y) = (s.max(c), e.min(d));
            if y > x {
                out.push((x, y));
            }
        }
    }
    out
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

impl Region {
    pub(super) fn cells(path: &BezPath, opts: &LayoutOptions, wrap: &[vectorcraft_doc::WrapShape]) -> Vec<Region> {
        // A frame out of range has no cells, and a wrap shape out of range is left out.
        let Some(polys) = polygons(path) else { return vec![] };
        // Every cell shares one copy of the frame's polygons and one copy of the text wrap shapes.
        let polys: std::sync::Arc<[Vec<Point>]> = polys.into();
        let wraps: std::sync::Arc<[FlatWrap]> = wrap
            .iter()
            .filter_map(|w| Some((polygons(&w.path.to_bezpath())?, w.wrap.offset.max(0.0), w.wrap.invert)))
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
    /// inside it over the whole band, for Invert Wrap). Offsets grow each wrap shape vertically
    /// and horizontally.
    fn wrapped(&self, mut iv: Vec<(f64, f64)>, top: f64, bottom: f64) -> Vec<(f64, f64)> {
        let (top, bottom) = (top.min(bottom), top.max(bottom));
        for (polys, off, invert) in self.wraps.iter() {
            if iv.is_empty() {
                break;
            }
            let grow = |v: Vec<(f64, f64)>| union_intervals(v.into_iter().map(|(a, b)| (a - off, b + off)).collect());
            iv = if *invert {
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
                if *off > 0.0 {
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
                    iv = intersect_intervals(&iv, &grow(held));
                    if iv.is_empty() {
                        break;
                    }
                }
                iv
            } else {
                subtract_intervals(&iv, &grow(band_cover(polys, top - off, bottom + off, false)))
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
        let clip = |(a, b): (f64, f64)| {
            let (a, b) = (a.max(self.cell.x0) + self.inset, b.min(self.cell.x1) - self.inset);
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
        let free = if plain { vec![(self.cell.x0, self.cell.x1)] } else { band_intervals(&self.polys, clamp(top), clamp(bottom)) };
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

#[cfg(test)]
fn polygon_path(contours: &[&[(f64, f64)]]) -> BezPath {
    let mut path = BezPath::new();
    for c in contours {
        path.move_to(c[0]);
        for &p in &c[1..] {
            path.line_to(p);
        }
        path.close_path();
    }
    path
}

#[cfg(test)]
fn assert_spans(got: &[(f64, f64)], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?} != {want:?}");
    assert!(got.iter().zip(want).all(|(g, w)| (g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9), "{got:?} != {want:?}");
}

/// A line stays inside a frame whose right edge pinches in to a point between any fixed heights of
/// the line's band, with or without text wrap shapes elsewhere.
#[cfg(test)]
#[test]
fn a_line_stays_inside_a_frame_that_pinches_in_within_its_band() {
    let notch = |tip: (f64, f64), (y0, y1): (f64, f64)| {
        polygon_path(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, y0), tip, (100.0, y1), (100.0, 100.0), (0.0, 100.0)]])
    };
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
        let path = PathData::from_bezpath(&polygon_path(&[shape]));
        let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset, invert } };
        let cells = Region::cells(&frame, &LayoutOptions::default(), &[wrap]);
        assert!(cells[0].rect);
        cells[0].spans(40.0, 60.0)
    };
    assert_spans(&spans(&[(100.0, 45.0), (40.0, 47.5), (100.0, 50.0)], 0.0, false), &[(0.0, 40.0)]);
    assert_spans(&spans(&[(100.0, 47.0), (40.0, 47.5), (100.0, 48.0)], 1.0, false), &[(0.0, 39.0)]);
    // Two contours that cancel each other (even-odd) hold no area and leave the line whole.
    let band: &[(f64, f64)] = &[(0.0, 45.0), (100.0, 45.0), (100.0, 55.0), (0.0, 55.0)];
    let path = PathData::from_bezpath(&polygon_path(&[band, band]));
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
    let path = PathData::from_bezpath(&polygon_path(&[upper, lower]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert_spans(&Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0), &[(40.0, 45.0)]);
    // An Invert Wrap shape of two parts whose offsets meet across the gap between them (45.25 + 5
    // reaches past 55.125 - 5) keeps the whole line.
    let upper: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 0.0), (100.0, 45.25), (0.0, 45.25)];
    let lower: &[(f64, f64)] = &[(0.0, 55.125), (100.0, 55.125), (100.0, 100.0), (0.0, 100.0)];
    let path = PathData::from_bezpath(&polygon_path(&[upper, lower]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert_spans(&Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // A gap the offsets do not bridge (45 + 5 stops short of 55.125 - 5) holds no line.
    let upper: &[(f64, f64)] = &[(0.0, 0.0), (100.0, 0.0), (100.0, 45.0), (0.0, 45.0)];
    let path = PathData::from_bezpath(&polygon_path(&[upper, lower]));
    let wrap = vectorcraft_doc::WrapShape { path, wrap: vectorcraft_doc::TextWrap { offset: 5.0, invert: true } };
    assert!(Region::cells(&frame, &LayoutOptions::default(), &[wrap])[0].spans(40.0, 60.0).is_empty());
}

/// A hole that starts and ends inside a line's band splits the line, and a band below the hole, or
/// one that only touches it, keeps the whole width.
#[cfg(test)]
#[test]
fn a_hole_inside_a_band_splits_the_line() {
    let frame = polygon_path(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], &[(40.0, 52.0), (60.0, 52.0), (60.0, 58.0), (40.0, 58.0)]]);
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
    let seam = polygon_path(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], &[(0.0, 50.0), (100.0, 50.0), (100.0, 100.0), (0.0, 100.0)]]);
    let cells = Region::cells(&seam, &LayoutOptions::default(), &[]);
    assert!(!cells[0].rect);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    let step = polygon_path(&[&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], &[(0.0, 50.0), (50.0, 50.0), (50.0, 100.0), (0.0, 100.0)]]);
    let cells = Region::cells(&step, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 50.0)]);
    // Contours that meet along a seam that bends within the band, one drawn each way along it.
    let bent = polygon_path(&[
        &[(0.0, 0.0), (50.0, 0.0), (70.0, 47.5), (50.0, 100.0), (0.0, 100.0)],
        &[(50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 100.0), (70.0, 47.5)],
    ]);
    let cells = Region::cells(&bent, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // Contours side by side, meeting along a vertical edge.
    let side = polygon_path(&[&[(0.0, 0.0), (50.0, 0.0), (50.0, 100.0), (0.0, 100.0)], &[(50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 100.0)]]);
    let cells = Region::cells(&side, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // The same, with an extra anchor on one side of the shared edge.
    let split = polygon_path(&[
        &[(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (50.0, 100.0), (0.0, 100.0)],
        &[(50.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 100.0)],
    ]);
    let cells = Region::cells(&split, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // A slanted shared edge with an extra anchor on one side.
    let slanted = polygon_path(&[
        &[(0.0, 0.0), (40.0, 0.0), (50.0, 50.0), (60.0, 100.0), (0.0, 100.0)],
        &[(40.0, 0.0), (100.0, 0.0), (100.0, 100.0), (60.0, 100.0)],
    ]);
    let cells = Region::cells(&slanted, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 100.0)]);
    // Slanted edges of two contours 1e-7 pt apart are not one edge: the line stops at each.
    let near = polygon_path(&[
        &[(0.0, 0.0), (40.0, 0.0), (60.0, 100.0), (0.0, 100.0)],
        &[(40.0 + 1e-7, 0.0), (100.0, 0.0), (100.0, 100.0), (60.0 + 1e-7, 100.0)],
    ]);
    let cells = Region::cells(&near, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(0.0, 48.0), (52.0 + 1e-7, 100.0)]);
    // The same 1e-6 pt apart, 1e8 pt from the origin.
    let (x, y) = (1e8, 1e8);
    let far = polygon_path(&[
        &[(x, y), (x + 40.0, y), (x + 60.0, y + 100.0), (x, y + 100.0)],
        &[(x + 40.0 + 1e-6, y), (x + 100.0, y), (x + 100.0, y + 100.0), (x + 60.0 + 1e-6, y + 100.0)],
    ]);
    let cells = Region::cells(&far, &LayoutOptions::default(), &[]);
    let spans = cells[0].spans(y + 40.0, y + 60.0);
    assert!(spans.len() == 2 && spans[0].1 < x + 48.001 && spans[1].0 > x + 51.999, "{spans:?}");
    // A real gap of 0.01 pt between contours far from the origin stays a gap.
    let x = 1e8;
    let apart = polygon_path(&[
        &[(x, 0.0), (x + 50.0, 0.0), (x + 50.0, 100.0), (x, 100.0)],
        &[(x + 50.01, 0.0), (x + 100.0, 0.0), (x + 100.0, 100.0), (x + 50.01, 100.0)],
    ]);
    let cells = Region::cells(&apart, &LayoutOptions::default(), &[]);
    assert_spans(&cells[0].spans(40.0, 60.0), &[(x, x + 50.0), (x + 50.01, x + 100.0)]);
    // So does a gap of 1e-7 pt between vertical edges there, a few units of rounding wide.
    let apart = polygon_path(&[
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
    let cells = Region::cells(&polygon_path(&strips), &LayoutOptions::default(), &[]);
    assert!(cells[0].spans(40.0, 60.0).is_empty());
    assert!(!cells[0].spans(50.0, 50.0).is_empty());
}

/// A frame shorter than the band's margin at the frame's top and bottom holds no line and does
/// not panic; a rectangle keeps its inset spans, top and bottom.
#[cfg(test)]
#[test]
fn flat_frames_and_rectangles_keep_their_spans() {
    for height in [0.0, 1e-5] {
        let flat = polygon_path(&[&[(0.0, 0.0), (10.0, 0.0), (0.0, height)], &[(20.0, 0.0), (30.0, 0.0), (20.0, height)]]);
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
