//! The circle that touches a curve at one of its points with the curve's curvature there (its
//! osculating circle): the circle the curve bends along at that point.

use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, Point, Vec2};

use crate::{AnchorKind, SubPath};

/// Why no circle matches a curve at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoCircle {
    /// The curve is straight there (a straight segment, or the turn of an S curve): the circle
    /// would have no end.
    Straight,
    /// The curve has no single direction there: a cusp, a handle drawn back onto its anchor, or a
    /// segment with no length.
    Cusp,
    /// Two segments meet at an anchor at an angle: the path has no single direction there.
    Corner,
}

/// The circle that touches a curve at one point and bends as the curve does there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvatureCircle {
    /// The point where the circle touches the curve.
    pub at: Point,
    pub center: Point,
    pub radius: f64,
    /// The segment is the usual cubic of a circular arc of 90° or less ([`circular_arc`]), and the
    /// circle has that arc's own radius rather than the cubic's curvature at the point.
    pub nominal: bool,
}

/// Largest turn (radians) of an arc that [`circular_arc`] takes: a quarter turn.
const MAX_ARC: f64 = std::f64::consts::FRAC_PI_2 * (1.0 + 1e-9);
/// How far apart (as a share of the radius) the arc's two ends may lie from its center.
const RADIUS_TOL: f64 = 1e-4;
/// How far each handle's length may differ from the circular arc's (as a share of it).
const HANDLE_TOL: f64 = 1e-3;
/// Below this sine of the angle between the curve's first two derivatives, the curve is straight.
const STRAIGHT_SIN: f64 = 1e-10;
/// Below this share of the control polygon's length, the curve's direction is not defined.
const CUSP_SHARE: f64 = 1e-9;
/// The largest turn (radians) at an anchor not marked smooth that still counts as smooth: half a
/// degree, which takes in handles rounded to a few decimals in imported files.
const SMOOTH_TURN: f64 = 0.5 * std::f64::consts::PI / 180.0;

/// The circle that touches segment `c` at `t` (0 to 1) with its curvature there, on the side the
/// segment bends toward, or on the other side with `convex`. A segment that is the usual cubic of
/// a circular arc of 90° or less gives a circle of that arc's radius: on the concave side it is the
/// arc's own circle, and it touches the arc's circle at the point nearest `c` at `t`; on the convex
/// side it touches `c` at `t`, its center one radius out from there. A `t` outside 0 to
/// 1 and a segment whose points are not finite give [`NoCircle::Cusp`].
pub fn curvature_circle(c: CubicBez, t: f64, convex: bool) -> Result<CurvatureCircle, NoCircle> {
    curvature_circle_with(c, t, convex, true)
}

/// [`curvature_circle`], with `arcs` off giving the cubic's own curvature at `t` on the cubic of a
/// circular arc too.
pub fn curvature_circle_with(c: CubicBez, t: f64, convex: bool, arcs: bool) -> Result<CurvatureCircle, NoCircle> {
    let finite = [c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.is_finite());
    if !finite || !(0.0..=1.0).contains(&t) {
        return Err(NoCircle::Cusp);
    }
    let at = c.eval(t);
    let (center, radius, nominal) = match circular_arc(c).filter(|_| arcs) {
        Some((center, radius)) => {
            let out = at - center;
            let len = out.hypot();
            if len == 0.0 || !len.is_finite() {
                return Err(NoCircle::Cusp);
            }
            if convex {
                // One radius out from `at`, so the circle touches the segment at `at` even where
                // the cubic strays a little from the arc's circle.
                return Ok(CurvatureCircle { at, center: at + out * (radius / len), radius, nominal: true });
            }
            // The point of the arc's circle nearest `at`: the circle touches the segment there.
            let on = center + out * (radius / len);
            return Ok(CurvatureCircle { at: on, center, radius, nominal: true });
        }
        None => {
            let (center, radius) = osculating(c, t)?;
            (center, radius, false)
        }
    };
    let center = if convex { at + (at - center) } else { center };
    Ok(CurvatureCircle { at, center, radius, nominal })
}

/// [`curvature_circle`] at `t` on segment `seg` of `sp`. At an end of the segment where it meets
/// the next or previous segment at a corner, the path has no single direction: that is a
/// [`NoCircle::Corner`]. A segment `sp` lacks is a [`NoCircle::Cusp`]. Segments with no length
/// between the two are passed over, and a closed subpath with one segment of length meets itself.
/// The anchor is smooth when it is marked smooth, with both handles pulled out and no segment of no
/// length passed over to reach it, and the path turns there by less than 90°, or when
/// the two segments leave it within [`SMOOTH_TURN`] of one direction.
pub fn curvature_circle_on(sp: &SubPath, seg: usize, t: f64, convex: bool) -> Result<CurvatureCircle, NoCircle> {
    curvature_circle_on_with(sp, seg, t, convex, true)
}

/// [`curvature_circle_on`] with [`curvature_circle_with`]'s `arcs`.
pub fn curvature_circle_on_with(sp: &SubPath, seg: usize, t: f64, convex: bool, arcs: bool) -> Result<CurvatureCircle, NoCircle> {
    let n = sp.segment_count();
    if seg >= n {
        return Err(NoCircle::Cusp);
    }
    let c = sp.segment(seg);
    let corner = if start_direction(c) == Vec2::ZERO {
        false
    } else if t == 0.0 {
        neighbor(sp, seg, false).is_some_and(|(prev, skipped)| !smooth(sp, seg, skipped, end_direction(prev), start_direction(c)))
    } else if t == 1.0 {
        neighbor(sp, seg, true).is_some_and(|(next, skipped)| !smooth(sp, (seg + 1) % n, skipped, end_direction(c), start_direction(next)))
    } else {
        false
    };
    if corner {
        return Err(NoCircle::Corner);
    }
    curvature_circle_with(c, t, convex, arcs)
}

/// The nearest segment with length before (or with `forward`, after) segment `seg`, and whether
/// segments with no length lay between; `None` at an open end of the subpath. In a closed subpath
/// whose only segment with length is `seg`, that is `seg` itself.
fn neighbor(sp: &SubPath, seg: usize, forward: bool) -> Option<(CubicBez, bool)> {
    let n = sp.segment_count();
    for k in 1..=n {
        let i = if forward { (seg + k < n || sp.closed).then(|| (seg + k) % n)? } else { (seg >= k || sp.closed).then(|| (seg + n - k) % n)? };
        let s = sp.segment(i);
        if start_direction(s) != Vec2::ZERO {
            return Some((s, k > 1));
        }
    }
    None
}

/// Is the join at anchor `anchor` smooth: marked smooth (with no segment of no length passed
/// over) with the directions `a` into it and `b` out of it less than 90° apart, or `a` and `b` the
/// same within [`SMOOTH_TURN`]?
fn smooth(sp: &SubPath, anchor: usize, skipped: bool, a: Vec2, b: Vec2) -> bool {
    let marked = !skipped && sp.anchors.get(anchor).is_some_and(|an| an.kind == AnchorKind::Smooth && an.has_in() && an.has_out());
    let (la, lb) = (a.hypot(), b.hypot());
    let ahead = la > 0.0 && lb > 0.0 && a.dot(b) > 0.0;
    ahead && (marked || a.cross(b).abs() <= SMOOTH_TURN.sin() * la * lb)
}

/// The direction a segment leaves its first anchor in: toward its first control point that is
/// not on that anchor; zero for a segment with no length.
fn start_direction(c: CubicBez) -> Vec2 {
    [c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0].into_iter().find(|v| v.hypot() > 0.0).unwrap_or_default()
}

/// The direction a segment reaches its last anchor in, as [`start_direction`] finds it.
fn end_direction(c: CubicBez) -> Vec2 {
    [c.p3 - c.p2, c.p3 - c.p1, c.p3 - c.p0].into_iter().find(|v| v.hypot() > 0.0).unwrap_or_default()
}

/// The center and radius of the circle of curvature of `c` at `t`.
fn osculating(c: CubicBez, t: f64) -> Result<(Point, f64), NoCircle> {
    let polygon = c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3);
    let d = c.deriv().eval(t).to_vec2();
    let dd = c.deriv().deriv().eval(t).to_vec2();
    let speed = d.hypot();
    if polygon <= 0.0 || !polygon.is_finite() {
        return Err(NoCircle::Cusp);
    }
    // All four control points on one line: a straight segment, whatever its handles.
    let (v1, v2, v3) = (c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0);
    if [v1.cross(v2), v1.cross(v3), v2.cross(v3)].iter().all(|a| a.abs() <= 1e-12 * polygon * polygon) {
        return Err(NoCircle::Straight);
    }
    if speed <= CUSP_SHARE * polygon {
        return Err(NoCircle::Cusp);
    }
    let cross = d.cross(dd);
    if cross.abs() <= STRAIGHT_SIN * speed * dd.hypot().max(polygon) {
        return Err(NoCircle::Straight);
    }
    // The center lies along the normal toward the side the curve turns to, `|d|² / cross` of the
    // normal `(-d.y, d.x)` away: its length is the radius, `|d|³ / |cross|`.
    let center = c.eval(t) + Vec2::new(-d.y, d.x) * (d.hypot2() / cross);
    let radius = speed.powi(3) / cross.abs();
    if !center.is_finite() || !radius.is_finite() {
        return Err(NoCircle::Straight);
    }
    Ok((center, radius))
}

/// The center and radius of the circular arc `c` draws, when it is the usual cubic of an arc of
/// 90° or less: its two ends are the same distance from the point where the lines square to its
/// handles meet, and each handle is `4/3 · tan(θ/4)` of the radius long for an arc of θ, pointing
/// along the arc.
pub fn circular_arc(c: CubicBez) -> Option<(Point, f64)> {
    let (d0, d3, chord) = (c.p1 - c.p0, c.p3 - c.p2, c.p3 - c.p0);
    let (l0, l3) = (d0.hypot(), d3.hypot());
    let usable = |v: f64| v.is_finite() && v != 0.0;
    if !usable(l0) || !usable(l3) || !usable(chord.hypot()) || d0.dot(chord) <= 0.0 || d3.dot(chord) <= 0.0 {
        return None;
    }
    // The ends' normals: p0 + s·n0 = p3 + u·n3.
    let (n0, n3) = (Vec2::new(-d0.y, d0.x), Vec2::new(-d3.y, d3.x));
    let den = n0.cross(n3);
    if den.abs() <= 1e-12 * l0 * l3 {
        return None;
    }
    let center = c.p0 + n0 * (chord.cross(n3) / den);
    let (r0, r3) = (c.p0.distance(center), c.p3.distance(center));
    let radius = (r0 + r3) / 2.0;
    if !center.is_finite() || !usable(radius) || (r0 - r3).abs() > RADIUS_TOL * radius {
        return None;
    }
    let (a, b) = (c.p0 - center, c.p3 - center);
    let turn = a.cross(b).abs().atan2(a.dot(b));
    if turn.is_nan() || turn <= 0.0 || turn > MAX_ARC {
        return None;
    }
    let handle = 4.0 / 3.0 * (turn / 4.0).tan() * radius;
    ((l0 - handle).abs() <= HANDLE_TOL * handle && (l3 - handle).abs() <= HANDLE_TOL * handle).then_some((center, radius))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anchor, shapes};
    use kurbo::Rect;

    /// Every segment of a circle drawn by [`shapes::ellipse`] is the usual cubic of a quarter
    /// turn, and the circle at any point of it is the circle itself: radius 100, center (150, 150).
    #[test]
    fn the_curvature_circle_of_a_circle_is_the_circle() {
        let pd = shapes::ellipse(Rect::new(50.0, 50.0, 250.0, 250.0));
        let sp = &pd.subpaths[0];
        for seg in 0..sp.segment_count() {
            for t in [0.0, 0.3, 0.5, 1.0] {
                let got = curvature_circle(sp.segment(seg), t, false).unwrap();
                assert!(got.nominal, "{seg} {t}");
                assert!((got.radius - 100.0).abs() < 1e-9 && got.center.distance(Point::new(150.0, 150.0)) < 1e-9, "{seg} {t}: {got:?}");
                assert!((got.at.distance(got.center) - 100.0).abs() < 1e-9, "the circle touches on its own edge");
            }
        }
    }

    /// A quarter circle of radius 10,000 whose handles are 0.09% longer than the usual cubic's still
    /// counts as the arc, and its middle lies a few points off the arc's circle: on the convex side
    /// the circle still touches the segment at that point, one radius from its center.
    #[test]
    fn the_convex_circle_of_a_near_arc_touches_the_segment() {
        let r = 10_000.0;
        let k = 4.0 / 3.0 * (std::f64::consts::FRAC_PI_8).tan() * r * 1.0009;
        let c = CubicBez::new(Point::new(r, 0.0), Point::new(r, k), Point::new(k, r), Point::new(0.0, r));
        assert!(circular_arc(c).is_some());
        let at = c.eval(0.5);
        assert!((at.distance(Point::ORIGIN) - r).abs() > 1.0, "the middle lies off the arc's circle");
        let got = curvature_circle(c, 0.5, true).unwrap();
        assert!(got.nominal);
        assert_eq!(got.at, at);
        assert!((got.center.distance(at) - got.radius).abs() < 1e-6, "{got:?}");
        assert!(got.center.distance(Point::ORIGIN) > got.radius, "the center is on the side away from the arc's");
    }

    /// The cubic's own radius of curvature on a quarter circle's segment is about 2.2% larger than
    /// the circle's at its ends and 0.6% larger at its middle: the nominal radius is the circle's.
    #[test]
    fn a_circle_segment_bends_near_its_radius() {
        let pd = shapes::ellipse(Rect::new(-100.0, -100.0, 100.0, 100.0));
        let seg = pd.subpaths[0].segment(0);
        for t in [0.0, 0.5, 1.0] {
            let (center, r) = osculating(seg, t).unwrap();
            assert!((r - 100.0).abs() < 2.5 && center.distance(Point::ORIGIN) < 2.5, "{t}: {center:?} {r}");
        }
    }

    /// With `arcs` off, a quarter circle's segment gives the cubic's own curvature, which is smaller
    /// than the circle's at its ends: the circle is not nominal and is larger than the arc's.
    #[test]
    fn without_arcs_a_circle_segment_gives_its_own_curvature() {
        let pd = shapes::ellipse(Rect::new(-100.0, -100.0, 100.0, 100.0));
        let sp = &pd.subpaths[0];
        let arc = curvature_circle_on_with(sp, 0, 0.0, false, true).unwrap();
        let own = curvature_circle_on_with(sp, 0, 0.0, false, false).unwrap();
        assert!(arc.nominal && !own.nominal, "{arc:?} {own:?}");
        assert!(own.radius > arc.radius && own.center != arc.center, "{arc:?} {own:?}");
        assert_eq!(own.at, sp.segment(0).eval(0.0));
    }

    /// On the parabola y = x² / 200, the curvature at its vertex is 1/100: drawn as a cubic from
    /// (-100, 50) to (100, 50), the circle at the middle has radius 100 and center (0, 100), above
    /// the curve on the side it bends toward; Shift's other side puts the center at (0, -100).
    #[test]
    fn the_curvature_circle_at_a_parabola_vertex() {
        // The quadratic (-100, 50), (0, -50), (100, 50) raised to a cubic.
        let (p0, q, p3) = (Point::new(-100.0, 50.0), Point::new(0.0, -50.0), Point::new(100.0, 50.0));
        let c = CubicBez::new(p0, p0 + (q - p0) * (2.0 / 3.0), p3 + (q - p3) * (2.0 / 3.0), p3);
        assert_eq!(circular_arc(c), None);
        let got = curvature_circle(c, 0.5, false).unwrap();
        assert!(!got.nominal && got.at.distance(Point::ORIGIN) < 1e-9, "{got:?}");
        assert!((got.radius - 100.0).abs() < 1e-9 && got.center.distance(Point::new(0.0, 100.0)) < 1e-9, "{got:?}");
        let other = curvature_circle(c, 0.5, true).unwrap();
        assert!((other.radius - 100.0).abs() < 1e-9 && other.center.distance(Point::new(0.0, -100.0)) < 1e-9, "{other:?}");
    }

    /// A straight segment, with its handles on its line or on its anchors, and the
    /// turning point of an S curve have no curvature circle.
    #[test]
    fn a_straight_segment_has_no_circle() {
        let (a, b) = (Point::new(10.0, 20.0), Point::new(130.0, 70.0));
        let on_line = CubicBez::new(a, a.lerp(b, 0.3), a.lerp(b, 0.7), b);
        assert_eq!(curvature_circle(on_line, 0.4, false), Err(NoCircle::Straight));
        // A line's handles sit on its anchors: straight at its ends too.
        for t in [0.0, 0.5, 1.0] {
            assert_eq!(curvature_circle(CubicBez::new(a, a, b, b), t, false), Err(NoCircle::Straight), "{t}");
        }
        let s = CubicBez::new(Point::new(0.0, 0.0), Point::new(50.0, 50.0), Point::new(50.0, -50.0), Point::new(100.0, 0.0));
        assert_eq!(curvature_circle(s, 0.5, false), Err(NoCircle::Straight));
        assert!(curvature_circle(s, 0.25, false).is_ok());
    }

    /// A cusp, a handle drawn back onto its anchor at that end, a segment with no length, points
    /// that are not finite and a `t` outside 0 to 1 have no curvature circle.
    #[test]
    fn a_cusp_has_no_circle() {
        let cusp = CubicBez::new(Point::new(0.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0), Point::new(100.0, 0.0));
        assert_eq!(curvature_circle(cusp, 0.5, false), Err(NoCircle::Cusp));
        let p = Point::new(5.0, 5.0);
        let retracted = CubicBez::new(Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(60.0, 40.0), Point::new(100.0, 0.0));
        assert_eq!(curvature_circle(retracted, 0.0, false), Err(NoCircle::Cusp));
        assert_eq!(curvature_circle(CubicBez::new(p, p, p, p), 0.5, false), Err(NoCircle::Cusp));
        let far = CubicBez::new(Point::new(f64::NAN, 0.0), p, p, Point::new(9.0, 0.0));
        assert_eq!(curvature_circle(far, 0.5, false), Err(NoCircle::Cusp));
        for t in [-0.1, 1.5, f64::NAN] {
            assert_eq!(curvature_circle(cusp, t, false), Err(NoCircle::Cusp), "{t}");
        }
    }

    /// At an anchor where a curve meets a line at a corner, the path has no single direction, from
    /// either segment; at a smooth anchor of a circle, and at the open ends of a path, the circle is
    /// drawn. A segment the subpath lacks has none.
    #[test]
    fn a_corner_anchor_has_no_circle() {
        let bp = kurbo::BezPath::from_svg("M0 0 C50 -50 100 -50 150 0 L300 0").unwrap();
        let sp = &crate::PathData::from_bezpath(&bp).subpaths[0];
        assert_eq!(curvature_circle_on(sp, 0, 1.0, false), Err(NoCircle::Corner));
        assert_eq!(curvature_circle_on(sp, 1, 0.0, false), Err(NoCircle::Corner));
        assert_eq!(curvature_circle_on(sp, 1, 0.5, false), Err(NoCircle::Straight));
        assert!(curvature_circle_on(sp, 0, 0.0, false).is_ok() && curvature_circle_on(sp, 0, 0.5, false).is_ok());
        assert_eq!(curvature_circle_on(sp, 2, 0.5, false), Err(NoCircle::Cusp));
        let circle = &shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)).subpaths[0];
        for seg in 0..4 {
            for t in [0.0, 1.0] {
                assert!(curvature_circle_on(circle, seg, t, false).is_ok_and(|c| c.nominal), "{seg} {t}");
            }
        }
        // A closed triangle of curves: the anchor where the last segment meets the first is a corner.
        let bp = kurbo::BezPath::from_svg("M0 0 C10 -10 90 -10 100 0 C110 10 60 90 50 100 C40 90 -10 30 0 0 Z").unwrap();
        let tri = &crate::PathData::from_bezpath(&bp).subpaths[0];
        assert!(tri.closed);
        assert_eq!(curvature_circle_on(tri, 0, 0.0, false), Err(NoCircle::Corner));
    }

    /// A closed subpath whose only segment with length loops back to its start meets itself at its
    /// point: a teardrop's tip is a corner, from both ends of the loop.
    #[test]
    fn a_loop_that_meets_itself_at_an_angle_has_no_circle_there() {
        let a = Point::new(0.0, 0.0);
        let tip = |out: Point, back: Point| SubPath::new(vec![Anchor::with_handles(a, a, out), Anchor::with_handles(a, back, a)], true);
        let drop = tip(Point::new(100.0, -100.0), Point::new(100.0, 100.0));
        assert_eq!(drop.segment_count(), 2);
        assert_eq!(curvature_circle_on(&drop, 0, 0.0, false), Err(NoCircle::Corner));
        assert_eq!(curvature_circle_on(&drop, 0, 1.0, false), Err(NoCircle::Corner));
        assert!(curvature_circle_on(&drop, 0, 0.5, false).is_ok());
    }

    /// A segment with no length between a curve and a line, or a closed subpath whose last anchor
    /// repeats its first, still leaves a corner there: the segments with no length are passed over.
    #[test]
    fn a_corner_behind_a_segment_with_no_length_has_no_circle() {
        let (a, b, c) = (Point::new(0.0, 0.0), Point::new(150.0, 0.0), Point::new(300.0, 0.0));
        let curve_in = Anchor::with_handles(b, Point::new(100.0, -50.0), b);
        let open = SubPath::new(vec![Anchor::with_handles(a, a, Point::new(50.0, -50.0)), curve_in, Anchor::corner(b), Anchor::corner(c)], false);
        assert_eq!(curvature_circle_on(&open, 0, 1.0, false), Err(NoCircle::Corner));
        assert_eq!(curvature_circle_on(&open, 2, 0.0, false), Err(NoCircle::Corner), "from the line's side too");
        assert_eq!(curvature_circle_on(&open, 2, 0.5, false), Err(NoCircle::Straight));
        // A closed triangle of curves drawn back to its first anchor before it closes.
        let tri = SubPath::new(
            vec![
                Anchor::with_handles(a, a, Point::new(10.0, -10.0)),
                Anchor::with_handles(Point::new(100.0, 0.0), Point::new(90.0, -10.0), Point::new(110.0, 10.0)),
                Anchor::with_handles(Point::new(50.0, 100.0), Point::new(60.0, 90.0), Point::new(40.0, 90.0)),
                Anchor::with_handles(a, Point::new(-10.0, 30.0), a),
            ],
            true,
        );
        assert_eq!(tri.segment_count(), 4);
        assert_eq!(curvature_circle_on(&tri, 0, 0.0, false), Err(NoCircle::Corner));
        assert_eq!(curvature_circle_on(&tri, 2, 1.0, false), Err(NoCircle::Corner));
    }

    /// An anchor whose handles turn by a thousandth of a radian, as rounded coordinates leave them,
    /// is smooth; one marked smooth is smooth unless its handles turn by 90° or more; a turn of 2°
    /// is a corner.
    #[test]
    fn a_nearly_smooth_or_marked_smooth_anchor_has_a_circle() {
        let mid = |out_x: f64, kind: AnchorKind| {
            let p = Point::new(100.0, 0.0);
            let mut an = Anchor::with_handles(p, Point::new(100.0, -50.0), Point::new(out_x, 50.0));
            an.kind = kind;
            let ends = |q: Point, h: Point| Anchor::with_handles(q, h, h);
            SubPath::new(vec![ends(Point::new(0.0, 0.0), Point::new(0.0, -50.0)), an, ends(Point::new(0.0, 100.0), Point::new(0.0, 150.0))], false)
        };
        let rounded = mid(100.05, AnchorKind::Corner);
        assert!(curvature_circle_on(&rounded, 0, 1.0, false).is_ok() && curvature_circle_on(&rounded, 1, 0.0, false).is_ok());
        let turned = 50.0 * 2f64.to_radians().tan();
        assert_eq!(curvature_circle_on(&mid(100.0 - turned, AnchorKind::Corner), 0, 1.0, false), Err(NoCircle::Corner));
        assert!(curvature_circle_on(&mid(100.0 - turned, AnchorKind::Smooth), 0, 1.0, false).is_ok());
        // Marked smooth, with the out handle pointing back along the in handle.
        let mut back = mid(100.0, AnchorKind::Smooth);
        back.anchors.get_mut(1).unwrap().h_out = Point::new(100.0, -40.0);
        assert_eq!(curvature_circle_on(&back, 0, 1.0, false), Err(NoCircle::Corner));
        assert_eq!(curvature_circle_on(&back, 1, 0.0, false), Err(NoCircle::Corner));
        // Marked smooth, with the out handle square to the in handle: a turn of 90°.
        let mut square = mid(100.0, AnchorKind::Smooth);
        square.anchors.get_mut(1).unwrap().h_out = Point::new(150.0, 0.0);
        assert_eq!(curvature_circle_on(&square, 0, 1.0, false), Err(NoCircle::Corner));
    }

    /// An arc's cubic is taken only up to a quarter turn, with both ends on one circle and handles
    /// of the arc's length: a third of a circle, a circle squashed into an ellipse and handles 1%
    /// too long give the cubic's own curvature.
    #[test]
    fn only_a_true_arc_of_a_quarter_turn_or_less_is_nominal() {
        let arc = |deg: f64, r: f64, stretch: f64, handle: f64| {
            let th = deg.to_radians();
            let k = 4.0 / 3.0 * (th / 4.0).tan() * r * handle;
            let (p0, p3) = (Point::new(r, 0.0), Point::new(r * th.cos() * stretch, r * th.sin()));
            CubicBez::new(p0, p0 + Vec2::new(0.0, k), p3 + Vec2::new(th.sin(), -th.cos()) * k, p3)
        };
        let (center, r) = circular_arc(arc(60.0, 50.0, 1.0, 1.0)).unwrap();
        assert!(center.distance(Point::ORIGIN) < 1e-9 && (r - 50.0).abs() < 1e-9);
        assert!(circular_arc(arc(90.0, 50.0, 1.0, 1.0)).is_some());
        assert_eq!(circular_arc(arc(120.0, 50.0, 1.0, 1.0)), None);
        assert_eq!(circular_arc(arc(60.0, 50.0, 1.1, 1.0)), None);
        assert_eq!(circular_arc(arc(60.0, 50.0, 1.0, 1.01)), None);
        // On the convex side, the circle touches the arc's circle from outside.
        let c = curvature_circle(arc(60.0, 50.0, 1.0, 1.0), 0.0, true).unwrap();
        assert!(c.nominal && c.center.distance(Point::new(100.0, 0.0)) < 1e-9 && (c.radius - 50.0).abs() < 1e-9, "{c:?}");
    }
}
