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

/// Inside intervals (even-odd) of the horizontal line at `y` through `polys`, less the points
/// closer than `inset` to any edge of them (holes included), measured from the edge itself.
fn inset_intervals(polys: &[Vec<Point>], y: f64, inset: f64) -> Vec<(f64, f64)> {
    let iv = poly_intervals(polys, y);
    if inset <= 0.0 || iv.is_empty() {
        return iv;
    }
    if !inset.is_finite() {
        return vec![];
    }
    let near = polys.iter().flat_map(|p| (0..p.len()).filter_map(move |i| near_segment(p[i], p[(i + 1) % p.len()], y, inset)));
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

/// `a` minus `b` (both unions of disjoint intervals).
fn subtract_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(mut s, e) in a {
        for &(c, d) in b {
            if d <= s || c >= e {
                continue;
            }
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

    /// Inside intervals (even-odd) of the horizontal line at `y`, minus the wrap objects (or
    /// inside them, for Invert Wrap). Offsets grow each wrap shape vertically and horizontally.
    /// On a shaped frame the intervals keep the inset from the frame's edges, measured from the
    /// edge itself. Along the line, the text also keeps the inset beside each wrap object (outside
    /// it, or inside it for Invert Wrap); above and below a wrap object only its offset applies.
    fn intervals(&self, y: f64) -> Vec<(f64, f64)> {
        let mut iv =
            if self.polys.is_empty() || self.rect { vec![(self.cell.x0, self.cell.x1)] } else { inset_intervals(&self.polys, y, self.inset) };
        for (polys, off, invert) in self.wraps.iter() {
            let ys = if *off > 0.0 { vec![y - off, y - off * 0.5, y, y + off * 0.5, y + off] } else { vec![y] };
            let w = union_intervals(ys.into_iter().flat_map(|y| poly_intervals(polys, y)).map(|(a, b)| (a - off, b + off)).collect());
            iv = if *invert {
                let w: Vec<_> = w.into_iter().map(|(a, b)| (a + self.inset, b - self.inset)).filter(|(a, b)| b > a).collect();
                intersect_intervals(&iv, &w)
            } else {
                let w = union_intervals(w.into_iter().map(|(a, b)| (a - self.inset, b + self.inset)).collect());
                subtract_intervals(&iv, &w)
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
        // The inset from the cell's edges; `intervals` already keeps it from the frame's edges and
        // beside the wrap objects.
        let clip = |(a, b): (f64, f64)| {
            let (a, b) = (a.max(self.cell.x0 + self.inset), b.min(self.cell.x1 - self.inset));
            (b > a).then_some((a, b))
        };
        let plain = self.polys.is_empty() || self.rect;
        if plain && self.wraps.is_empty() {
            return (self.cell.width() > 0.0).then_some((self.cell.x0, self.cell.x1)).and_then(clip).into_iter().collect();
        }
        let fb = if plain {
            self.cell
        } else {
            self.polys.iter().flatten().fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p))
        };
        let clamp = |y: f64| if plain { y } else { y.clamp(fb.y0 + 1e-4, fb.y1 - 1e-4) };
        // Sample the band densely enough for curved frames (circles, blobs).
        let samples = 5;
        let mut rows: Vec<Vec<(f64, f64)>> =
            (0..samples).map(|k| self.intervals(clamp(top + (bottom - top) * k as f64 / (samples - 1) as f64))).collect();
        let mid = rows.swap_remove(samples / 2);
        if !self.wraps.is_empty() {
            // Wrap objects split lines: keep exactly what is free on every sampled row.
            let free = rows.iter().fold(mid, |acc, r| intersect_intervals(&acc, r));
            return free.into_iter().filter_map(clip).collect();
        }
        mid.into_iter()
            .filter_map(|(mut a, mut b)| {
                for o in &rows {
                    let best =
                        o.iter().map(|&(c, d)| (a.max(c), b.min(d))).filter(|(c, d)| d > c).max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))?;
                    a = best.0;
                    b = best.1;
                }
                clip((a, b))
            })
            .collect()
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
        assert!((g.0 - w.0).abs() < 1e-6 && (g.1 - w.1).abs() < 1e-6, "{got:?} != {want:?}");
    }
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
