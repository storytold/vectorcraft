//! Gradients laid along or across a stroke ([`StrokeGradientMode::Along`] and
//! [`StrokeGradientMode::Across`]).
//!
//! Each subpath becomes a strip of *ribs*: cross-sections through the flattened path that reach
//! past everything the stroke paints there (its width profile, joins, caps and arrowheads). Between
//! two ribs lies a slice painted with a linear piece of the gradient: along, from the first rib's
//! centre to the next one's (padded beyond, so the outside of a turn keeps the colour of the point
//! it turns at); across, from the stroke's left edge to its right edge. Ribs sit on the bisectors
//! of gentle turns, so neighbouring slices meet where both give the same colour; a sharp corner
//! gets a fan of ribs round its outside, painted in the corner's colour.
//!
//! Clipped to the stroke's outline, the slices paint it: the canvas and the SVG and PDF writers
//! fill them ([`gradient_slices`]); Outline Stroke turns the strip into a gradient mesh
//! ([`gradient_meshes`]).

use kurbo::{BezPath, Point, Shape, Vec2};
use vectorcraft_doc::color::{Gradient, GradientGeom, GradientPaint, Paint};
use vectorcraft_doc::live::{H_DOWN, H_LEFT, H_RIGHT, H_UP};
use vectorcraft_doc::{GradientMesh, LineJoin, MeshPoint, StrokeAlign, StrokeGradientMode, StrokeLayer};
use vectorcraft_geom::FillRule;

use super::width::{flatten, left, length, split_at};
use super::{aligned_width, cap_extent, is_closed, segments, subpaths, unit};

/// Largest stretch of a rib on a turn's bisector (1 / cos of half the turn, about 74°) before the
/// corner gets a fan of ribs round its outside.
const SHARP: f64 = 1.25;
/// Past this stretch (a turn of about 150°) the inside of a corner is not cut on its bisector,
/// which would reach far back along both sides: the slices either side overlap there instead.
const HAIRPIN: f64 = 4.0;
/// Rows of a mesh across the stroke (more where the gradient has stops).
const MESH_ROWS: usize = 16;
/// The most of a subpath one column of a mesh along the stroke spans.
const MESH_STEP: f64 = 1.0 / 32.0;
/// The most of a subpath one slice spans while a width profile shapes the across gradient.
const PROFILE_STEP: f64 = 1.0 / 64.0;
/// Flattening tolerance, cover margin and growth (see [`gradient_slices`]) of meshes and written
/// slices (document points).
const SLICE_TOL: f64 = 0.05;
const SLICE_MARGIN: f64 = 0.5;
const SLICE_GROW: f64 = 0.25;

/// A cross-section of the strip a stroke's gradient is laid on.
#[derive(Clone, Copy, Debug)]
struct Rib {
    /// Where it crosses the path.
    c: Point,
    /// Its left and right ends: the strip runs from `l` through `c` to `r`.
    l: Point,
    r: Point,
    /// How far across the stroke `l` and `r` stand: a point `x` left of the path (negative:
    /// right) lies `x / reach` of the way from `c` to the end on its side.
    reach: (f64, f64),
    /// Fraction of the subpath's length at `c`.
    t: f64,
    /// Where the across gradient starts and ends: this far left and right of the path.
    band: (f64, f64),
}

impl Rib {
    /// A rib square to direction `d`, reaching `r` either side of `c`.
    fn square(c: Point, d: Vec2, r: f64, t: f64, band: (f64, f64)) -> Self {
        let n = left(d);
        Rib { c, l: c + n * r, r: c - n * r, reach: (r, r), t, band }
    }
    /// The point `x` left of the path along the rib (negative: right).
    fn at(&self, x: f64) -> Point {
        if x >= 0.0 {
            self.c + (self.l - self.c) * (x / self.reach.0.max(1e-12))
        } else {
            self.c + (self.r - self.c) * (-x / self.reach.1.max(1e-12))
        }
    }
}

/// What lays out the ribs of one stroke.
struct Lay<'a> {
    st: &'a StrokeLayer,
    /// Half the stroked width ([`aligned_width`]).
    hw: f64,
    /// How far ribs reach either side of the path where nothing wider is near.
    cover: f64,
    margin: f64,
    /// How far a strip runs on past the start and end of an open subpath (caps and heads).
    ext: (f64, f64),
}

impl Lay<'_> {
    /// The across band at fraction `t` of a subpath whose visible side is `side` (`Some(true)`:
    /// left; `None`: both, a centred stroke).
    fn band(&self, t: f64, side: Option<bool>) -> (f64, f64) {
        let (l, r) = self.st.profile.as_ref().map_or((1.0, 1.0), |p| p.at(t));
        let (l, r) = (self.hw * l.max(0.0), self.hw * r.max(0.0));
        match side {
            None => (l, r),
            Some(true) => (l, 0.0),
            Some(false) => (0.0, r),
        }
    }

    /// The ribs at a turn at `p` from direction `din` to `dout` (unit vectors), reaching `r`
    /// across: one on the bisector, or a fan round the outside of a sharp corner (its miter, or the
    /// box round its round or bevel join).
    fn turn(&self, p: Point, din: Vec2, dout: Vec2, r: f64, t: f64, band: (f64, f64)) -> Vec<Rib> {
        let (nin, nout) = (left(din), left(dout));
        let den = 1.0 + nin.dot(nout);
        let stretch = if den > 1e-12 { (2.0 / den).sqrt() } else { f64::INFINITY };
        if stretch <= SHARP {
            let m = (nin + nout) / den;
            return vec![Rib { c: p, l: p + m * r, r: p - m * r, reach: (r, r), t, band }];
        }
        // `s` is 1 when the outside of the turn is on the left; `out_dir` the outward bisector.
        let s = if dout.dot(nin) > 0.0 { -1.0 } else { 1.0 };
        let sum = nin + nout;
        let out_dir = if sum.hypot() > 1e-9 { unit(sum) * s } else { din };
        let (o1, o2) = (p + nin * (s * r), p + nout * (s * r));
        let mitered = stretch <= std::f64::consts::SQRT_2 || (self.st.join == LineJoin::Miter && stretch <= self.st.miter_limit);
        let outer = if mitered { vec![o1, p + out_dir * (stretch * r), o2] } else { vec![o1, o1 + din * r, o2 - dout * r, o2] };
        let last = outer.len() - 1;
        outer
            .into_iter()
            .enumerate()
            .map(|(k, o)| {
                let (inner, inner_reach) = match k {
                    _ if stretch <= HAIRPIN => (p - out_dir * (stretch * r), r),
                    0 => (p - nin * (s * r), r),
                    _ if k == last => (p - nout * (s * r), r),
                    _ => (p, 0.0),
                };
                let o_reach = o.distance(p);
                if s > 0.0 {
                    Rib { c: p, l: o, r: inner, reach: (o_reach, inner_reach), t, band }
                } else {
                    Rib { c: p, l: inner, r: o, reach: (inner_reach, o_reach), t, band }
                }
            })
            .collect()
    }
}

/// The ribs of each subpath of `bp` (filled with `rule`) under stroke `st`, flattened to `tol`,
/// reaching `margin` past what the stroke paints. Fractions `marks` of each subpath get ribs of
/// their own, and no two are more than `max_dt` of it apart.
fn strips(bp: &BezPath, rule: FillRule, st: &StrokeLayer, tol: f64, margin: f64, marks: &[f64], max_dt: f64) -> Vec<Vec<Rib>> {
    let closed_path = is_closed(bp);
    let hw = aligned_width(st, closed_path) / 2.0;
    if !(hw > 0.0 && hw.is_finite()) {
        return vec![];
    }
    let full = hw * st.profile_max();
    let heads = (st.head_reach(false), st.head_reach(true));
    let cap = cap_extent(st.cap, 2.0 * full);
    let lay = Lay { st, hw, cover: full + margin, margin, ext: (cap.max(heads.0) + margin, cap.max(heads.1) + margin) };
    let subs = subpaths(bp);
    let count = subs.len();
    let mut out = vec![];
    for (k, (range, closed)) in subs.into_iter().enumerate() {
        let (pts, corner) = flatten(&segments(&bp.elements()[range]), closed, tol.max(1e-4));
        let side =
            (st.align != StrokeAlign::Center && closed_path && closed).then(|| band_on_left(bp, rule, &pts, st.align, (4.0 * tol).min(hw / 2.0)));
        // Arrowheads sit on the start of the first subpath and the end of the last one.
        let heads = (if k == 0 { heads.0 } else { 0.0 }, if k + 1 == count { heads.1 } else { 0.0 });
        let ribs = match pts.len() {
            0 => continue,
            // A subpath of no length: a dot (round and projecting caps paint it).
            1 => {
                let (d, b, e) = (Vec2::new(1.0, 0.0), lay.band(0.0, side), lay.ext.0.max(lay.ext.1));
                vec![Rib::square(pts[0] - d * e, d, lay.cover, 0.0, b), Rib::square(pts[0] + d * e, d, lay.cover, 0.0, b)]
            }
            _ => {
                let total = length(&pts, closed);
                if total <= 1e-12 {
                    continue;
                }
                let steps = if max_dt < 1.0 { (1.0 / max_dt).ceil() as usize } else { 1 };
                let profile = st.profile.iter().flat_map(|p| p.points.iter().map(|q| q.0));
                let at: Vec<f64> =
                    marks.iter().copied().chain(profile).chain((1..steps).map(|i| i as f64 / steps as f64)).map(|t| t * total).collect();
                let (pts, _) = split_at(&pts, &corner, closed, &at);
                strip(&lay, &pts, closed, total, side, if closed { (0.0, 0.0) } else { heads })
            }
        };
        out.push(ribs);
    }
    out
}

/// Is the visible band of an inside or outside stroke (`align`) on the left of the closed
/// subpath `pts` of `bp`? Probes `off` left of the middle of its longest segment (past where
/// flattening moved it).
fn band_on_left(bp: &BezPath, rule: FillRule, pts: &[Point], align: StrokeAlign, off: f64) -> bool {
    let n = pts.len();
    let Some(i) = (0..n).max_by(|&a, &b| pts[a].distance(pts[(a + 1) % n]).total_cmp(&pts[b].distance(pts[(b + 1) % n]))) else { return true };
    let (a, b) = (pts[i], pts[(i + 1) % n]);
    let probe = a.midpoint(b) + left(unit(b - a)) * off.max(1e-6);
    let w = bp.winding(probe);
    let inside = if rule == FillRule::EvenOdd { w % 2 != 0 } else { w != 0 };
    inside == (align == StrokeAlign::Inside)
}

/// The ribs along one flattened subpath `pts` of length `total` (`heads`: how far arrowheads
/// reach from its start and end).
fn strip(lay: &Lay, pts: &[Point], closed: bool, total: f64, side: Option<bool>, heads: (f64, f64)) -> Vec<Rib> {
    let n = pts.len();
    let seg_count = if closed { n } else { n - 1 };
    let dir: Vec<Vec2> = (0..seg_count).map(|i| unit(pts[(i + 1) % n] - pts[i])).collect();
    // Ribs near an arrowhead reach across it.
    let reach = |s: f64| {
        let mut r = lay.cover;
        if heads.0 > 0.0 && s <= heads.0 {
            r = r.max(heads.0 + lay.margin);
        }
        if heads.1 > 0.0 && total - s <= heads.1 {
            r = r.max(heads.1 + lay.margin);
        }
        r
    };
    let mut out = Vec::with_capacity(n + 4);
    let mut s = 0.0;
    if !closed {
        let (d, b) = (dir[0], lay.band(0.0, side));
        out.push(Rib::square(pts[0] - d * lay.ext.0, d, reach(0.0), 0.0, b));
        out.push(Rib::square(pts[0], d, reach(0.0), 0.0, b));
        s = pts[0].distance(pts[1]);
    }
    let (first, last) = if closed { (0, n) } else { (1, n - 1) };
    for i in first..last {
        let v = i % n;
        let t = s / total;
        let ribs = lay.turn(pts[v], dir[(v + seg_count - 1) % seg_count], dir[v % seg_count], reach(s), t, lay.band(t, side));
        // A closed subpath starts after the fan of its first corner and ends with it.
        let skip = if closed && i == 0 { ribs.len() - 1 } else { 0 };
        out.extend(ribs.into_iter().skip(skip));
        s += pts[v].distance(pts[(v + 1) % n]);
    }
    if closed {
        out.extend(lay.turn(pts[0], dir[seg_count - 1], dir[0], reach(total), 1.0, lay.band(1.0, side)));
    } else {
        let (d, b) = (dir[seg_count - 1], lay.band(1.0, side));
        out.push(Rib::square(pts[n - 1], d, reach(total), 1.0, b));
        out.push(Rib::square(pts[n - 1] + d * lay.ext.1, d, reach(total), 1.0, b));
    }
    out
}

/// A piece of a stroke whose gradient runs along or across it ([`gradient_slices`]).
#[derive(Clone, Debug)]
pub struct Slice {
    /// The area it paints (clipped to the stroke's outline).
    pub shape: BezPath,
    /// Its piece of the gradient runs from position `span.0` at `from` to `span.1` at `to`, and
    /// on unchanged past them; one colour when the span is empty.
    pub from: Point,
    pub to: Point,
    pub span: (f64, f64),
}

impl Slice {
    /// The slice's paint: its piece of `g` as a linear gradient placed from `from` to `to`.
    pub fn paint(&self, g: &Gradient) -> Paint {
        let gradient = g.span(self.span.0 as f32, self.span.1 as f32);
        let end = if self.from.distance(self.to) > 1e-9 { self.to } else { self.from + Vec2::new(1.0, 0.0) };
        let geom = GradientGeom { start: self.from, end, aspect: 1.0, focal: None };
        Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), angle: geom.angle_deg(), ..GradientPaint::new(gradient) }))
    }
}

/// The slices that paint stroke `st` of `bp` (filled with `rule`) when its gradient runs along or
/// across the path (see the module docs): flattened to `tol`, reaching `margin` past the
/// stroke's edge. Clip them to the stroke's outline and fill them in order. With `grow` > 0 each
/// slice reaches that much past its edges into its neighbours, so antialiased edges show no
/// seams: filled so that each replaces what it covers (copy), they paint each point once; filled
/// over each other, only opaque gradients may grow (translucent ones would darken there).
pub fn gradient_slices(bp: &BezPath, rule: FillRule, st: &StrokeLayer, tol: f64, margin: f64, grow: f64) -> Vec<Slice> {
    let across = st.gradient_mode == StrokeGradientMode::Across;
    // A width profile changes the across band along the path: ribs close enough to follow it.
    let max_dt = if across && st.profile.is_some() { PROFILE_STEP } else { 1.0 };
    let mut out = vec![];
    for ribs in strips(bp, rule, st, tol, margin, &[], max_dt) {
        for w in ribs.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let shape = polygon(&[a.l, b.l, b.c, b.r, a.r, a.c], grow);
            if shape.elements().is_empty() {
                continue;
            }
            let (from, to, span) = if across {
                let axis = b.c - a.c;
                let n = if axis.hypot() > 1e-9 { left(unit(axis)) } else { unit((a.l - a.r) + (b.l - b.r)) };
                let mid = a.c.midpoint(b.c);
                let (l, r) = ((a.band.0 + b.band.0) / 2.0, (a.band.1 + b.band.1) / 2.0);
                (mid + n * l, mid - n * r, (0.0, 1.0))
            } else {
                (a.c, b.c, (a.t, b.t))
            };
            out.push(Slice { shape, from, to, span });
        }
    }
    out
}

/// The closed polygon through `pts`, each edge moved out by `grow` (mitred, at most 4 × `grow`
/// out at a corner). Repeated points and spikes are dropped; empty when no area is left.
fn polygon(pts: &[Point], grow: f64) -> BezPath {
    let mut v: Vec<Point> = pts.to_vec();
    // Drop repeated points and spikes (there and straight back), which have no outward side.
    loop {
        let n = v.len();
        let same = |a: Point, b: Point| a.distance(b) <= 1e-9;
        let Some(i) = (0..n).find(|&i| n > 2 && (same(v[i], v[(i + 1) % n]) || same(v[(i + n - 1) % n], v[(i + 1) % n]))) else { break };
        v.remove(i);
    }
    let mut out = BezPath::new();
    if v.len() < 3 {
        return out;
    }
    let n = v.len();
    if grow > 0.0 {
        let area: f64 = (0..n).map(|i| v[i].to_vec2().cross(v[(i + 1) % n].to_vec2())).sum();
        let side = if area >= 0.0 { 1.0 } else { -1.0 };
        let normal = |i: usize| {
            let d = unit(v[(i + 1) % n] - v[i]);
            Vec2::new(d.y, -d.x) * side
        };
        v = (0..n)
            .map(|i| {
                let (n1, n2) = (normal((i + n - 1) % n), normal(i));
                let den = 1.0 + n1.dot(n2);
                let m = if den > 1e-9 { (n1 + n2) / den } else { n1 };
                let m = if m.hypot() > 4.0 { m * (4.0 / m.hypot()) } else { m };
                v[i] + m * grow
            })
            .collect();
    }
    out.move_to(v[0]);
    for p in &v[1..] {
        out.line_to(*p);
    }
    out.close_path();
    out
}

/// What a writer paints for a stroke whose gradient runs along or across it.
#[derive(Clone, Debug)]
pub struct WrittenSlices {
    /// The outlines the stroke is written as, as one path (non-zero rule) to clip the slices.
    pub clip: BezPath,
    /// The slices in paint order, each with its piece of the gradient.
    pub slices: Vec<(BezPath, Paint)>,
}

/// How a file writer paints stroke `st` of `bp` (filled with `rule`) when its gradient runs along
/// or across it ([`StrokeLayer::path_gradient`]), given the filled outlines it writes for the
/// stroke ([`for_writer`](super::for_writer)): [`gradient_slices`] clipped to them. Where the
/// gradient is opaque the slices overlap a little, so viewers that antialias every shape show no
/// seams. `None` without such a gradient.
pub fn written_slices(bp: &BezPath, rule: FillRule, st: &StrokeLayer, outlines: &[BezPath]) -> Option<WrittenSlices> {
    let g = &st.path_gradient()?.gradient;
    let clip = match outlines {
        [one] => one.clone(),
        _ => vectorcraft_pathops::stroke_region(outlines).to_bezpath(),
    };
    let grow = if g.stops.iter().all(|s| s.opacity >= 1.0) { SLICE_GROW } else { 0.0 };
    let slices = gradient_slices(bp, rule, st, SLICE_TOL, SLICE_MARGIN, grow)
        .into_iter()
        .map(|s| {
            let paint = s.paint(g);
            (s.shape, paint)
        })
        .collect();
    Some(WrittenSlices { clip, slices })
}

/// Outline Stroke of stroke `st` of `bp` (filled with `rule`) whose gradient `g` runs along or
/// across it: a gradient mesh per subpath over all the stroke can paint there (clip it to the
/// stroke's outline), coloured as [`gradient_slices`] paint it.
pub fn gradient_meshes(bp: &BezPath, rule: FillRule, st: &StrokeLayer, g: &Gradient) -> Vec<GradientMesh> {
    let across = st.gradient_mode == StrokeGradientMode::Across;
    let color = |t: f64| g.sample_keep_model(t.clamp(0.0, 1.0) as f32);
    // Along: a column at each stop (colours are linear between columns), close enough to look smooth.
    let marks: Vec<f64> = if across { vec![] } else { g.expanded_stops().iter().map(|s| s.0 as f64).collect() };
    let max_dt = match (across, st.profile.is_some()) {
        (false, _) => MESH_STEP,
        (true, true) => PROFILE_STEP,
        (true, false) => 1.0,
    };
    // Rows across: the strip's edges, then (across) the band's left edge to its right edge with a row at each stop.
    let mut us: Vec<f64> = if across {
        (0..=MESH_ROWS).map(|i| i as f64 / MESH_ROWS as f64).chain(g.expanded_stops().iter().map(|s| s.0 as f64)).collect()
    } else {
        vec![0.5]
    };
    us.sort_by(f64::total_cmp);
    us.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    strips(bp, rule, st, SLICE_TOL, SLICE_MARGIN, &marks, max_dt)
        .into_iter()
        .filter(|ribs| ribs.len() > 1)
        .map(|ribs| {
            let (rows, cols) = (us.len() + 1, ribs.len() - 1);
            // Each level across: a point on each rib and the gradient position there.
            let level = |row: usize, rib: &Rib| -> (Point, f64) {
                let t = |u: f64| if across { u } else { rib.t };
                match row {
                    0 => (rib.l, t(0.0)),
                    _ if row == rows => (rib.r, t(1.0)),
                    _ if across => {
                        let (l, r) = rib.band;
                        (rib.at(l - us[row - 1] * (l + r)), us[row - 1])
                    }
                    _ => (rib.c, rib.t),
                }
            };
            let level = &level;
            let pos: Vec<Point> = (0..=rows).flat_map(|row| ribs.iter().map(move |rib| level(row, rib).0)).collect();
            let at = |row: usize, col: usize| pos[row * (cols + 1) + col];
            let mut points = Vec::with_capacity(pos.len());
            for row in 0..=rows {
                for (col, rib) in ribs.iter().enumerate() {
                    let p = at(row, col);
                    let toward = |o: Option<Point>| o.map_or(Vec2::ZERO, |o| (o - p) / 3.0);
                    let mut handles = [Vec2::ZERO; 4];
                    handles[H_RIGHT] = toward((col < cols).then(|| at(row, col + 1)));
                    handles[H_LEFT] = toward(col.checked_sub(1).map(|c| at(row, c)));
                    handles[H_DOWN] = toward((row < rows).then(|| at(row + 1, col)));
                    handles[H_UP] = toward(row.checked_sub(1).map(|r| at(r, col)));
                    let (color, opacity) = color(level(row, rib).1);
                    points.push(MeshPoint { p, color, opacity, handles });
                }
            }
            GradientMesh { rows: rows as u32, cols: cols as u32, points }
        })
        .collect()
}
