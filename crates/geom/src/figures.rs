//! Heart, arrow and gear outlines (#3865): shapes the reference app's Shape tools don't draw,
//! each drawn in a box, clockwise on screen like [`crate::shapes::rectangle`].

use std::f64::consts::{FRAC_PI_2, TAU};

use kurbo::{Point, Rect, Vec2};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};

/// The thinnest arrow shaft and shortest arrowhead, as a fraction of the box.
pub const ARROW_MIN: f64 = 0.01;
/// Most teeth a gear has.
pub const GEAR_MAX_TEETH: u32 = 360;
/// Shallowest and deepest gear teeth, as a fraction of the outer radius.
pub const GEAR_DEPTH_RANGE: (f64, f64) = (0.02, 0.9);
/// The thinnest gear rim left round the hole, as a fraction of the outer radius.
pub const GEAR_MIN_RIM: f64 = 0.05;

/// A point given in the unit box (0..1 both ways, y down) placed in `r`.
fn place(r: Rect, x: f64, y: f64) -> Point {
    Point::new(r.x0 + x * r.width(), r.y0 + y * r.height())
}

/// A finite fraction clamped into `lo..=hi`; `fallback` when it isn't a number.
fn fraction(v: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { fallback }
}

/// A heart filling `r`: two round lobes meeting in a notch at the top, and a point at the bottom.
/// Six anchors, clockwise from the notch: the notch and the point are corners, the tops and the
/// widest points of the lobes are smooth (so the box touches them there).
pub fn heart(r: Rect) -> PathData {
    let r = r.abs();
    let at = |x: f64, y: f64| place(r, x, y);
    let a = |p: (f64, f64), h_in: (f64, f64), h_out: (f64, f64), kind: AnchorKind| Anchor {
        p: at(p.0, p.1),
        h_in: at(h_in.0, h_in.1),
        h_out: at(h_out.0, h_out.1),
        kind,
    };
    let anchors = vec![
        // The notch, both handles pointing up into the lobes.
        a((0.5, 0.22), (0.5, 0.08), (0.5, 0.08), AnchorKind::Corner),
        a((0.75, 0.0), (0.61, 0.0), (0.89, 0.0), AnchorKind::Smooth),
        a((1.0, 0.3), (1.0, 0.12), (1.0, 0.55), AnchorKind::Smooth),
        // The point, reached from both sides at the same slope.
        a((0.5, 1.0), (0.66, 0.8), (0.34, 0.8), AnchorKind::Corner),
        a((0.0, 0.3), (0.0, 0.55), (0.0, 0.12), AnchorKind::Smooth),
        a((0.25, 0.0), (0.11, 0.0), (0.39, 0.0), AnchorKind::Smooth),
    ];
    PathData::single(SubPath::new(anchors, true))
}

/// An arrow filling `r`, pointing right (`heads` 1) or both ways (`heads` 2 or more). `shaft` is
/// the shaft's thickness as a fraction of the height, `head` each arrowhead's length as a fraction
/// of the width (with two heads at most half of it each). A shaft as thick as the box, or a head as
/// long as it, leaves out the anchors that would fall on others.
pub fn arrow(r: Rect, shaft: f64, head: f64, heads: u32) -> PathData {
    let r = r.abs();
    let two = heads >= 2;
    let s = fraction(shaft, ARROW_MIN, 1.0, 0.4);
    let h = fraction(head, ARROW_MIN, if two { 0.5 } else { 1.0 }, 0.35);
    let (top, bottom) = ((1.0 - s) / 2.0, (1.0 + s) / 2.0);
    let neck = 1.0 - h;
    let pts: Vec<(f64, f64)> = if two {
        vec![(0.0, 0.5), (h, 0.0), (h, top), (neck, top), (neck, 0.0), (1.0, 0.5), (neck, 1.0), (neck, bottom), (h, bottom), (h, 1.0)]
    } else {
        vec![(0.0, top), (neck, top), (neck, 0.0), (1.0, 0.5), (neck, 1.0), (neck, bottom), (0.0, bottom)]
    };
    let placed: Vec<Point> = corners_only(pts).into_iter().map(|(x, y)| place(r, x, y)).collect();
    PathData::single(SubPath::polyline(&placed, true))
}

/// The points of a closed polygon less those that fall on a neighbour or on the straight line
/// between their neighbours (they would be anchors that turn nothing).
fn corners_only(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    const EPS: f64 = 1e-12;
    loop {
        let n = pts.len();
        if n <= 3 {
            return pts;
        }
        let idle = (0..n).find(|&i| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs() < EPS
        });
        match idle {
            Some(i) => {
                pts.remove(i);
            }
            None => return pts,
        }
    }
}

/// The anchors of the elliptical arc of `c` and radii `rad` from angle `a0` to `a1` (radians, y down,
/// so a rising angle turns clockwise on screen), at most a quarter turn: the two ends as corners,
/// their handles along the arc.
fn arc_ends(c: Point, rad: Vec2, a0: f64, a1: f64) -> (Anchor, Anchor) {
    let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan();
    let at = |a: f64| Point::new(c.x + rad.x * a.cos(), c.y + rad.y * a.sin());
    let d = |a: f64| Vec2::new(-rad.x * a.sin(), rad.y * a.cos()) * k;
    let (p0, p1) = (at(a0), at(a1));
    (Anchor { p: p0, h_in: p0, h_out: p0 + d(a0), kind: AnchorKind::Corner }, Anchor { p: p1, h_in: p1 - d(a1), h_out: p1, kind: AnchorKind::Corner })
}

/// A gear in `r` (its tooth tops on the ellipse inscribed in `r`): `teeth` teeth (3 to [`GEAR_MAX_TEETH`]) round a rim, the first one straight
/// up. `depth` is how far the teeth stand out, and `hole` the radius of the hole in the middle (0:
/// none), both as a fraction of the outer radius; the hole leaves at least [`GEAR_MIN_RIM`] of rim.
/// Tooth tops and the rim between them are arcs. The hole is a second, counterclockwise subpath, so
/// it stays open under the non-zero fill rule.
pub fn gear(r: Rect, teeth: u32, depth: f64, hole: f64) -> PathData {
    let r = r.abs();
    let n = teeth.clamp(3, GEAR_MAX_TEETH);
    let d = fraction(depth, GEAR_DEPTH_RANGE.0, GEAR_DEPTH_RANGE.1, 0.2);
    let root = 1.0 - d;
    let c = r.center();
    let outer = Vec2::new(r.width() / 2.0, r.height() / 2.0);
    let inner = outer * root;
    let pitch = TAU / f64::from(n);
    // A tooth is 40 % of the pitch at its foot and 25 % at its top, so its flanks lean in.
    let (foot, top) = (0.2 * pitch, 0.125 * pitch);
    let mut anchors: Vec<Anchor> = Vec::with_capacity(4 * n as usize);
    for i in 0..n {
        let mid = -FRAC_PI_2 + pitch * f64::from(i);
        let (rim_end, tip_start) = (arc_ends(c, inner, mid - pitch + foot, mid - foot), arc_ends(c, outer, mid - top, mid + top));
        // The rim before this tooth ends at its foot; the previous tooth's foot started it.
        if let Some(last) = anchors.last_mut() {
            last.h_out = rim_end.0.h_out;
        }
        anchors.push(rim_end.1);
        anchors.push(tip_start.0);
        anchors.push(tip_start.1);
        anchors.push(Anchor::corner(Point::new(c.x + inner.x * (mid + foot).cos(), c.y + inner.y * (mid + foot).sin())));
    }
    // The first rim arc starts at the last tooth's foot.
    let first_rim = arc_ends(c, inner, -FRAC_PI_2 - pitch + foot, -FRAC_PI_2 - foot);
    if let Some(last) = anchors.last_mut() {
        last.h_out = first_rim.0.h_out;
    }
    let mut subpaths = vec![SubPath::new(anchors, true)];
    let hole = fraction(hole, 0.0, root - GEAR_MIN_RIM, 0.0);
    if hole > 0.0 {
        let half = outer * hole;
        let mut ring = crate::shapes::ellipse(Rect::new(c.x - half.x, c.y - half.y, c.x + half.x, c.y + half.y)).subpaths;
        for sp in &mut ring {
            sp.reverse();
        }
        subpaths.extend(ring);
    }
    PathData::new(subpaths)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: Rect = Rect::new(10.0, 20.0, 210.0, 120.0);

    fn close(a: Option<Rect>, b: Rect) -> bool {
        a.is_some_and(|a| (a.x0 - b.x0).abs() < 1e-6 && (a.y0 - b.y0).abs() < 1e-6 && (a.x1 - b.x1).abs() < 1e-6 && (a.y1 - b.y1).abs() < 1e-6)
    }

    #[test]
    fn a_heart_fills_its_box_clockwise_with_a_notch_and_a_point() {
        let h = heart(BOX);
        assert!(close(h.bounds(), BOX), "{:?}", h.bounds());
        let sp = &h.subpaths[0];
        assert!(sp.closed && sp.anchors.len() == 6);
        assert!(sp.area() > 0.0, "clockwise on screen, as the rectangle");
        let corners: Vec<Point> = sp.anchors.iter().filter(|a| a.kind == AnchorKind::Corner).map(|a| a.p).collect();
        assert_eq!(corners, [Point::new(110.0, 42.0), Point::new(110.0, 120.0)], "the notch and the point");
        // Mirror-symmetric about the vertical centre line.
        for a in &sp.anchors {
            let m = Point::new(2.0 * 110.0 - a.p.x, a.p.y);
            assert!(sp.anchors.iter().any(|b| b.p.distance(m) < 1e-9), "{:?} has no mirror", a.p);
        }
        // The upside-down box is the same box.
        assert_eq!(heart(Rect::new(210.0, 120.0, 10.0, 20.0)), h);
    }

    #[test]
    fn an_arrow_points_right_with_its_shaft_and_head_in_proportion() {
        let a = arrow(BOX, 0.4, 0.25, 1);
        assert!(close(a.bounds(), BOX));
        let sp = &a.subpaths[0];
        assert!(sp.closed && sp.area() > 0.0);
        let pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
        assert_eq!(pts.len(), 7);
        assert!(pts.contains(&Point::new(210.0, 70.0)), "the tip, half way down the right side");
        assert!(pts.contains(&Point::new(10.0, 50.0)) && pts.contains(&Point::new(10.0, 90.0)), "a shaft 40 % of the height");
        assert!(pts.contains(&Point::new(160.0, 20.0)), "a head a quarter of the width long");
        // Area: the shaft plus the head's triangle.
        let want = 150.0 * 40.0 + 50.0 * 100.0 / 2.0;
        assert!((sp.area() - want).abs() < 1e-6, "{}", sp.area());
    }

    #[test]
    fn a_double_arrow_points_both_ways_and_keeps_its_heads_apart() {
        let a = arrow(BOX, 0.4, 0.25, 2);
        let pts: Vec<Point> = a.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(pts.len(), 10);
        assert!(pts.contains(&Point::new(10.0, 70.0)) && pts.contains(&Point::new(210.0, 70.0)));
        assert!(a.subpaths[0].area() > 0.0);
        // Heads asked to be longer than half the width meet in the middle.
        let long = arrow(BOX, 0.4, 0.9, 2);
        assert!(close(long.bounds(), BOX));
        assert!(long.subpaths[0].anchors.iter().all(|a| (a.p.x - 110.0).abs() < 1e-9 || a.p.x <= 10.0 + 1e-9 || a.p.x >= 210.0 - 1e-9));
    }

    #[test]
    fn arrow_extremes_leave_no_doubled_anchors() {
        // A shaft as thick as the box: the shoulders fall on the head's back corners.
        let fat = arrow(BOX, 1.0, 0.25, 1);
        let pts: Vec<Point> = fat.subpaths[0].anchors.iter().map(|a| a.p).collect();
        for (i, p) in pts.iter().enumerate() {
            let next = pts[(i + 1) % pts.len()];
            assert!(p.distance(next) > 1e-9, "{pts:?}");
        }
        // A head as long as the box: a triangle.
        assert_eq!(arrow(BOX, 0.4, 1.0, 1).subpaths[0].anchors.len(), 3);
        // Out-of-range and non-numbers are clamped or fall back, never NaN.
        for (s, h) in [(f64::NAN, f64::INFINITY), (-1.0, -1.0), (5.0, 5.0)] {
            let p = arrow(BOX, s, h, 1);
            assert!(p.subpaths[0].anchors.iter().all(|a| a.p.x.is_finite() && a.p.y.is_finite()));
            assert!(close(p.bounds(), BOX));
        }
    }

    #[test]
    fn a_gear_has_its_teeth_round_a_rim_and_a_hole() {
        let r = Rect::new(0.0, 0.0, 200.0, 200.0);
        let g = gear(r, 12, 0.2, 0.3);
        assert_eq!(g.subpaths.len(), 2, "the outline and the hole");
        let (outline, hole) = (&g.subpaths[0], &g.subpaths[1]);
        assert_eq!(outline.anchors.len(), 48, "four anchors a tooth");
        assert!(outline.area() > 0.0 && hole.area() < 0.0, "the hole runs the other way");
        assert!(close(g.bounds(), r), "{:?}", g.bounds());
        // Every anchor is on the rim (radius 80) or on a tooth top (radius 100).
        for a in &outline.anchors {
            let d = a.p.distance(Point::new(100.0, 100.0));
            assert!((d - 80.0).abs() < 1e-9 || (d - 100.0).abs() < 1e-9, "{d}");
        }
        // The first tooth is straight up: its top is centred on x = 100.
        let tops: Vec<Point> = outline.anchors.iter().filter(|a| a.p.y < 1.0).map(|a| a.p).collect();
        assert_eq!(tops.len(), 2);
        assert!((tops[0].x + tops[1].x - 200.0).abs() < 1e-9);
        // The hole: radius 30.
        let hb = hole.anchors.iter().map(|a| a.p.distance(Point::new(100.0, 100.0))).fold(0.0, f64::max);
        assert!((hb - 30.0).abs() < 1e-9);
        // Smaller than a disc of the outer radius, larger than the rim's disc less the hole.
        let area = outline.area() + hole.area();
        assert!(area < std::f64::consts::PI * 100.0 * 100.0 && area > std::f64::consts::PI * (80.0 * 80.0 - 30.0 * 30.0));
    }

    #[test]
    fn gear_parameters_are_clamped() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(gear(r, 0, 0.2, 0.0).subpaths[0].anchors.len(), 12, "three teeth at least");
        assert_eq!(gear(r, 100_000, 0.2, 0.0).subpaths[0].anchors.len(), 4 * GEAR_MAX_TEETH as usize);
        assert_eq!(gear(r, 8, 0.2, 0.0).subpaths.len(), 1, "no hole");
        // A hole as big as the gear leaves a rim.
        let g = gear(Rect::new(0.0, 0.0, 100.0, 100.0), 8, 0.2, 1.0);
        let hole = g.subpaths[1].anchors.iter().map(|a| a.p.distance(Point::new(50.0, 50.0))).fold(0.0, f64::max);
        assert!((hole - 50.0 * (0.8 - GEAR_MIN_RIM)).abs() < 1e-9, "{hole}");
        for (d, h) in [(f64::NAN, f64::NAN), (-3.0, -3.0), (9.0, 9.0)] {
            let g = gear(r, 8, d, h);
            assert!(g.subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.x.is_finite() && a.h_in.y.is_finite() && a.h_out.x.is_finite()));
            assert!(close(g.bounds(), r), "{:?}", g.bounds());
        }
    }
}
