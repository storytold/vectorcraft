//! Shape generators matching Illustrator's anchor layouts (so later edits behave the same).

use std::f64::consts::{PI, TAU};

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};

/// Circle-approximation handle ratio for a quarter arc.
pub const KAPPA: f64 = 0.552_284_749_830_793_6;

/// Rectangle with 4 corner anchors, clockwise from the top-left.
pub fn rectangle(r: Rect) -> PathData {
    let r = r.abs();
    PathData::single(SubPath::polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true))
}

/// Rounded rectangle (8 anchors). The radius is clamped to half the shorter side.
pub fn rounded_rectangle(r: Rect, radius: f64) -> PathData {
    rectangle_with_corners(r, [radius; 4], [CornerKind::Round; 4])
}

/// How a rectangle's corner is cut (Live Corners).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CornerKind {
    /// A quarter circle, bulging outwards.
    #[default]
    Round,
    /// A quarter circle cut into the shape, centred on the corner.
    InvertedRound,
    /// A straight bevel.
    Chamfer,
}

impl CornerKind {
    pub const ALL: [CornerKind; 3] = [CornerKind::Round, CornerKind::InvertedRound, CornerKind::Chamfer];

    /// The kind after this one (Alt-clicking a corner widget cycles through them).
    pub fn next(self) -> Self {
        match self {
            CornerKind::Round => CornerKind::InvertedRound,
            CornerKind::InvertedRound => CornerKind::Chamfer,
            CornerKind::Chamfer => CornerKind::Round,
        }
    }
}

/// The largest corner radius a `w` × `h` rectangle draws: half its shorter side.
pub fn max_corner_radius(w: f64, h: f64) -> f64 {
    w.abs().min(h.abs()) / 2.0
}

/// The radii the corners of a `w` × `h` rectangle (top-left, top-right, bottom-right,
/// bottom-left) are drawn with: each its own, no larger than leaves room for its neighbours'
/// cuts ([`crate::corners::fitted_radii`]): half the shorter side when they are all as large, up
/// to a whole side when its neighbours are square (#2332); 0 when negative or not a number.
pub fn fitted_corner_radii(w: f64, h: f64, radii: [f64; 4]) -> [f64; 4] {
    let corners = crate::corners::path_corners(&rectangle(Rect::new(0.0, 0.0, w, h)));
    let mut out = [0.0; 4];
    for (c, r) in corners.iter().zip(crate::corners::fitted_radii(&corners, &radii)) {
        if let Some(o) = out.get_mut(c.index) {
            *o = r;
        }
    }
    out
}

/// Rectangle whose corners (top-left, top-right, bottom-right, bottom-left, y down) each have
/// their own radius and kind, every radius no larger than [`fitted_corner_radii`] leaves. A corner without a
/// radius is one anchor, the others two (where the cut meets each side). Clockwise from the
/// top-left corner, its anchor on the top side first. Cut as any path's corners are
/// ([`crate::corners::cut_corners`]).
pub fn rectangle_with_corners(r: Rect, radii: [f64; 4], kinds: [CornerKind; 4]) -> PathData {
    crate::corners::cut_corners(&rectangle(r), &radii, &kinds).0
}

/// Ellipse inscribed in `r` with 4 smooth anchors (left, top, right, bottom).
pub fn ellipse(r: Rect) -> PathData {
    let r = r.abs();
    let c = r.center();
    let rx = r.width() / 2.0;
    let ry = r.height() / 2.0;
    let kx = rx * KAPPA;
    let ky = ry * KAPPA;
    let s = |p: Point, hin: Vec2| Anchor { p, h_in: p + hin, h_out: p - hin, kind: AnchorKind::Smooth };
    let anchors = vec![
        s(Point::new(c.x - rx, c.y), Vec2::new(0.0, ky)),
        s(Point::new(c.x, c.y - ry), Vec2::new(-kx, 0.0)),
        s(Point::new(c.x + rx, c.y), Vec2::new(0.0, -ky)),
        s(Point::new(c.x, c.y + ry), Vec2::new(kx, 0.0)),
    ];
    PathData::single(SubPath::new(anchors, true))
}

/// The ellipse inscribed in `r` cut as a pie: from `start` to `end` degrees, measured
/// counterclockwise on screen from 3 o'clock (as Illustrator's Pie Start and End Angle are), then
/// back through the centre. A pie spanning the whole turn (or nothing) is the whole [`ellipse`].
/// The arc is one cubic per quarter turn at most: the centre, then the arc's anchors.
pub fn ellipse_pie(r: Rect, start: f64, end: f64) -> PathData {
    let span = (end - start).rem_euclid(360.0);
    if !(start.is_finite() && end.is_finite()) || span < 1e-9 || 360.0 - span < 1e-9 {
        return ellipse(r);
    }
    let r = r.abs();
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    // Screen y runs down: counterclockwise on screen is a falling y.
    let at = |deg: f64| {
        let t = deg.to_radians();
        Point::new(c.x + rx * t.cos(), c.y - ry * t.sin())
    };
    let tangent = |deg: f64| {
        let t = deg.to_radians();
        Vec2::new(-rx * t.sin(), -ry * t.cos())
    };
    let pieces = if span <= 90.0 {
        1
    } else if span <= 180.0 {
        2
    } else if span <= 270.0 {
        3
    } else {
        4
    };
    let step = span / f64::from(pieces);
    let k = 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
    let mut anchors = vec![Anchor::corner(c)];
    for i in 0..=pieces {
        let a = start + step * f64::from(i);
        let (p, d) = (at(a), tangent(a) * k);
        let h_in = if i == 0 { p } else { p - d };
        let h_out = if i == pieces { p } else { p + d };
        let kind = if i == 0 || i == pieces { AnchorKind::Corner } else { AnchorKind::Smooth };
        anchors.push(Anchor { p, h_in, h_out, kind });
    }
    PathData::single(SubPath::new(anchors, true))
}

/// Regular polygon with `sides` (≥ 3), first vertex straight up from the centre.
pub fn polygon(center: Point, radius: f64, sides: u32, rotation_deg: f64) -> PathData {
    let n = sides.max(3);
    let rot = rotation_deg.to_radians();
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = -PI / 2.0 + rot + TAU * i as f64 / n as f64;
            Point::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Star with `points` tips (≥ 2), outer radius `r1`, inner radius `r2`, first tip up.
pub fn star(center: Point, r1: f64, r2: f64, points: u32, rotation_deg: f64) -> PathData {
    let n = points.max(2) * 2;
    let rot = rotation_deg.to_radians();
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let r = if i % 2 == 0 { r1 } else { r2 };
            let a = -PI / 2.0 + rot + TAU * i as f64 / n as f64;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Inner radius of a "straight-armed" star (Illustrator's Alt-drag behaviour): shoulders aligned.
pub fn straight_star_inner_radius(r1: f64, points: u32) -> f64 {
    let n = points.max(3) as f64;
    r1 * (PI / n * 2.0).cos() / (PI / n).cos()
}

/// Open straight line.
pub fn line(a: Point, b: Point) -> PathData {
    PathData::single(SubPath::polyline(&[a, b], false))
}

/// Bounding box of an arc segment when reference point `reference` (0…8) is placed on `anchor`.
pub fn arc_bbox(anchor: Point, reference: usize, length_x: f64, length_y: f64) -> Rect {
    let w = length_x.abs().max(1e-9);
    let h = length_y.abs().max(1e-9);
    let rp = crate::reference_point(Rect::new(0.0, 0.0, w, h), reference.min(8));
    Rect::new(anchor.x - rp.x, anchor.y - rp.y, anchor.x - rp.x + w, anchor.y - rp.y + h)
}

/// Arc segment from the Arc tool dialog: lengths, reference point, base axis, slope and open/closed.
pub fn arc_segment(anchor: Point, reference: usize, length_x: f64, length_y: f64, base_along_x: bool, slope: f64, closed: bool) -> PathData {
    let r = arc_bbox(anchor, reference, length_x, length_y).abs();
    let (a, b) = if base_along_x { (Point::new(r.x0, r.y0), Point::new(r.x1, r.y1)) } else { (Point::new(r.x0, r.y1), Point::new(r.x1, r.y0)) };
    arc(a, b, slope, closed, base_along_x)
}

/// Quarter-ellipse arc from `a` to `b` (Arc tool default: open). `slope` is −100…100
/// (concave…convex); 0 is a straight chord. A convex arc bulges toward the corner `(b.x, a.y)`,
/// and a closed arc closes through the corner `(a.x, b.y)`, so a closed arc at slope 100 is the
/// quarter-ellipse sector. The shape does not depend on `_base_along_x`: for a Y-base arc,
/// [`arc_segment`] passes ends on the box mirrored top to bottom.
pub fn arc(a: Point, b: Point, slope: f64, closed: bool, _base_along_x: bool) -> PathData {
    let t = slope.clamp(-100.0, 100.0) / 100.0;
    let corner = if t >= 0.0 { Point::new(b.x, a.y) } else { Point::new(a.x, b.y) };
    let k = KAPPA * t.abs();
    let h1 = if k == 0.0 { a } else { a + (corner - a) * k };
    let h2 = if k == 0.0 { b } else { b + (corner - b) * k };
    let mut anchors =
        vec![Anchor { p: a, h_in: a, h_out: h1, kind: AnchorKind::Corner }, Anchor { p: b, h_in: h2, h_out: b, kind: AnchorKind::Corner }];
    if closed {
        anchors.push(Anchor::corner(Point::new(a.x, b.y)));
    }
    PathData::single(SubPath::new(anchors, closed))
}

/// Archimedean-style spiral approximating Illustrator's Spiral tool (decay-based, `segments` quarter turns).
pub fn spiral(center: Point, radius: f64, decay_percent: f64, segments: u32, clockwise: bool) -> PathData {
    let decay = (decay_percent / 100.0).clamp(0.05, 0.9999);
    let dir = if clockwise { 1.0 } else { -1.0 };
    let mut anchors = Vec::new();
    let mut r = radius;
    for i in 0..=segments {
        let a = dir * (i as f64) * PI / 2.0;
        let p = Point::new(center.x + r * a.cos(), center.y + r * a.sin());
        // Tangent direction for a quarter arc of radius r.
        let t = Vec2::new(-a.sin(), a.cos()) * dir;
        let r_prev = r / decay;
        let r_next = r * decay;
        let h_in = p - t * (KAPPA * (r + r_prev) / 2.0);
        let h_out = p + t * (KAPPA * (r + r_next) / 2.0);
        anchors.push(Anchor { p, h_in: if i == 0 { p } else { h_in }, h_out: if i == segments { p } else { h_out }, kind: AnchorKind::Smooth });
        r *= decay;
    }
    // Illustrator spirals wind inward from the outer point; reverse so the path starts at the centre end.
    let mut sp = SubPath::new(anchors, false);
    sp.reverse();
    PathData::single(sp)
}

/// The largest grid divider skew, in percent either way.
pub const GRID_SKEW_MAX: f64 = 500.0;

/// Where a divider at fraction `t` (0..1) of a grid's extent lands under `skew` percent: evenly
/// spaced at 0, crowded toward 1 by a positive skew and toward 0 by a negative one (`t` raised to
/// 1 + |skew| / 100 from the far end).
fn skewed(t: f64, skew: f64) -> f64 {
    // NaN reads as no skew.
    let k = 1.0 + skew.abs().min(GRID_SKEW_MAX) / 100.0;
    if skew > 0.0 {
        1.0 - (1.0 - t).powf(k)
    } else if skew < 0.0 {
        t.powf(k)
    } else {
        t
    }
}

/// Rectangular grid, bottom object first: the outside as one rectangle (`frame`) or as its four
/// sides (top, bottom, left, right), then the horizontal and the vertical dividers as open lines.
/// A positive `h_skew` crowds the horizontal dividers toward the top and a positive `v_skew` the
/// vertical ones toward the right (percent, see [`GRID_SKEW_MAX`]).
pub fn rectangular_grid(r: Rect, (h_dividers, h_skew): (u32, f64), (v_dividers, v_skew): (u32, f64), frame: bool) -> Vec<PathData> {
    let r = r.abs();
    let (tl, tr, br, bl) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
    let mut out = if frame { vec![rectangle(r)] } else { vec![line(tl, tr), line(bl, br), line(tl, bl), line(tr, br)] };
    for i in 1..=h_dividers {
        // From the top down: toward the top is toward 0.
        let y = r.y0 + r.height() * skewed(i as f64 / (h_dividers + 1) as f64, -h_skew);
        out.push(line(Point::new(r.x0, y), Point::new(r.x1, y)));
    }
    for i in 1..=v_dividers {
        let x = r.x0 + r.width() * skewed(i as f64 / (v_dividers + 1) as f64, v_skew);
        out.push(line(Point::new(x, r.y0), Point::new(x, r.y1)));
    }
    out
}

/// Polar grid, bottom object first: the concentric ellipses from the outside in, then the radial
/// dividers from the top one clockwise. A positive `c_skew` crowds the ellipses toward the outside
/// and a positive `r_skew` the radial dividers clockwise of the top one (percent, see
/// [`GRID_SKEW_MAX`]). With `compound`, the ellipses pair up from the outside in into rings (two
/// subpaths, the inner one reversed so that it is a hole under either fill rule); an odd one out
/// is the innermost ellipse alone.
pub fn polar_grid(r: Rect, (concentric, c_skew): (u32, f64), (radial, r_skew): (u32, f64), compound: bool) -> Vec<PathData> {
    let r = r.abs();
    let c = r.center();
    let mut out: Vec<PathData> = Vec::new();
    for i in (1..=concentric + 1).rev() {
        let f = skewed(i as f64 / (concentric + 1) as f64, c_skew);
        let mut e = ellipse(Rect::from_center_size(c, (r.width() * f, r.height() * f)));
        // Every second ellipse from the outside is the hole of the one before it.
        match out.last_mut().filter(|_| compound && (concentric + 1 - i) % 2 == 1) {
            Some(ring) => {
                e.reverse();
                ring.subpaths.append(&mut e.subpaths);
            }
            None => out.push(e),
        }
    }
    // No dividers at 0 (the loop never divides by it).
    for i in 0..radial {
        // The turn runs clockwise on screen: clockwise of the top is toward its start.
        let a = -PI / 2.0 + TAU * skewed(i as f64 / radial as f64, -r_skew);
        out.push(line(c, Point::new(c.x + r.width() / 2.0 * a.cos(), c.y + r.height() / 2.0 * a.sin())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corner (index into `radii`) of each anchor of [`rectangle_with_corners`].
    fn corner_sources(r: Rect, radii: [f64; 4]) -> Vec<usize> {
        crate::corners::cut_corners(&rectangle(r), &radii, &[]).1.concat()
    }

    /// A pie is the arc from its start to its end angle, counterclockwise on screen, closed through
    /// the centre; a whole turn (or none) is the ellipse.
    #[test]
    fn a_pie_is_its_arc_closed_through_the_centre() {
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(ellipse_pie(r, 0.0, 360.0), ellipse(r));
        assert_eq!(ellipse_pie(r, 90.0, 90.0), ellipse(r));
        // A quarter, from 3 o'clock up to 12: the centre, then the arc's two ends.
        let q = ellipse_pie(r, 0.0, 90.0);
        let a: Vec<Point> = q.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(a.len(), 3);
        assert!(a[0].distance(Point::new(100.0, 50.0)) < 1e-9 && a[1].distance(Point::new(200.0, 50.0)) < 1e-9);
        assert!(a[2].distance(Point::new(100.0, 0.0)) < 1e-9, "counterclockwise on screen: up");
        assert_eq!(q.bounds().map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())), Some((100.0, 0.0, 200.0, 50.0)));
        // Three quarters, from 12 round to 3 o'clock: four pieces at most, the whole box covered.
        let big = ellipse_pie(r, 90.0, 0.0);
        assert!(big.subpaths[0].anchors.len() <= 6 && big.subpaths[0].closed);
        assert_eq!(big.bounds().map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())), Some((0.0, 0.0, 200.0, 100.0)));
        assert!(ellipse_pie(r, f64::NAN, 10.0).anchor_count() == 4, "not a number: the whole ellipse");
    }

    #[test]
    fn rect_is_4_corners_clockwise() {
        let p = rectangle(Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(p.anchor_count(), 4);
        assert!(p.subpaths[0].area() > 0.0);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn rect_normalizes_negative() {
        let p = rectangle(Rect::new(10.0, 10.0, 0.0, 0.0));
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn ellipse_bounds_and_area() {
        let p = ellipse(Rect::new(0.0, 0.0, 100.0, 60.0));
        let b = p.bounds().unwrap();
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 60.0).abs() < 1e-6);
        let area = p.subpaths[0].area().abs();
        assert!((area - PI * 50.0 * 30.0).abs() / area < 0.001);
    }

    #[test]
    fn rounded_rect_clamps() {
        let p = rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 100.0);
        assert_eq!(p.anchor_count(), 8);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 20.0, 10.0)));
        assert_eq!(rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 0.0).anchor_count(), 4);
    }

    #[test]
    fn uniform_corners_keep_the_rounded_rectangle_layout() {
        // The 8 anchors a rounded rectangle always had, from the top side's first one.
        let (r, rad) = (Rect::new(10.0, 20.0, 110.0, 70.0), 7.5);
        let (x0, y0, x1, y1, k) = (10.0, 20.0, 110.0, 70.0, rad * KAPPA);
        let a =
            |p: (f64, f64), hin: (f64, f64), hout: (f64, f64)| Anchor { p: p.into(), h_in: hin.into(), h_out: hout.into(), kind: AnchorKind::Corner };
        let want = vec![
            a((x0 + rad, y0), (x0 + rad - k, y0), (x0 + rad, y0)),
            a((x1 - rad, y0), (x1 - rad, y0), (x1 - rad + k, y0)),
            a((x1, y0 + rad), (x1, y0 + rad - k), (x1, y0 + rad)),
            a((x1, y1 - rad), (x1, y1 - rad), (x1, y1 - rad + k)),
            a((x1 - rad, y1), (x1 - rad + k, y1), (x1 - rad, y1)),
            a((x0 + rad, y1), (x0 + rad, y1), (x0 + rad - k, y1)),
            a((x0, y1 - rad), (x0, y1 - rad + k), (x0, y1 - rad)),
            a((x0, y0 + rad), (x0, y0 + rad), (x0, y0 + rad - k)),
        ];
        assert_eq!(rounded_rectangle(r, rad).subpaths[0].anchors, want);
        assert_eq!(corner_sources(r, [rad; 4]), [0, 1, 1, 2, 2, 3, 3, 0]);
        assert_eq!(rounded_rectangle(r, 0.0), rectangle(r));
    }

    #[test]
    fn each_corner_takes_its_own_radius_and_kind() {
        let r = Rect::new(0.0, 0.0, 100.0, 60.0);
        // Only the top-right corner rounded: 5 anchors, the corner's two on its sides.
        let radii = [0.0, 10.0, 0.0, 0.0];
        let p = rectangle_with_corners(r, radii, [CornerKind::Round; 4]);
        let pts: Vec<Point> = p.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(pts, [Point::new(0.0, 0.0), Point::new(90.0, 0.0), Point::new(100.0, 10.0), Point::new(100.0, 60.0), Point::new(0.0, 60.0)]);
        assert_eq!(corner_sources(r, radii), [0, 1, 1, 2, 3]);
        let quarter = PI * 25.0;
        assert!((p.subpaths[0].area() - (6000.0 - 100.0 + quarter)).abs() < 0.1, "a quarter circle off one corner");
        // The top-left corner rounded: its top-side anchor first, its left-side one last.
        assert_eq!(corner_sources(r, [10.0, 0.0, 0.0, 200.0]), [0, 1, 2, 3, 3, 0]);
        // Inverted round cuts a quarter circle out; a chamfer cuts a triangle.
        let inverted = rectangle_with_corners(r, radii, [CornerKind::InvertedRound; 4]);
        assert!((inverted.subpaths[0].area() - (6000.0 - quarter)).abs() < 0.1);
        let chamfer = rectangle_with_corners(r, radii, [CornerKind::Chamfer; 4]);
        assert!((chamfer.subpaths[0].area() - (6000.0 - 50.0)).abs() < 1e-9);
        assert!(chamfer.subpaths[0].anchors.iter().all(|a| !a.has_in() && !a.has_out()), "straight");
        // Radii clamp to what their neighbours leave, the whole shorter side when they are
        // square (#2332): the cut reaches the bottom-left corner. Negative and NaN radii are none.
        let p = rectangle_with_corners(r, [500.0, -5.0, f64::NAN, 0.0], [CornerKind::Round; 4]);
        assert_eq!(p.subpaths[0].anchors[0].p, Point::new(60.0, 0.0));
        assert_eq!(p.subpaths[0].anchors.last().map(|a| a.p), Some(Point::new(0.0, 60.0)));
        assert_eq!(p.anchor_count(), 5);
        assert_eq!(CornerKind::ALL.map(CornerKind::next), [CornerKind::InvertedRound, CornerKind::Chamfer, CornerKind::Round]);
    }

    /// #442: on a rectangle that isn't square, every corner is a circular arc of the same radius,
    /// and radii past the limit stop at half the shorter side, whichever side a corner is on.
    #[test]
    fn corners_of_a_non_square_rectangle_are_alike() {
        let r = Rect::new(0.0, 0.0, 200.0, 80.0);
        assert_eq!(max_corner_radius(-200.0, 80.0), 40.0);
        for (radius, drawn) in [(30.0, 30.0), (60.0, 40.0)] {
            for kinds in [[CornerKind::Round; 4], [CornerKind::Chamfer, CornerKind::InvertedRound, CornerKind::Round, CornerKind::Chamfer]] {
                let a: Vec<Point> = rectangle_with_corners(r, [radius; 4], kinds).subpaths[0].anchors.iter().map(|a| a.p).collect();
                for (i, j) in [(7, 0), (1, 2), (3, 4), (5, 6)] {
                    let (x, y) = ((a[j].x - a[i].x).abs(), (a[j].y - a[i].y).abs());
                    assert!((x - drawn).abs() < 1e-9 && (y - drawn).abs() < 1e-9, "radius {radius}: a corner spans {x} × {y}");
                }
            }
        }
    }

    #[test]
    fn polygon_first_vertex_up() {
        let p = polygon(Point::new(0.0, 0.0), 10.0, 6, 0.0);
        assert_eq!(p.anchor_count(), 6);
        let a = p.subpaths[0].anchors[0].p;
        assert!(a.x.abs() < 1e-9 && (a.y + 10.0).abs() < 1e-9);
    }

    #[test]
    fn star_alternates_radii() {
        let p = star(Point::ZERO, 10.0, 5.0, 5, 0.0);
        assert_eq!(p.anchor_count(), 10);
        let d: Vec<f64> = p.subpaths[0].anchors.iter().map(|a| a.p.to_vec2().hypot()).collect();
        assert!((d[0] - 10.0).abs() < 1e-9 && (d[1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn straight_star_five_points() {
        let r2 = straight_star_inner_radius(100.0, 5);
        assert!((r2 - 38.196_601).abs() < 1e-3);
    }

    #[test]
    fn spiral_has_segments() {
        let p = spiral(Point::ZERO, 100.0, 80.0, 10, true);
        assert_eq!(p.anchor_count(), 11);
        assert!(!p.subpaths[0].closed);
    }

    #[test]
    fn grids() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(rectangular_grid(r, (5, 0.0), (5, 0.0), true).len(), 11);
        assert_eq!(polar_grid(r, (5, 0.0), (5, 0.0), false).len(), 11);
        // #1008: no radial dividers is none, only the outer ellipse without concentric ones.
        for (radial, len) in [(0, 1), (1, 2), (2, 3)] {
            assert_eq!(polar_grid(r, (0, 0.0), (radial, 0.0), false).len(), len, "{radial}");
        }
    }

    fn close(got: &[f64], want: &[f64]) -> bool {
        got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    /// The first anchor of each of `paths`.
    fn starts(paths: &[PathData]) -> Vec<Point> {
        paths.iter().map(|p| p.subpaths[0].anchors[0].p).collect()
    }

    /// #1377: the frame is one rectangle or its four sides, at the bottom; a skew moves the
    /// dividers toward the top / right (positive) or the bottom / left, in order and inside the
    /// grid, and none leaves them evenly spaced.
    #[test]
    fn a_rectangular_grid_frames_and_skews_its_dividers() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let framed = rectangular_grid(r, (3, 0.0), (1, 0.0), true);
        assert!(framed[0].is_closed() && framed[1..].iter().all(|p| !p.is_closed()));
        assert_eq!(starts(&framed[1..]), [Point::new(0.0, 25.0), Point::new(0.0, 50.0), Point::new(0.0, 75.0), Point::new(50.0, 0.0)]);
        let sides = rectangular_grid(r, (3, 0.0), (1, 0.0), false);
        assert_eq!(sides.len(), 8);
        assert!(sides.iter().all(|p| !p.is_closed() && p.anchor_count() == 2));
        assert_eq!(sides[4..], framed[1..]);
        for (skew, toward_start) in [(100.0, true), (-100.0, false), (GRID_SKEW_MAX, true), (-1e9, false), (f64::INFINITY, true)] {
            let g = rectangular_grid(r, (3, skew), (3, -skew), true);
            let ys: Vec<f64> = starts(&g[1..4]).iter().map(|p| p.y).collect();
            let xs: Vec<f64> = starts(&g[4..]).iter().map(|p| p.x).collect();
            for v in [&ys, &xs] {
                assert!(v.windows(2).all(|w| w[0] < w[1]) && v[0] > 0.0 && v[2] < 100.0, "{skew}: {v:?}");
                // Toward the top (y) and the left (x) is toward 0: the middle divider passes the middle.
                assert_eq!(v[1] < 50.0, toward_start, "{skew}: {v:?}");
            }
            assert_eq!(ys, xs, "one curve for both axes");
        }
        // 100 % squares the fraction from the side the dividers crowd to: 1/4, 1/2 and 3/4 from the top.
        let g = rectangular_grid(r, (3, 100.0), (0, 0.0), true);
        assert!(close(&starts(&g[1..]).iter().map(|p| p.y).collect::<Vec<_>>(), &[6.25, 25.0, 56.25]), "{g:?}");
        assert_eq!(rectangular_grid(r, (3, f64::NAN), (1, f64::NAN), true), framed);
    }

    /// #1377: the ellipses come from the outside in and skew toward the outside (positive) or the
    /// centre; the radial dividers keep the top one and skew clockwise (positive) of it or
    /// counterclockwise; as compound paths the ellipses pair up into rings.
    #[test]
    fn a_polar_grid_skews_its_dividers_and_pairs_ellipses_into_rings() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let widths = |g: &[PathData], n: usize| g[..n].iter().map(|p| p.bounds().unwrap().width()).collect::<Vec<_>>();
        for (skew, want) in [(0.0, [100.0, 75.0, 50.0, 25.0]), (100.0, [100.0, 93.75, 75.0, 43.75]), (-100.0, [100.0, 56.25, 25.0, 6.25])] {
            let got = widths(&polar_grid(r, (3, skew), (0, 0.0), false), 4);
            assert!(close(&got, &want), "{skew}: {got:?}");
        }
        // Radial: four dividers point up, right, down and left; skewed, the first stays up and the
        // other three all point to the right (clockwise of it) or to the left.
        let ends = |skew: f64| polar_grid(r, (0, 0.0), (4, skew), false)[1..].iter().map(|p| p.subpaths[0].anchors[1].p).collect::<Vec<_>>();
        let even = ends(0.0);
        for (p, want) in even.iter().zip([(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)]) {
            assert!(p.distance(Point::new(want.0, want.1)) < 1e-9, "{even:?}");
        }
        for (skew, right) in [(GRID_SKEW_MAX, true), (-GRID_SKEW_MAX, false)] {
            let e = ends(skew);
            assert!(e[0].distance(even[0]) < 1e-9);
            assert!(e[1..].iter().all(|p| (p.x > 50.0) == right && (p.x - 50.0).abs() > 1e-6), "{skew}: {e:?}");
        }
        // Compound: 4 ellipses are 2 rings, 5 are 2 rings and the innermost ellipse; each ring's
        // hole runs against its outside.
        for (concentric, shapes) in [(3, vec![2, 2]), (4, vec![2, 2, 1]), (0, vec![1])] {
            let g = polar_grid(r, (concentric, 0.0), (2, 0.0), true);
            assert_eq!(g.len(), shapes.len() + 2);
            assert_eq!(g[..shapes.len()].iter().map(|p| p.subpaths.len()).collect::<Vec<_>>(), shapes);
            for ring in g.iter().filter(|p| p.subpaths.len() == 2) {
                assert!(ring.is_closed());
                assert!(ring.subpaths[0].signed_area() * ring.subpaths[1].signed_area() < 0.0);
                assert!(ring.subpaths[0].signed_area().abs() > ring.subpaths[1].signed_area().abs());
            }
        }
    }

    /// An arc with Base Along Y, open or closed, convex or concave, is the arc with Base Along X in
    /// the same box mirrored top to bottom, anchor for anchor and handle for handle.
    #[test]
    fn a_y_base_arc_is_the_x_base_arc_mirrored_top_to_bottom() {
        let mirror = |p: Point| Point::new(p.x, 20.0 + 70.0 - p.y);
        for closed in [false, true] {
            for slope in [100.0, 50.0, -50.0, -100.0] {
                let x = arc_segment(Point::new(10.0, 20.0), 0, 100.0, 50.0, true, slope, closed);
                let y = arc_segment(Point::new(10.0, 20.0), 0, 100.0, 50.0, false, slope, closed);
                let (xs, ys) = (&x.subpaths[0], &y.subpaths[0]);
                assert_eq!(ys.closed, closed);
                assert_eq!(xs.anchors.len(), ys.anchors.len(), "slope {slope}, closed {closed}");
                for (i, (p, q)) in xs.anchors.iter().zip(&ys.anchors).enumerate() {
                    for (u, v) in [(p.p, q.p), (p.h_in, q.h_in), (p.h_out, q.h_out)] {
                        assert!(mirror(u).distance(v) < 1e-9, "slope {slope}, closed {closed}, anchor {i}: {u:?} mirrored is not {v:?}");
                    }
                }
            }
        }
    }

    /// A closed convex arc is the quarter-ellipse sector of its box with either base: its area is
    /// a quarter of the ellipse's, π/4 of the box's.
    #[test]
    fn a_closed_convex_arc_is_a_quarter_ellipse_sector_with_either_base() {
        for base_along_x in [true, false] {
            let a = arc_segment(Point::ZERO, 0, 100.0, 50.0, base_along_x, 100.0, true).subpaths[0].signed_area().abs();
            assert!((a / (100.0 * 50.0) - PI / 4.0).abs() < 0.01, "base along x {base_along_x}: area {a}");
        }
    }

    #[test]
    fn arc_open_and_closed() {
        assert_eq!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, false, true).anchor_count(), 2);
        assert!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, true, true).subpaths[0].closed);
        let seg = arc_segment(Point::ZERO, 0, 100.0, 50.0, true, 50.0, false);
        assert_eq!(seg.anchor_count(), 2);
    }
}
