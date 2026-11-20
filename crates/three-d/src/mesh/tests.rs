use std::collections::HashMap;

use vectorcraft_geom::{BezPath, FillRule, PathData, Rect};

use super::*;
use crate::{Bevel, BevelShape, Edge};

fn square(b: &mut BezPath, x: f64, y: f64, s: f64, clockwise: bool) {
    b.move_to((x, y));
    if clockwise {
        b.line_to((x, y + s));
        b.line_to((x + s, y + s));
        b.line_to((x + s, y));
    } else {
        b.line_to((x + s, y));
        b.line_to((x + s, y + s));
        b.line_to((x, y + s));
    }
    b.close_path();
}

fn shape(b: &BezPath) -> (PathData, Rect) {
    let p = PathData::from_bezpath(b);
    let r = p.bounds().unwrap();
    (p, r)
}

/// Straight on, without perspective.
fn front() -> View {
    View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, perspective: 0.0, ..View::default() }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn corners(m: &Mesh, t: [u32; 3]) -> [[f64; 3]; 3] {
    t.map(|i| m.positions[i as usize])
}

/// The area of every triangle together.
fn area(m: &Mesh) -> f64 {
    m.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = corners(m, *t);
            dot(cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(c, a))).sqrt() / 2.0
        })
        .sum()
}

/// Every edge of every triangle is met by an edge running the other way, as often: the surface is
/// closed and its triangles turn one way.
fn closed(m: &Mesh) -> bool {
    let key = |p: [f64; 3]| p.map(|v| (v * 1e6).round() as i64);
    let mut edges: HashMap<([i64; 3], [i64; 3]), i64> = HashMap::new();
    for t in &m.triangles {
        let c = corners(m, *t).map(key);
        for (a, b) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[0])] {
            *edges.entry((a, b)).or_default() += 1;
            *edges.entry((b, a)).or_default() -= 1;
        }
    }
    edges.values().all(|n| *n == 0)
}

/// Every triangle of a solid convex about the origin turns its front away from the origin, and
/// agrees with its corners' normals.
fn outward(m: &Mesh) -> bool {
    m.triangles.iter().all(|t| {
        let [a, b, c] = corners(m, *t);
        let n = cross(sub(b, a), sub(c, a));
        let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
        dot(n, centre) > 0.0 && dot(n, m.normals[t[0] as usize]) > 0.0
    })
}

#[test]
fn an_extruded_square_is_a_closed_box_about_its_centre() {
    let mut b = BezPath::new();
    square(&mut b, 200.0, 300.0, 100.0, false);
    let (p, r) = shape(&b);
    let m = extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: 50.0, ..Extrude::default() }, front()).unwrap();
    assert_eq!(m.triangles.len(), 12, "two triangles for each side of the box");
    assert_eq!(m.positions.len(), m.normals.len());
    assert_eq!(m.bounds(), Some(([-50.0, -50.0, -25.0], [50.0, 50.0, 25.0])));
    assert!((area(&m) - (2.0 * 100.0 * 100.0 + 4.0 * 100.0 * 50.0)).abs() < 1e-6, "{}", area(&m));
    assert!(closed(&m));
    assert!(outward(&m));
    assert!(m.normals.iter().all(|n| (dot(*n, *n) - 1.0).abs() < 1e-9));
}

#[test]
fn caps_with_holes_are_cut_around_them() {
    // Two holes in one outline: each cap keeps the outline less both holes.
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 100.0, false);
    square(&mut b, 10.0, 10.0, 30.0, true);
    square(&mut b, 60.0, 50.0, 30.0, true);
    let (p, r) = shape(&b);
    let m = extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: 20.0, ..Extrude::default() }, front()).unwrap();
    let caps = 2.0 * (100.0 * 100.0 - 2.0 * 30.0 * 30.0);
    let walls = 4.0 * 100.0 * 20.0 + 2.0 * 4.0 * 30.0 * 20.0;
    assert!((area(&m) - (caps + walls)).abs() < 1e-6, "{} != {}", area(&m), caps + walls);
    assert!(closed(&m));
}

#[test]
fn a_bridge_from_a_hole_never_runs_through_the_hole() {
    // The hole's rightmost corner (60, 50) is nearest the outline's spike tip (10, 50), and the
    // straight line to it runs through the hole and out at its corner (20, 50).
    let poly = |b: &mut BezPath, pts: &[(f64, f64)]| {
        for (k, p) in pts.iter().enumerate() {
            if k == 0 { b.move_to(*p) } else { b.line_to(*p) }
        }
        b.close_path();
    };
    let mut b = BezPath::new();
    poly(&mut b, &[(0.0, 0.0), (200.0, 0.0), (200.0, 100.0), (0.0, 100.0), (0.0, 55.0), (10.0, 50.0), (0.0, 45.0)]);
    let hole = [(60.0, 50.0), (30.0, 40.0), (20.0, 50.0), (30.0, 60.0)];
    poly(&mut b, &hole);
    let (p, r) = shape(&b);
    let m = plane_mesh(&p, FillRule::EvenOdd, r, front()).unwrap();
    let face = 200.0 * 100.0 - 50.0 - 400.0;
    assert!((area(&m) - face).abs() < 1e-6, "{} != {face}", area(&m));
    // No triangle's centre is in the hole (page coordinates: the mesh is y up about (100, 50)).
    let in_hole = |x: f64, y: f64| {
        let mut odd = false;
        for (k, a) in hole.iter().enumerate() {
            let c = hole[(k + 1) % hole.len()];
            if (a.1 > y) != (c.1 > y) && x < a.0 + (y - a.1) / (c.1 - a.1) * (c.0 - a.0) {
                odd = !odd;
            }
        }
        odd
    };
    for t in &m.triangles {
        let [a, b, c] = corners(&m, *t);
        let (x, y) = ((a[0] + b[0] + c[0]) / 3.0 + 100.0, 50.0 - (a[1] + b[1] + c[1]) / 3.0);
        assert!(!in_hole(x, y), "a triangle at ({x}, {y})");
    }
}

#[test]
fn a_hole_that_crosses_the_outline_is_refused_rather_than_filled() {
    // The hole starts inside the square, but its rightmost corner (15, 5) lies outside it, so no
    // bridge from there reaches the square through the face.
    let ring = |pts: &[(f64, f64)]| pts.iter().map(|(x, y)| V3(*x, *y, 0.0)).collect::<Vec<_>>();
    let outline = ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let hole = ring(&[(5.0, 4.0), (5.0, 6.0), (15.0, 5.0)]);
    let mut work = 0;
    assert!(super::triangulate(std::slice::from_ref(&outline), V3(0.0, 0.0, 1.0), &mut work).is_ok());
    let r = super::triangulate(&[outline.clone(), hole], V3(0.0, 0.0, 1.0), &mut work);
    assert!(r.as_ref().is_err_and(|e| e.contains("hole")), "{:?}", r.map(|t| t.len()));
    // The same hole starting outside the square belongs to no outline, and is refused too.
    let hole = ring(&[(15.0, 5.0), (5.0, 4.0), (5.0, 6.0)]);
    let r = super::triangulate(&[outline, hole], V3(0.0, 0.0, 1.0), &mut work);
    assert!(r.as_ref().is_err_and(|e| e.contains("hole")), "{:?}", r.map(|t| t.len()));
}

#[test]
fn a_bridge_through_another_corner_touches_it() {
    assert!(super::touches((5.0, 0.0), (0.0, 0.0), (10.0, 0.0)));
    assert!(!super::touches((0.0, 0.0), (0.0, 0.0), (10.0, 0.0)), "an end is not a touch");
    assert!(!super::touches((5.0, 1.0), (0.0, 0.0), (10.0, 0.0)));
}

#[test]
fn flat_triangles_keep_corners_that_lie_along_an_edge() {
    // A square with a hole, its edges carrying corners midway, as a cell shares with its finer
    // neighbors: every corner is a corner of some triangle, and the triangles cover the face.
    let p = |x: f64, y: f64| Point::new(x, y);
    let outline = vec![p(5.0, 0.0), p(10.0, 0.0), p(10.0, 5.0), p(10.0, 10.0), p(5.0, 10.0), p(0.0, 10.0), p(0.0, 5.0), p(0.0, 0.0)];
    let hole = vec![p(1.0, 4.0), p(1.0, 6.0), p(2.0, 6.0), p(2.0, 4.0)];
    let mut work = 0;
    let triangles = flat_triangles(&[outline.clone(), hole], &mut work, usize::MAX).unwrap();
    let area = |t: &[Point; 3]| ((t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[1].y - t[0].y) * (t[2].x - t[0].x)) / 2.0;
    assert!(triangles.iter().all(|t| area(t) > 0.0));
    assert!((triangles.iter().map(area).sum::<f64>() - 98.0).abs() < 1e-9);
    for q in &outline {
        assert!(triangles.iter().any(|t| t.contains(q)), "{q:?} is no triangle's corner");
    }
    assert!(work > 0);
}

#[test]
fn flat_triangles_keep_corners_on_the_edge_an_ear_would_cut() {
    // The first ear's new edge runs along the bottom, through its two middle corners.
    let p = |x: f64, y: f64| Point::new(x, y);
    let outline = vec![p(0.0, 3.0), p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0), p(3.0, 0.0)];
    let mut work = 0;
    let triangles = flat_triangles(std::slice::from_ref(&outline), &mut work, usize::MAX).unwrap();
    let area = |t: &[Point; 3]| ((t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[1].y - t[0].y) * (t[2].x - t[0].x)) / 2.0;
    assert!((triangles.iter().map(area).sum::<f64>() - 4.5).abs() < 1e-9);
    for q in &outline {
        assert!(triangles.iter().any(|t| t.contains(q)), "{q:?} is no triangle's corner");
    }
}

#[test]
fn flat_triangles_pass_over_an_ear_whose_new_edge_runs_through_a_dent() {
    // The first ear's new edge, from (0, 4) to (4, 0), runs through the dent's corner at (2, 2).
    let p = |x: f64, y: f64| Point::new(x, y);
    let outline = vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(2.0, 2.0), p(0.0, 4.0)];
    let mut work = 0;
    let triangles = flat_triangles(std::slice::from_ref(&outline), &mut work, usize::MAX).unwrap();
    let area = |t: &[Point; 3]| ((t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[1].y - t[0].y) * (t[2].x - t[0].x)) / 2.0;
    assert!((triangles.iter().map(area).sum::<f64>() - 12.0).abs() < 1e-9);
    for t in &triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            for q in &outline {
                assert!(!super::touches((q.x, q.y), (a.x, a.y), (b.x, b.y)), "{q:?} lies inside the edge {a:?} to {b:?}");
            }
        }
    }
}

#[test]
fn flat_triangles_stop_at_their_budget() {
    let p = |x: f64, y: f64| Point::new(x, y);
    let outline = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    let hole = vec![p(1.0, 4.0), p(1.0, 6.0), p(2.0, 6.0), p(2.0, 4.0)];
    let mut all = 0;
    assert!(flat_triangles(&[outline.clone(), hole.clone()], &mut all, usize::MAX).is_some());
    let mut work = 0;
    assert!(flat_triangles(&[outline, hole], &mut work, all - 1).is_none());
    assert!(work >= all, "{work} of {all}");
}

#[test]
fn a_beveled_extrusion_stays_closed() {
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 100.0, false);
    let (p, r) = shape(&b);
    let bevel = Bevel { width: 6.0, height: 6.0, both_sides: true, ..Bevel::default() };
    let m = extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: 40.0, bevel: Some(bevel), ..Extrude::default() }, front()).unwrap();
    assert!(closed(&m));
    assert!(outward(&m));
}

#[test]
fn the_mesh_keeps_the_turn_of_the_view() {
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 100.0, false);
    let (p, r) = shape(&b);
    // Turned 90° about the vertical axis, the extrusion's depth lies along x.
    let view = View { rotation_y: 90.0, ..front() };
    let m = extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: 20.0, ..Extrude::default() }, view).unwrap();
    let (lo, hi) = m.bounds().unwrap();
    assert!((hi[0] - lo[0] - 20.0).abs() < 1e-9 && (hi[2] - lo[2] - 100.0).abs() < 1e-9, "{lo:?} {hi:?}");
}

#[test]
fn a_turned_plane_keeps_its_area_less_its_hole() {
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 100.0, false);
    square(&mut b, 25.0, 25.0, 50.0, true);
    let (p, r) = shape(&b);
    let view = View { rotation_x: 30.0, rotation_y: -40.0, ..front() };
    let m = plane_mesh(&p, FillRule::NonZero, r, view).unwrap();
    assert!((area(&m) - (100.0 * 100.0 - 50.0 * 50.0)).abs() < 1e-6, "{}", area(&m));
}

#[test]
fn y_points_up() {
    // The art's top edge (y 0 on the page) is the mesh's highest.
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    b.line_to((100.0, 0.0));
    b.line_to((50.0, 100.0));
    b.close_path();
    let (p, r) = shape(&b);
    let m = plane_mesh(&p, FillRule::NonZero, r, front()).unwrap();
    let top = m.positions.iter().filter(|q| (q[1] - 50.0).abs() < 1e-9).count();
    assert!(top >= 2, "{:?}", m.positions);
    assert!(m.normals.iter().all(|n| n[2] > 0.99), "a plane facing the viewer has its normal toward +z");
}

#[test]
fn a_revolved_profile_is_a_closed_cylinder() {
    // A vertical line 10 pt from the axis, swept round with its ends capped: a closed cylinder.
    let mut b = BezPath::new();
    b.move_to((20.0, 0.0));
    b.line_to((20.0, 100.0));
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    let o = Revolve { offset: 10.0, edge: Edge::Left, segments: 64, ..Revolve::default() };
    let m = revolve_mesh(&p, FillRule::NonZero, r, o).unwrap();
    assert!(closed(&m));
    let exact = 2.0 * std::f64::consts::PI * 10.0 * 100.0 + 2.0 * std::f64::consts::PI * 100.0;
    assert!((area(&m) - exact).abs() / exact < 0.01, "{} vs {exact}", area(&m));
    assert!(outward(&m));
}

#[test]
fn a_partial_revolve_caps_its_ends_with_sectors_and_closes_its_cut_sides() {
    let mut b = BezPath::new();
    b.move_to((20.0, 0.0));
    b.line_to((20.0, 100.0));
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    let o = Revolve { angle: 90.0, offset: 10.0, segments: 64, ..Revolve::default() };
    let m = revolve_mesh(&p, FillRule::NonZero, r, o).unwrap();
    // The side and the two end sectors, and the two cut sides, each 10 pt from the axis by 100 pt.
    let exact = 0.25 * (2.0 * std::f64::consts::PI * 10.0 * 100.0 + 2.0 * std::f64::consts::PI * 100.0) + 2.0 * 10.0 * 100.0;
    assert!((area(&m) - exact).abs() / exact < 0.01, "{} vs {exact}", area(&m));
    assert!(closed(&m), "the quarter of a cylinder is a closed solid");
    let hollow = revolve_mesh(&p, FillRule::NonZero, r, Revolve { cap_ends: false, ..o }).unwrap();
    assert!(!closed(&hollow));
}

/// A vertical line 40 pt from the axis, from y 10 to 90: an open cylinder of radius 40 about the
/// axis at x 0, its middle height 50.
fn tube() -> (PathData, Rect) {
    let mut b = BezPath::new();
    b.move_to((40.0, 10.0));
    b.line_to((40.0, 90.0));
    (PathData::from_bezpath(&b), Rect::new(0.0, 10.0, 40.0, 90.0))
}

#[test]
fn a_revolved_sides_corners_lie_where_the_canvas_places_the_unrolled_side() {
    let (p, r) = tube();
    let o = Revolve { map_surface: true, ..Revolve::default() };
    let (m, on) = revolve_mesh_surface(&p, FillRule::NonZero, r, o).unwrap();
    assert_eq!(on.len(), m.triangles.len());
    assert_eq!(m.triangles.len(), revolve_mesh(&p, FillRule::NonZero, r, o).unwrap().triangles.len());
    let step = 40.0 * std::f64::consts::TAU / 64.0;
    let (mut side, mut caps) = (0, 0);
    for (t, points) in m.triangles.iter().zip(&on) {
        let c = corners(&m, *t);
        let Some(points) = points else {
            // The end caps: flat at the top and bottom of the tube.
            assert!(c.iter().all(|q| (q[1].abs() - 40.0).abs() < 1e-9), "{c:?}");
            caps += 1;
            continue;
        };
        side += 1;
        for (q, uv) in c.iter().zip(points) {
            // y runs down the profile from its middle (the mesh is y up), and x around the axis is
            // the angle from the front (90°, toward the viewer), increasing to the viewer's right,
            // times the radius.
            assert!((uv.y + q[1]).abs() < 1e-9, "{q:?} {uv:?}");
            let turned = std::f64::consts::FRAC_PI_2 - q[2].atan2(q[0]);
            let around = (turned + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
            let seam = (uv.x.abs() - 40.0 * std::f64::consts::PI).abs() < step + 1e-6;
            assert!((uv.x - 40.0 * around).abs() < 1e-6 || seam, "{q:?} {uv:?}");
        }
        // No triangle reaches across the cut at the back: each spans one step around the axis.
        let (lo, hi) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
        assert!(hi - lo < step + 1e-6, "{points:?}");
    }
    assert_eq!((side, caps > 0), (2 * 64, true));
    // The cut is at the back: the side runs half a turn each way from the front.
    let xs = on.iter().flatten().flatten().map(|p| p.x);
    let (lo, hi) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
    assert!((lo + 40.0 * std::f64::consts::PI).abs() < 1e-6 && (hi - 40.0 * std::f64::consts::PI).abs() < 1e-6, "{lo} {hi}");
}

#[test]
fn a_revolved_sides_layout_matches_its_drawn_faces_at_every_turn() {
    let (p, r) = tube();
    for o in [
        Revolve { map_surface: true, angle: 90.0, ..Revolve::default() },
        Revolve { map_surface: true, angle: 250.0, offset: 15.0, edge: Edge::Right, rotation_x: -20.0, rotation_y: 35.0, ..Revolve::default() },
    ] {
        let (m, on) = revolve_mesh_surface(&p, FillRule::NonZero, r, o).unwrap();
        // The drawn faces' surface points, which the canvas places the graphics by.
        let drawn: Vec<Point> = crate::revolve(&p, r, o).unwrap().iter().filter_map(|f| f.surface.as_ref()).flat_map(|c| c.uv.clone()).collect();
        let near = |q: Point| drawn.iter().any(|d| d.distance(q) < 1e-6);
        let points: Vec<Point> = on.iter().flatten().flatten().copied().collect();
        assert!(!points.is_empty() && points.iter().all(|q| near(*q)), "{o:?}");
        assert!(drawn.iter().all(|d| points.iter().any(|q| q.distance(*d) < 1e-6)), "{o:?}");
        // The end sectors and the two cut sides carry no surface points.
        assert!(on.iter().any(Option::is_none) && on.len() == m.triangles.len());
    }
    // Without the layout, no triangle carries one.
    let (_, on) = revolve_mesh_surface(&p, FillRule::NonZero, r, Revolve::default()).unwrap();
    assert!(on.iter().all(Option::is_none));
}

#[test]
fn a_tapered_sides_triangles_are_the_drawn_faces_triangles() {
    // A cone's side: each face's ends lie at different distances from the axis, so its layout on
    // the surface is not one affine map, and each triangle must take the corners the canvas cuts
    // the face into.
    let mut b = BezPath::new();
    b.move_to((10.0, 10.0));
    b.line_to((40.0, 90.0));
    let p = PathData::from_bezpath(&b);
    let r = Rect::new(0.0, 10.0, 40.0, 90.0);
    let o = Revolve { map_surface: true, ..Revolve::default() };
    let key = |t: &[Point]| {
        let mut k: Vec<(i64, i64)> = t.iter().map(|q| ((q.x * 1e6).round() as i64, (q.y * 1e6).round() as i64)).collect();
        k.sort_unstable();
        k
    };
    let drawn: Vec<_> = crate::revolve(&p, r, o).unwrap().iter().filter_map(|f| f.surface.as_ref()).map(|c| key(&c.uv)).collect();
    assert!(drawn.iter().all(|k| k.len() == 3), "the canvas draws the smooth side in triangles");
    let (_, on) = revolve_mesh_surface(&p, FillRule::NonZero, r, o).unwrap();
    let side: Vec<_> = on.iter().flatten().collect();
    assert_eq!(side.len(), drawn.len());
    assert!(side.iter().all(|t| drawn.contains(&key(t.as_slice()))));
}

#[test]
fn only_the_longest_contour_of_a_revolved_profile_lies_on_the_side_surface() {
    let mut b = BezPath::new();
    b.move_to((40.0, 10.0));
    b.line_to((40.0, 90.0));
    b.move_to((20.0, 30.0));
    b.line_to((20.0, 50.0));
    let p = PathData::from_bezpath(&b);
    let r = Rect::new(0.0, 10.0, 40.0, 90.0);
    let (m, on) = revolve_mesh_surface(&p, FillRule::NonZero, r, Revolve { map_surface: true, cap_ends: false, ..Revolve::default() }).unwrap();
    for (t, points) in m.triangles.iter().zip(&on) {
        let radius = corners(&m, *t).iter().map(|q| q[0].hypot(q[2])).fold(0.0f64, f64::max);
        assert_eq!(points.is_some(), radius > 30.0, "{radius}");
    }
}

#[test]
fn a_revolved_surface_has_unit_normals_facing_out_whatever_its_length_and_direction() {
    // A wall 100 pt tall and one 0.5 pt tall, each drawn up and down the page.
    for (from, to) in [(0.0, 100.0), (100.0, 0.0), (0.0, 0.5), (0.5, 0.0)] {
        let mut b = BezPath::new();
        b.move_to((20.0, from));
        b.line_to((20.0, to));
        let p = PathData::from_bezpath(&b);
        let r = p.bounds().unwrap();
        let o = Revolve { offset: 10.0, edge: Edge::Left, segments: 64, ..Revolve::default() };
        let m = revolve_mesh(&p, FillRule::NonZero, r, o).unwrap();
        assert!(!m.is_empty(), "{from} to {to}: the revolve made no triangles");
        assert!(m.normals.iter().all(|n| (dot(*n, *n) - 1.0).abs() < 1e-9), "{from} to {to}: a normal is not unit length");
        assert!(closed(&m), "{from} to {to}");
        assert!(outward(&m), "{from} to {to}: a face turns inward");
    }
}

/// The radial direction of every triangle's normal at the wall `x` pt from the axis, each as the
/// sign of the normal's part along the way from the axis: 1 away from it, -1 toward it.
fn wall_directions(m: &Mesh, x: f64) -> Vec<f64> {
    m.triangles
        .iter()
        .filter_map(|t| {
            let [a, b, c] = corners(m, *t);
            let centre = [(a[0] + b[0] + c[0]) / 3.0, 0.0, (a[2] + b[2] + c[2]) / 3.0];
            let radius = centre[0].hypot(centre[2]);
            let n = m.normals[t[0] as usize];
            ((radius - x).abs() < 1.0 && n[1].abs() < 0.5).then(|| dot(n, centre).signum())
        })
        .collect()
}

#[test]
fn the_wall_of_a_counter_faces_into_the_hole() {
    // A square ring 20 pt thick, its axis at the outer square's left edge minus 10: the outer wall
    // reaches 110 pt from the axis, and the hole's far wall 90 pt from it.
    let ring = |hole_clockwise: bool, rule: FillRule| {
        let mut b = BezPath::new();
        square(&mut b, 0.0, 0.0, 100.0, false);
        square(&mut b, 20.0, 20.0, 60.0, hole_clockwise);
        let (p, r) = shape(&b);
        revolve_mesh(&p, rule, r, Revolve { offset: 10.0, segments: 32, ..Revolve::default() }).unwrap()
    };
    for (clockwise, rule) in [(true, FillRule::NonZero), (true, FillRule::EvenOdd), (false, FillRule::EvenOdd)] {
        let m = ring(clockwise, rule);
        let outer = wall_directions(&m, 110.0);
        assert!(!outer.is_empty() && outer.iter().all(|s| *s > 0.0), "outer wall, {clockwise} {rule:?}");
        let hole = wall_directions(&m, 90.0);
        assert!(!hole.is_empty() && hole.iter().all(|s| *s < 0.0), "the hole's far wall faces the axis, {clockwise} {rule:?}");
    }
    // Wound the same way under the nonzero rule, the inner square is not a hole.
    let m = ring(false, FillRule::NonZero);
    let inner = wall_directions(&m, 90.0);
    assert!(!inner.is_empty() && inner.iter().all(|s| *s > 0.0));
}

#[test]
fn an_edge_that_crosses_another_contour_faces_the_hole_only_where_it_walls_it() {
    // Two squares overlapping under the even-odd rule leave their overlap empty. The first square's
    // right edge (110 pt from the axis) and the second's left edge (50 pt from it) each wall the
    // overlap along part of their length and the outside along the rest.
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 100.0, false);
    square(&mut b, 40.0, 40.0, 100.0, false);
    let (p, r) = shape(&b);
    let m = revolve_mesh(&p, FillRule::EvenOdd, r, Revolve { offset: 10.0, segments: 32, ..Revolve::default() }).unwrap();
    for radius in [110.0, 50.0] {
        let walls = wall_directions(&m, radius);
        assert!(walls.iter().any(|s| *s > 0.0) && walls.iter().any(|s| *s < 0.0), "both ways at {radius}");
    }
    assert!(wall_directions(&m, 150.0).iter().all(|s| *s > 0.0), "the second square's far edge faces out");
}

#[test]
fn inflate_puffs_the_front_toward_the_viewer() {
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 100.0, false);
    let (p, r) = shape(&b);
    let m = inflate_meshes(&[(&p, FillRule::NonZero)], r, Inflate::default(), front()).last().unwrap().unwrap();
    assert!(!m.is_empty());
    let (lo, hi) = m.bounds().unwrap();
    assert!(hi[2] > 0.0 && lo[2] < hi[2], "the front puffs toward the viewer: {lo:?} {hi:?}");
}

#[test]
fn inflate_builds_each_solid_as_it_is_reached() {
    let (mut a, mut b) = (BezPath::new(), BezPath::new());
    square(&mut a, 0.0, 0.0, 100.0, false);
    square(&mut b, 120.0, 0.0, 100.0, false);
    let ((pa, _), (pb, _)) = (shape(&a), shape(&b));
    let built = || crate::solid::INFLATED.with(|n| n.get());
    let start = built();
    let mut meshes =
        inflate_meshes(&[(&pa, FillRule::NonZero), (&pb, FillRule::NonZero)], Rect::new(0.0, 0.0, 220.0, 100.0), Inflate::default(), front());
    assert_eq!(built(), start, "no solid is built before the first mesh is asked for");
    assert!(meshes.next().is_some_and(|m| m.is_ok_and(|m| !m.is_empty())));
    assert_eq!(built(), start + 1, "the second solid waits until it is reached");
    assert!(meshes.next().is_some_and(|m| m.is_ok_and(|m| !m.is_empty())));
    assert_eq!(built(), start + 2);
}

#[test]
fn an_empty_shape_makes_an_empty_mesh() {
    let p = PathData::new(vec![]);
    let m = plane_mesh(&p, FillRule::NonZero, Rect::new(0.0, 0.0, 10.0, 10.0), front()).unwrap();
    assert!(m.is_empty());
    assert_eq!(m.bounds(), None);
}

#[test]
fn join_numbers_each_parts_corners_after_the_last() {
    let mut b = BezPath::new();
    square(&mut b, 0.0, 0.0, 10.0, false);
    let (p, r) = shape(&b);
    let one = plane_mesh(&p, FillRule::NonZero, r, front()).unwrap();
    let both = Mesh::join([one.clone(), one.clone()]);
    assert_eq!(both.triangles.len(), 2 * one.triangles.len());
    assert!(both.triangles.iter().all(|t| t.iter().all(|i| (*i as usize) < both.positions.len())));
    assert_eq!(both.triangles[one.triangles.len()][0], one.triangles[0][0] + one.positions.len() as u32);
}

/// A mesh's positions and normals as bits, and its triangles.
type MeshBits = (Vec<[u64; 3]>, Vec<[u64; 3]>, Vec<[u32; 3]>);

/// The bits of every number in `m`, for comparing two meshes exactly.
fn bits(m: &Mesh) -> MeshBits {
    (m.positions.iter().map(|p| p.map(f64::to_bits)).collect(), m.normals.iter().map(|n| n.map(f64::to_bits)).collect(), m.triangles.clone())
}

/// A star of `points` points about (100, 100), with a square hole: curved and straight edges both.
fn star(points: usize) -> (PathData, Rect) {
    let mut b = BezPath::new();
    for k in 0..2 * points {
        let (s, c) = (k as f64 * std::f64::consts::PI / points as f64).sin_cos();
        let r = if k % 2 == 0 { 90.0 } else { 45.0 };
        let p = (100.0 + r * c, 100.0 + r * s);
        if k == 0 { b.move_to(p) } else { b.curve_to((p.0 + 3.0, p.1), (p.0, p.1 + 3.0), p) }
    }
    b.close_path();
    square(&mut b, 90.0, 90.0, 20.0, true);
    shape(&b)
}

/// The views the solids are compared in: straight on, turned, with perspective, and with colored
/// lights (Inflate's smooth triangles).
fn views() -> Vec<View> {
    let turned = View { rotation_x: -25.0, rotation_y: 35.0, rotation_z: 10.0, ..View::default() };
    vec![front(), turned, View { perspective: 60.0, ..turned }, View { light_color: [1.0, 0.5, 0.25], ..turned }]
}

/// Extrude & Bevel built once for its facets and its mesh gives what [`crate::extrude`] and
/// [`extrude_mesh`] give apart, to the bit, errors included.
#[test]
fn an_extruded_solid_and_its_mesh_match_the_two_built_apart() {
    let bevel = Bevel { shape: BevelShape::Rounded, width: 6.0, height: 5.0, ..Bevel::default() };
    let options = [
        Extrude::default(),
        Extrude { depth: 0.0, ..Extrude::default() },
        Extrude { cap: false, ..Extrude::default() },
        Extrude { hidden: true, ..Extrude::default() },
        Extrude { twist: 90.0, taper: 0.6, ..Extrude::default() },
        Extrude { bevel: Some(bevel), ..Extrude::default() },
        Extrude { bevel: Some(Bevel { both_sides: true, inside: true, ..bevel }), twist: 45.0, ..Extrude::default() },
    ];
    let (p, r) = star(7);
    for view in views() {
        for o in options {
            let meshed = extrude_meshed(&p, FillRule::NonZero, r, o, view).unwrap();
            let facets = crate::extrude(&p, FillRule::NonZero, r, o, view).unwrap();
            assert_eq!(format!("{:?}", meshed.facets), format!("{facets:?}"), "{o:?} {view:?}");
            let apart = extrude_mesh(&p, FillRule::NonZero, r, o, view).unwrap();
            assert!(!apart.is_empty());
            assert_eq!(bits(&meshed.mesh().unwrap()), bits(&apart), "{o:?} {view:?}");
        }
    }
    // A solid that can't be built gives the same error.
    let flat = Rect::new(0.0, 0.0, f64::INFINITY, 10.0);
    let o = Extrude::default();
    let error = crate::extrude(&p, FillRule::NonZero, flat, o, front()).err();
    assert!(error.is_some());
    assert_eq!(extrude_meshed(&p, FillRule::NonZero, flat, o, front()).err(), error);
}

/// Inflate's solids built once for their facets and meshes give what [`crate::inflate_each`] and
/// [`inflate_meshes`] give apart, to the bit: several puffs under one eye, both sides, open
/// backs, and twisted sides cut finer than room is left for.
#[test]
fn inflated_solids_and_their_meshes_match_the_two_built_apart() {
    let (star, _) = star(5);
    let mut b = BezPath::new();
    square(&mut b, 200.0, 20.0, 60.0, false);
    let (box_, _) = shape(&b);
    let ring: Vec<vectorcraft_geom::Point> = (0..600)
        .map(|k| {
            let (s, c) = (k as f64 * std::f64::consts::TAU / 600.0).sin_cos();
            vectorcraft_geom::Point::new(100.0 + 90.0 * c, 100.0 + 90.0 * s)
        })
        .collect();
    let ring = PathData::single(vectorcraft_geom::SubPath::polyline(&ring, true));
    let sets = [vec![(&star, FillRule::NonZero), (&box_, FillRule::NonZero), (&star, FillRule::EvenOdd)], vec![(&ring, FillRule::NonZero)]];
    let bounds = Rect::new(0.0, 0.0, 280.0, 200.0);
    let options = [
        Inflate::default(),
        Inflate { depth: 30.0, both_sides: true, ..Inflate::default() },
        Inflate { depth: 20.0, cap: false, volume: 80.0, ..Inflate::default() },
        Inflate { depth: 40.0, twist: 360.0, taper: 0.7, both_sides: true, ..Inflate::default() },
        // Thin and barely twisted: the star's sides want more pieces than room is left after its
        // front and back, and the mesh, which keeps the faces turned away, has less room left.
        Inflate { depth: 0.1, twist: 0.1, both_sides: true, ..Inflate::default() },
    ];
    for shapes in &sets {
        for view in views() {
            for o in options {
                let meshed: Vec<_> = inflate_meshed(shapes, bounds, o, view).collect();
                let facets: Vec<_> = crate::inflate_each(shapes, bounds, o, view).collect();
                let apart: Vec<_> = inflate_meshes(shapes, bounds, o, view).collect();
                assert_eq!(meshed.len(), shapes.len());
                for ((m, f), a) in meshed.iter().zip(&facets).zip(&apart) {
                    let (m, f, a) = (m.as_ref().unwrap(), f.as_ref().unwrap(), a.as_ref().unwrap());
                    assert_eq!(format!("{:?}", m.facets), format!("{f:?}"), "{o:?} {view:?}");
                    assert!(!a.is_empty());
                    assert_eq!(bits(&m.mesh().unwrap()), bits(a), "{o:?} {view:?}");
                }
            }
        }
    }
}

/// One build gives both the facets and the mesh: each solid is built once.
#[test]
fn a_meshed_solid_is_built_once() {
    let (p, r) = star(5);
    let extruded = || crate::solid::EXTRUDED.with(|n| n.get());
    let start = extruded();
    let solid = extrude_meshed(&p, FillRule::NonZero, r, Extrude::default(), front()).unwrap();
    assert!(!solid.facets.is_empty() && solid.mesh().is_ok_and(|m| !m.is_empty()));
    assert_eq!(extruded(), start + 1);
    let inflated = || crate::solid::INFLATED.with(|n| n.get());
    let start = inflated();
    let mut b = BezPath::new();
    square(&mut b, 200.0, 20.0, 60.0, false);
    let (q, _) = shape(&b);
    for solid in inflate_meshed(&[(&p, FillRule::NonZero), (&q, FillRule::NonZero)], Rect::new(0.0, 0.0, 280.0, 200.0), Inflate::default(), front()) {
        assert!(solid.is_ok_and(|s| !s.facets.is_empty() && s.mesh().is_ok_and(|m| !m.is_empty())));
    }
    assert_eq!(inflated(), start + 2);
}
