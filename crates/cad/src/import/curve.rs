//! CAD curves as Bézier paths: polylines with bulges (arc segments), circular and elliptical
//! arcs, and splines (NURBS of any degree, rational or not, or a curve through fit points).
//!
//! Curves are built in drawing units with tolerances relative to their own size, so they keep
//! their accuracy at any scale the drawing is imported at.

use kurbo::{Arc, BezPath, Shape, Vec2};
use vectorcraft_geom::{Affine, Point};

/// Arcs are approximated to within this fraction of their radius.
const ARC_TOLERANCE: f64 = 1e-6;
/// Splines are approximated to within this fraction of their control polygon's size.
const SPLINE_TOLERANCE: f64 = 1e-6;
/// The most Bézier pieces one spline span may become.
const MAX_DEPTH: u32 = 12;
/// The most path elements one spline may become (beyond, its pieces aren't split further).
const MAX_ELEMENTS: usize = 200_000;
/// The highest spline degree read.
const MAX_DEGREE: usize = 11;
/// Spline weights below this fraction of the largest are read as this fraction.
const MIN_RELATIVE_WEIGHT: f64 = 1e-100;

/// A polyline vertex: its point and the bulge of the segment that starts there (the tangent of a
/// quarter of the arc's angle; positive turns counter-clockwise).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Vertex {
    pub p: Point,
    pub bulge: f64,
}

/// A polyline: straight segments, and arcs where a vertex has a bulge.
pub(crate) fn polyline(verts: &[Vertex], closed: bool) -> Option<BezPath> {
    let first = verts.first()?;
    let mut bp = BezPath::new();
    bp.move_to(first.p);
    let n = verts.len();
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        let (Some(a), Some(b)) = (verts.get(i), verts.get((i + 1) % n)) else { break };
        bulge_to(&mut bp, a.p, b.p, a.bulge);
    }
    if closed {
        bp.close_path();
    }
    Some(bp)
}

/// A segment from `a` (the current point) to `b`, an arc when `bulge` isn't 0.
fn bulge_to(bp: &mut BezPath, a: Point, b: Point, bulge: f64) {
    let chord = b - a;
    let len = chord.hypot();
    if bulge.abs() < 1e-12 || len < 1e-12 || !bulge.is_finite() {
        bp.line_to(b);
        return;
    }
    // The included angle is 4·atan(bulge); the centre lies off the chord's middle.
    let sweep = 4.0 * bulge.atan();
    let radius = len / (2.0 * (sweep / 2.0).sin()).abs();
    let normal = Vec2::new(-chord.y, chord.x) / len;
    let offset = (radius * radius - len * len / 4.0).max(0.0).sqrt();
    // A counter-clockwise arc under a half turn has its centre left of the chord.
    let side = if (bulge > 0.0) == (bulge.abs() < 1.0) { 1.0 } else { -1.0 };
    let centre = a.midpoint(b) + normal * offset * side;
    let start = (a - centre).atan2();
    arc_to(bp, centre, radius, start, sweep);
}

/// An arc of a circle from angle `start` (radians) through `sweep` (positive: counter-clockwise),
/// from the current point.
fn arc_to(bp: &mut BezPath, centre: Point, radius: f64, start: f64, sweep: f64) {
    let arc = Arc::new(centre, (radius, radius), start, sweep, 0.0);
    for el in arc.append_iter((radius * ARC_TOLERANCE).max(1e-12)) {
        bp.push(el);
    }
}

/// A circular arc from `start` through `sweep` radians as its own subpath; a whole circle closes.
pub(crate) fn arc(centre: Point, radius: f64, start: f64, sweep: f64) -> Option<BezPath> {
    if !(radius > 0.0 && radius.is_finite() && sweep.is_finite()) {
        return None;
    }
    let full = sweep.abs() >= std::f64::consts::TAU - 1e-9;
    let sweep = sweep.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    let mut bp = BezPath::new();
    bp.move_to(centre + Vec2::from_angle(start) * radius);
    arc_to(&mut bp, centre, radius, start, sweep);
    if full {
        bp.close_path();
    }
    Some(bp)
}

/// An elliptical arc: the unit circle's arc from `start` through `sweep` radians mapped by `frame`
/// (its columns are the major and minor semi-axes, its translation the centre).
pub(crate) fn ellipse(frame: Affine, start: f64, sweep: f64) -> Option<BezPath> {
    let [a, b, c, d, _, _] = frame.as_coeffs();
    let size = a.hypot(b).max(c.hypot(d));
    if !(size > 1e-12 && size.is_finite()) {
        return None;
    }
    let full = sweep.abs() >= std::f64::consts::TAU - 1e-9;
    let sweep = sweep.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    let unit = Arc::new(Point::ZERO, (1.0, 1.0), start, sweep, 0.0);
    let mut bp = BezPath::new();
    bp.move_to(Point::ZERO + Vec2::from_angle(start));
    for el in unit.append_iter(ARC_TOLERANCE) {
        bp.push(el);
    }
    if full {
        bp.close_path();
    }
    bp.apply_affine(frame);
    Some(bp)
}

/// A point with a weight, in homogeneous form (`x·w, y·w, w`).
#[derive(Clone, Copy, Debug)]
struct H {
    x: f64,
    y: f64,
    w: f64,
}

impl H {
    fn lerp(self, o: Self, t: f64) -> Self {
        // Exact at both ends, so a weight far smaller than its neighbour's doesn't cancel to 0.
        let s = 1.0 - t;
        Self { x: self.x * s + o.x * t, y: self.y * s + o.y * t, w: self.w * s + o.w * t }
    }
    fn point(self) -> Option<Point> {
        (self.w.abs() > 1e-300).then(|| Point::new(self.x / self.w, self.y / self.w)).filter(|p| p.x.is_finite() && p.y.is_finite())
    }
}

/// A NURBS curve of `degree` with `knots`, control points `ctrl` and their `weights` (none:
/// all 1) as Bézier pieces: exact for polynomial curves up to degree 3, else within
/// [`SPLINE_TOLERANCE`] of the control polygon's size.
pub(crate) fn nurbs(degree: usize, knots: &[f64], ctrl: &[Point], weights: Option<&[f64]>) -> Option<BezPath> {
    let n = ctrl.len();
    if !(1..=MAX_DEGREE).contains(&degree) || n <= degree || knots.len() != n + degree + 1 {
        return None;
    }
    if knots.windows(2).any(|w| w[1] < w[0]) {
        return None;
    }
    let raw = |i: usize| weights.and_then(|w| w.get(i)).copied().filter(|w| *w > 0.0 && w.is_finite()).unwrap_or(1.0);
    // Scaling all weights alike leaves the curve unchanged: relative to the largest, so tiny ones
    // don't underflow, and floored so a weight tiny next to the others can't either.
    let max_w = (0..n).map(raw).fold(0.0, f64::max);
    let weight = |i: usize| (raw(i) / max_w).max(MIN_RELATIVE_WEIGHT);
    let hs: Vec<H> = ctrl.iter().enumerate().map(|(i, p)| H { x: p.x * weight(i), y: p.y * weight(i), w: weight(i) }).collect();
    let rational = (0..n).any(|i| (weight(i) - weight(0)).abs() > 1e-12);
    let (lo, hi) = ctrl.iter().fold((Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN)), |(lo, hi), p| {
        (Point::new(lo.x.min(p.x), lo.y.min(p.y)), Point::new(hi.x.max(p.x), hi.y.max(p.y)))
    });
    let tol = ((hi - lo).hypot() * SPLINE_TOLERANCE).max(1e-12);
    let mut bp = BezPath::new();
    // Each knot span of the domain is one polynomial piece: its Bézier points are blossoms.
    for span in degree..n {
        let (Some(&t0), Some(&t1)) = (knots.get(span), knots.get(span + 1)) else { continue };
        if t1 - t0 <= 1e-12 * (1.0 + t0.abs()) {
            continue;
        }
        let bez: Vec<H> = (0..=degree)
            .map(|j| {
                let args: Vec<f64> = (0..degree).map(|k| if k < degree - j { t0 } else { t1 }).collect();
                blossom(degree, knots, &hs, span, &args)
            })
            .collect::<Option<_>>()?;
        let start = bez.first().and_then(|h| h.point())?;
        if bp.elements().is_empty() {
            bp.move_to(start);
        }
        let pts: Option<Vec<Point>> = if rational { None } else { bez.iter().map(|h| h.point()).collect() };
        match pts.as_deref() {
            Some([_, p]) => bp.line_to(*p),
            Some([_, q, p]) => bp.quad_to(*q, *p),
            Some([_, a, b, p]) => bp.curve_to(*a, *b, *p),
            _ => fit(&mut bp, &bez, 0.0, 1.0, tol, 0)?,
        }
    }
    (bp.elements().len() > 1).then_some(bp)
}

/// The blossom of span `span` at `args` (de Boor's algorithm with one parameter per level).
fn blossom(degree: usize, knots: &[f64], ctrl: &[H], span: usize, args: &[f64]) -> Option<H> {
    let mut d: Vec<H> = ctrl.get(span - degree..=span)?.to_vec();
    for r in 1..=degree {
        let u = *args.get(r - 1)?;
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let (a, b) = (*knots.get(i)?, *knots.get(i + degree + 1 - r)?);
            let alpha = if b - a > 1e-300 { (u - a) / (b - a) } else { 0.0 };
            let prev = *d.get(j - 1)?;
            let cur = d.get_mut(j)?;
            *cur = prev.lerp(*cur, alpha);
        }
    }
    d.get(degree).copied()
}

/// A (rational) Bézier of any degree at `t`, with its derivative.
fn eval(bez: &[H], t: f64) -> Option<(Point, Vec2)> {
    let mut pts = bez.to_vec();
    let n = pts.len().checked_sub(1)?;
    // Down to the two points whose difference gives the tangent.
    for level in (1..=n).rev() {
        if level == 1 {
            let (a, b) = (*pts.first()?, *pts.get(1)?);
            let p = a.lerp(b, t);
            let pt = p.point()?;
            // d/dt (X/W) = (X'·W − X·W') / W², with X' = n·(b − a).
            let k = n as f64;
            let (dx, dy, dw) = (k * (b.x - a.x), k * (b.y - a.y), k * (b.w - a.w));
            let w2 = p.w * p.w;
            let tangent = Vec2::new((dx * p.w - p.x * dw) / w2, (dy * p.w - p.y * dw) / w2);
            return Some((pt, tangent));
        }
        for i in 0..level {
            let next = *pts.get(i + 1)?;
            let cur = pts.get_mut(i)?;
            *cur = cur.lerp(next, t);
        }
    }
    // Degree 0: a point.
    Some((pts.first()?.point()?, Vec2::ZERO))
}

/// Cubic pieces through the curve `bez` from `t0` to `t1`, each matching its ends and their
/// tangents, split until within `tol` of the curve.
fn fit(bp: &mut BezPath, bez: &[H], t0: f64, t1: f64, tol: f64, depth: u32) -> Option<()> {
    let (p0, d0) = eval(bez, t0)?;
    let (p3, d3) = eval(bez, t1)?;
    let h = (t1 - t0) / 3.0;
    let (c1, c2) = (p0 + d0 * h, p3 - d3 * h);
    // Non-finite tangents never fit, so splitting wouldn't help: a line.
    if ![c1, c2].iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        bp.line_to(p3);
        return Some(());
    }
    let cubic = kurbo::CubicBez::new(p0, c1, c2, p3);
    let close = (1..8).all(|i| {
        let s = f64::from(i) / 8.0;
        eval(bez, t0 + (t1 - t0) * s).is_some_and(|(p, _)| kurbo::ParamCurve::eval(&cubic, s).distance(p) <= tol)
    });
    if close || depth >= MAX_DEPTH || bp.elements().len() >= MAX_ELEMENTS {
        if [c1, c2].iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            bp.curve_to(c1, c2, p3);
        } else {
            bp.line_to(p3);
        }
        return Some(());
    }
    let mid = (t0 + t1) / 2.0;
    fit(bp, bez, t0, mid, tol, depth + 1)?;
    fit(bp, bez, mid, t1, tol, depth + 1)
}

/// A smooth curve through `pts` (a spline given by fit points only): Catmull-Rom tangents.
pub(crate) fn through(pts: &[Point], closed: bool) -> Option<BezPath> {
    let n = pts.len();
    let first = *pts.first()?;
    if n < 2 {
        return None;
    }
    let at = |i: isize| -> Point {
        let len = n as isize;
        let j = if closed { i.rem_euclid(len) } else { i.clamp(0, len - 1) };
        pts.get(j as usize).copied().unwrap_or(first)
    };
    let mut bp = BezPath::new();
    bp.move_to(first);
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs as isize {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        bp.curve_to(p1 + (p2 - p0) / 6.0, p2 - (p3 - p1) / 6.0, p2);
    }
    if closed {
        bp.close_path();
    }
    Some(bp)
}

/// `bp`'s bounds, when it has any finite extent.
pub(crate) fn bounds(bp: &BezPath) -> Option<vectorcraft_geom::Rect> {
    if bp.elements().is_empty() {
        return None;
    }
    let r = bp.bounding_box();
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then_some(r)
}

#[cfg(test)]
mod tests {
    use kurbo::PathEl;

    use super::*;

    /// The largest distance from `centre` minus `r` over points along `bp`.
    fn radial_error(bp: &BezPath, centre: Point, r: f64) -> f64 {
        let mut worst: f64 = 0.0;
        for seg in bp.segments() {
            for i in 0..=32 {
                let p = kurbo::ParamCurve::eval(&seg, f64::from(i) / 32.0);
                worst = worst.max((p.distance(centre) - r).abs());
            }
        }
        worst
    }

    #[test]
    fn a_bulge_of_one_is_a_half_circle_on_the_right_of_its_chord() {
        let verts = [Vertex { p: Point::new(0.0, 0.0), bulge: 1.0 }, Vertex { p: Point::new(10.0, 0.0), bulge: 0.0 }];
        let bp = polyline(&verts, false).unwrap();
        assert!(radial_error(&bp, Point::new(5.0, 0.0), 5.0) < 1e-4);
        // Counter-clockwise from (0, 0) to (10, 0) passes below the chord.
        let b = bp.bounding_box();
        assert!((b.y0 + 5.0).abs() < 1e-3 && b.y1.abs() < 1e-9, "{b:?}");
        // A negative bulge turns the other way; a small one is a shallow arc.
        let verts = [Vertex { p: Point::new(0.0, 0.0), bulge: -0.25 }, Vertex { p: Point::new(10.0, 0.0), bulge: 0.0 }];
        let bp = polyline(&verts, false).unwrap();
        // Sagitta = bulge · chord / 2; radius = chord (1 + bulge²) / (4 |bulge|).
        let b = bp.bounding_box();
        assert!((b.y1 - 1.25).abs() < 1e-4 && b.y0.abs() < 1e-9, "{b:?}");
        let r = 10.0 * (1.0 + 0.0625) / 1.0;
        assert!(radial_error(&bp, Point::new(5.0, 1.25 - r), r) < 1e-4);
    }

    #[test]
    fn a_bulge_over_one_is_the_major_arc() {
        let b = (3.0 * std::f64::consts::PI / 8.0).tan(); // a 270° arc
        let verts = [Vertex { p: Point::new(0.0, 0.0), bulge: b }, Vertex { p: Point::new(10.0, 0.0), bulge: 0.0 }];
        let bp = polyline(&verts, false).unwrap();
        let r = 10.0 / (2.0 * (135f64).to_radians().sin());
        // Counter-clockwise, so the centre and most of the circle lie right of the chord.
        let centre = Point::new(5.0, -5.0);
        assert!(radial_error(&bp, centre, r) < 1e-4);
        assert!(bp.bounding_box().y0 < -12.0, "most of the circle lies below the chord");
    }

    #[test]
    fn a_rational_quadratic_circle_stays_on_the_circle() {
        // A whole circle of radius 100 as nine weighted control points (four quarter arcs).
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let ctrl = [(1., 0.), (1., 1.), (0., 1.), (-1., 1.), (-1., 0.), (-1., -1.), (0., -1.), (1., -1.), (1., 0.)]
            .map(|(x, y)| Point::new(x * 100.0, y * 100.0));
        let weights = [1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0];
        let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 4.0];
        let bp = nurbs(2, &knots, &ctrl, Some(&weights)).unwrap();
        let err = radial_error(&bp, Point::ZERO, 100.0);
        assert!(err < 0.01, "error {err}");
    }

    #[test]
    fn polynomial_splines_become_exact_beziers() {
        // A clamped cubic with one interior knot: two cubic pieces meeting at its blossom.
        let ctrl = [(0., 0.), (10., 20.), (30., 20.), (40., 0.), (50., -10.)].map(|(x, y)| Point::new(x, y));
        let knots = [0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0];
        let bp = nurbs(3, &knots, &ctrl, None).unwrap();
        assert_eq!(bp.elements().iter().filter(|e| matches!(e, PathEl::CurveTo(..))).count(), 2);
        assert!(matches!(bp.elements().first(), Some(PathEl::MoveTo(p)) if *p == Point::ZERO));
        let end = match bp.elements().last() {
            Some(PathEl::CurveTo(_, _, p)) => *p,
            other => panic!("{other:?}"),
        };
        assert_eq!(end, Point::new(50.0, -10.0));
        // Bad knot vectors are refused rather than read.
        assert!(nurbs(3, &knots[1..], &ctrl, None).is_none());
        assert!(nurbs(3, &[0.0, 0.0, 0.0, 0.0, 2.0, 1.0, 2.0, 2.0, 2.0], &ctrl, None).is_none());
        assert!(nurbs(0, &knots, &ctrl, None).is_none());
    }

    #[test]
    fn a_high_degree_spline_is_fitted_within_tolerance() {
        // Degree 5 Bézier (one span) of a known polynomial: compare with direct evaluation.
        let ctrl: Vec<Point> = (0..6).map(|i| Point::new(f64::from(i) * 10.0, if i % 2 == 0 { 0.0 } else { 30.0 })).collect();
        let knots = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let bp = nurbs(5, &knots, &ctrl, None).unwrap();
        let hs: Vec<H> = ctrl.iter().map(|p| H { x: p.x, y: p.y, w: 1.0 }).collect();
        for i in 0..=20 {
            let (p, _) = eval(&hs, f64::from(i) / 20.0).unwrap();
            let near = bp.segments().map(|s| kurbo::ParamCurveNearest::nearest(&s, p, 1e-9).distance_sq.sqrt()).fold(f64::MAX, f64::min);
            assert!(near < 0.01, "error {near}");
        }
    }

    #[test]
    fn tiny_spline_weights_flatten_to_a_handful_of_pieces() {
        // Degree 5, three knot spans: fitted, not exact.
        let ctrl: Vec<Point> = (0..8).map(|i| Point::new(f64::from(i) * 10.0, if i % 2 == 0 { 0.0 } else { 30.0 })).collect();
        let knots = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0];
        let normal = nurbs(5, &knots, &ctrl, Some(&[1.0; 8])).unwrap().elements().len();
        // Scaling every weight alike leaves the curve as it is.
        let tiny = nurbs(5, &knots, &ctrl, Some(&[1e-200; 8])).unwrap().elements().len();
        assert!(tiny <= normal + 8, "normal weights: {normal} elements, 1e-200 weights: {tiny} elements");
        // Weights tiny next to the others don't split every span to the limit either.
        let mixed = [1.0, 1e-200, 1.0, 1e-200, 1.0, 1e-200, 1.0, 1e-200];
        let mixed = nurbs(5, &knots, &ctrl, Some(&mixed)).unwrap().elements().len();
        assert!(mixed < 500, "mixed weights: {mixed} elements");
    }

    #[test]
    fn fit_points_make_a_smooth_curve_through_them() {
        let pts = [Point::new(0.0, 0.0), Point::new(10.0, 10.0), Point::new(20.0, 0.0)];
        let bp = through(&pts, false).unwrap();
        let ends: Vec<Point> = bp.elements().iter().filter_map(|e| if let PathEl::CurveTo(_, _, p) = e { Some(*p) } else { None }).collect();
        assert_eq!(ends, [Point::new(10.0, 10.0), Point::new(20.0, 0.0)]);
    }
}
