//! Shape generators matching Illustrator's anchor layouts (so later edits behave the same).

use std::f64::consts::{PI, TAU};

use kurbo::{Point, Rect, Vec2};

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
    let r = r.abs();
    let rad = radius.max(0.0).min(r.width() / 2.0).min(r.height() / 2.0);
    if rad <= 1e-9 {
        return rectangle(r);
    }
    let k = rad * KAPPA;
    let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);
    let a = |p: (f64, f64), hin: (f64, f64), hout: (f64, f64)| Anchor { p: p.into(), h_in: hin.into(), h_out: hout.into(), kind: AnchorKind::Corner };
    let anchors = vec![
        a((x0 + rad, y0), (x0 + rad - k, y0), (x0 + rad, y0)),
        a((x1 - rad, y0), (x1 - rad, y0), (x1 - rad + k, y0)),
        a((x1, y0 + rad), (x1, y0 + rad - k), (x1, y0 + rad)),
        a((x1, y1 - rad), (x1, y1 - rad), (x1, y1 - rad + k)),
        a((x1 - rad, y1), (x1 - rad + k, y1), (x1 - rad, y1)),
        a((x0 + rad, y1), (x0 + rad, y1), (x0 + rad - k, y1)),
        a((x0, y1 - rad), (x0, y1 - rad + k), (x0, y1 - rad)),
        a((x0, y0 + rad), (x0, y0 + rad), (x0, y0 + rad - k)),
    ];
    PathData::single(SubPath::new(anchors, true))
}

/// Rectangle with its own radius at each corner (top-left, top-right, bottom-right, bottom-left;
/// each clamped to half the shorter side). A square corner is one anchor, a round one two, in the
/// layouts of [`rectangle`] and [`rounded_rectangle`]: with equal radii the result is the same.
pub fn rounded_rectangle_each(r: Rect, radii: [f64; 4]) -> PathData {
    let anchors = rect_corner_anchors(r, radii).into_iter().map(|(_, a)| a).collect();
    PathData::single(SubPath::new(anchors, true))
}

/// The corner (0 = top-left … 3 = bottom-left) of each anchor of [`rounded_rectangle_each`].
pub fn rounded_rectangle_corners(r: Rect, radii: [f64; 4]) -> Vec<usize> {
    rect_corner_anchors(r, radii).into_iter().map(|(k, _)| k).collect()
}

fn rect_corner_anchors(r: Rect, radii: [f64; 4]) -> Vec<(usize, Anchor)> {
    let r = r.abs();
    let max = r.width().min(r.height()) / 2.0;
    let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);
    // Each corner: its point and the unit directions of the edges arriving at and leaving it.
    let corners = [
        (Point::new(x0, y0), Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0)),
        (Point::new(x1, y0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)),
        (Point::new(x1, y1), Vec2::new(0.0, 1.0), Vec2::new(-1.0, 0.0)),
        (Point::new(x0, y1), Vec2::new(-1.0, 0.0), Vec2::new(0.0, -1.0)),
    ];
    let rad = |k: usize| {
        let v = radii.get(k).copied().unwrap_or(0.0);
        if v.is_finite() { v.max(0.0).min(max) } else { 0.0 }
    };
    let corner = |p: Point, kind| Anchor { p, h_in: p, h_out: p, kind };
    // Each round corner as (entry, exit): the entry's out handle and the exit's in handle bend
    // along the arc.
    let parts: Vec<(usize, Vec<Anchor>)> = corners
        .iter()
        .enumerate()
        .map(|(k, &(c, din, dout))| {
            let rk = rad(k);
            if rk <= 1e-9 {
                return (k, vec![corner(c, AnchorKind::Corner)]);
            }
            let (a, b) = (c - din * rk, c + dout * rk);
            let h = rk * KAPPA;
            let entry = Anchor { p: a, h_in: a, h_out: a + din * h, kind: AnchorKind::Corner };
            let exit = Anchor { p: b, h_in: b - dout * h, h_out: b, kind: AnchorKind::Corner };
            (k, vec![entry, exit])
        })
        .collect();
    // Start at the top-left corner's exit (or the corner itself) and end with its entry.
    let mut out = vec![];
    let mut tail = vec![];
    for (k, v) in parts {
        match (k, v.as_slice()) {
            (0, [entry, exit]) => {
                out.push((0, *exit));
                tail.push((0, *entry));
            }
            _ => out.extend(v.into_iter().map(|a| (k, a))),
        }
    }
    out.extend(tail);
    out
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

/// Quarter-ellipse arc from `a` to `b` (Arc tool default: concave, open, x-axis base).
pub fn arc(a: Point, b: Point, slope: f64, closed: bool) -> PathData {
    // Corner of the arc's bounding box is at (a.x, b.y): the arc bows away from it.
    let k = KAPPA * (1.0 + slope.clamp(-1.0, 1.0) * 0.0);
    let corner = Point::new(b.x, a.y);
    let h1 = a + (corner - a) * k;
    let h2 = b + (corner - b) * k;
    let mut anchors =
        vec![Anchor { p: a, h_in: a, h_out: h1, kind: AnchorKind::Corner }, Anchor { p: b, h_in: h2, h_out: b, kind: AnchorKind::Corner }];
    if closed {
        let o = Point::new(a.x, b.y);
        anchors.push(Anchor::corner(o));
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

/// Rectangular grid as separate open lines (plus an optional frame rectangle).
pub fn rectangular_grid(r: Rect, h_dividers: u32, v_dividers: u32, frame: bool) -> Vec<PathData> {
    let r = r.abs();
    let mut out = Vec::new();
    for i in 1..=h_dividers {
        let y = r.y0 + r.height() * i as f64 / (h_dividers + 1) as f64;
        out.push(line(Point::new(r.x0, y), Point::new(r.x1, y)));
    }
    for i in 1..=v_dividers {
        let x = r.x0 + r.width() * i as f64 / (v_dividers + 1) as f64;
        out.push(line(Point::new(x, r.y0), Point::new(x, r.y1)));
    }
    if frame {
        out.push(rectangle(r));
    }
    out
}

/// Polar grid: concentric ellipses plus radial dividers.
pub fn polar_grid(r: Rect, concentric: u32, radial: u32) -> Vec<PathData> {
    let r = r.abs();
    let c = r.center();
    let mut out = Vec::new();
    for i in 1..=concentric + 1 {
        let f = i as f64 / (concentric + 1) as f64;
        out.push(ellipse(Rect::from_center_size(c, (r.width() * f, r.height() * f))));
    }
    for i in 0..radial.max(1) {
        let a = -PI / 2.0 + TAU * i as f64 / radial.max(1) as f64;
        out.push(line(c, Point::new(c.x + r.width() / 2.0 * a.cos(), c.y + r.height() / 2.0 * a.sin())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn each_corner_keeps_the_uniform_layouts() {
        let r = Rect::new(0.0, 0.0, 20.0, 10.0);
        assert_eq!(rounded_rectangle_each(r, [3.0; 4]), rounded_rectangle(r, 3.0));
        assert_eq!(rounded_rectangle_each(r, [0.0; 4]), rectangle(r));
        assert_eq!(rounded_rectangle_each(r, [100.0; 4]), rounded_rectangle(r, 100.0));
        assert_eq!(rounded_rectangle_corners(r, [3.0; 4]), [0, 1, 1, 2, 2, 3, 3, 0]);
    }

    #[test]
    fn each_corner_rounds_on_its_own() {
        let r = Rect::new(0.0, 0.0, 20.0, 10.0);
        // Only the top-right corner round: 5 anchors, two of them on that corner.
        let p = rounded_rectangle_each(r, [0.0, 4.0, 0.0, f64::NAN]);
        assert_eq!(p.anchor_count(), 5);
        assert_eq!(rounded_rectangle_corners(r, [0.0, 4.0, 0.0, 0.0]), [0, 1, 1, 2, 3]);
        let pts: Vec<(f64, f64)> = p.subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect();
        assert_eq!(pts, [(0.0, 0.0), (16.0, 0.0), (20.0, 4.0), (20.0, 10.0), (0.0, 10.0)]);
        assert_eq!(p.bounds(), Some(r));
        // The top-left corner round: its exit first, its entry last.
        assert_eq!(rounded_rectangle_corners(r, [2.0, 0.0, 0.0, 0.0]), [0, 1, 2, 3, 0]);
    }

    #[test]
    fn rounded_rect_clamps() {
        let p = rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 100.0);
        assert_eq!(p.anchor_count(), 8);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 20.0, 10.0)));
        assert_eq!(rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 0.0).anchor_count(), 4);
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
        assert_eq!(rectangular_grid(Rect::new(0.0, 0.0, 10.0, 10.0), 5, 5, true).len(), 11);
        assert_eq!(polar_grid(Rect::new(0.0, 0.0, 10.0, 10.0), 5, 5).len(), 11);
    }

    #[test]
    fn arc_open_and_closed() {
        assert_eq!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, false).anchor_count(), 2);
        assert!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, true).subpaths[0].closed);
    }
}
