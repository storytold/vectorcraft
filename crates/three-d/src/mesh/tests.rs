use std::collections::HashMap;

use vectorcraft_geom::{BezPath, FillRule, PathData, Rect};

use super::*;
use crate::{Bevel, Edge};

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
fn a_partial_revolve_caps_its_ends_with_sectors() {
    let mut b = BezPath::new();
    b.move_to((20.0, 0.0));
    b.line_to((20.0, 100.0));
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    let o = Revolve { angle: 90.0, offset: 10.0, segments: 64, ..Revolve::default() };
    let m = revolve_mesh(&p, FillRule::NonZero, r, o).unwrap();
    let exact = 0.25 * (2.0 * std::f64::consts::PI * 10.0 * 100.0 + 2.0 * std::f64::consts::PI * 100.0);
    assert!((area(&m) - exact).abs() / exact < 0.01, "{} vs {exact}", area(&m));
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
