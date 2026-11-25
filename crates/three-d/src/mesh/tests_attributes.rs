use std::collections::HashMap;

use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect};

use super::*;
use crate::{Bevel, BevelProfile, BevelShape, Edge};

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> (PathData, Rect) {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + w, y));
    b.line_to((x + w, y + h));
    b.line_to((x, y + h));
    b.close_path();
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    (p, r)
}

/// A rectangle with a square hole: the hole's wall is a contour of its own.
fn holed() -> (PathData, Rect) {
    let mut b = BezPath::new();
    for (x, y, s, clockwise) in [(0.0, 0.0, 100.0, false), (40.0, 40.0, 20.0, true)] {
        b.move_to((x, y));
        let corners = if clockwise { [(x, y + s), (x + s, y + s), (x + s, y)] } else { [(x + s, y), (x + s, y + s), (x, y + s)] };
        for c in corners {
            b.line_to(c);
        }
        b.close_path();
    }
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    (p, r)
}

/// A vertical line from y 0 to 100, 20 pt from the left edge of its bounds' axis.
fn line_profile() -> (PathData, Rect) {
    let mut b = BezPath::new();
    b.move_to((20.0, 0.0));
    b.line_to((20.0, 100.0));
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    (p, r)
}

fn front() -> View {
    View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, perspective: 0.0, ..View::default() }
}

fn turned() -> View {
    View { rotation_x: -25.0, rotation_y: 35.0, rotation_z: 10.0, perspective: 0.0, ..View::default() }
}

fn bevel(shape: BevelShape) -> Bevel {
    Bevel { shape, width: 6.0, height: 6.0, ..Bevel::default() }
}

fn beveled(b: Bevel) -> Extrude {
    Extrude { bevel: Some(b), ..Extrude::default() }
}

fn custom() -> BevelShape {
    BevelShape::Custom(BevelProfile::new(&[(0.0, 0.0), (0.5, 0.2), (0.8, 0.5), (1.0, 1.0)]).unwrap())
}

fn length(v: [f64; 3]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

fn geometric(m: &Mesh, t: [u32; 3]) -> [f64; 3] {
    let [a, b, c] = t.map(|i| m.positions[i as usize]);
    let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = length(n);
    n.map(|x| x / l)
}

/// Every list of `m` matches, every texture coordinate is a number from 0 to 1, and every normal
/// is a unit vector.
fn check(m: &Mesh, what: &str) {
    assert!(m.is_consistent(), "{what}");
    assert!(m.uvs.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{what}");
    assert!(m.normals.iter().all(|n| (length(*n) - 1.0).abs() < 1e-9), "{what}");
    assert!(m.positions.iter().flatten().all(|v| v.is_finite()), "{what}");
}

fn parts(m: &Mesh) -> Vec<MeshPart> {
    let mut out: Vec<MeshPart> = vec![];
    for p in &m.parts {
        if !out.contains(p) {
            out.push(*p);
        }
    }
    out
}

/// The meshes of every effect and many options.
fn meshes() -> Vec<(String, Mesh)> {
    let (square, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    let (hole, hr) = holed();
    let mut out = vec![];
    let shapes = [BevelShape::Classic, BevelShape::Rounded, BevelShape::Cove, BevelShape::Step, BevelShape::Ogee, custom()];
    for view in [front(), turned(), View { perspective: 60.0, ..turned() }] {
        for shape in shapes {
            for b in [
                bevel(shape),
                Bevel { inside: true, ..bevel(shape) },
                Bevel { both_sides: true, ..bevel(shape) },
                Bevel { repeat: 3, space: 0.2, ..bevel(shape) },
            ] {
                let o = Extrude { bevel: Some(b), ..Extrude::default() };
                out.push((format!("{shape:?} {b:?} {view:?}"), extrude_mesh(&square, FillRule::NonZero, r, o, view).unwrap()));
            }
        }
        for o in [
            Extrude::default(),
            Extrude { twist: 90.0, taper: 0.5, bevel: Some(bevel(BevelShape::Rounded)), ..Extrude::default() },
            Extrude { depth: 0.0, ..Extrude::default() },
            Extrude { cap: false, ..Extrude::default() },
        ] {
            out.push((format!("{o:?} {view:?}"), extrude_mesh(&hole, FillRule::NonZero, hr, o, view).unwrap()));
        }
        out.push((format!("plane {view:?}"), plane_mesh(&hole, FillRule::NonZero, hr, view).unwrap()));
        let deep = Inflate { depth: 10.0, ..Inflate::default() };
        for o in [Inflate::default(), Inflate { both_sides: true, twist: 30.0, ..deep }, Inflate { cap: false, ..deep }] {
            for (k, m) in inflate_meshes(&[(&hole, FillRule::NonZero)], hr, o, view).enumerate() {
                out.push((format!("inflate {k} {o:?} {view:?}"), m.unwrap()));
            }
        }
    }
    let (line, lr) = line_profile();
    for angle in [360.0, 90.0] {
        let o = Revolve { angle, offset: 10.0, edge: Edge::Left, rotation_x: -20.0, rotation_y: 30.0, ..Revolve::default() };
        out.push((format!("revolve line {angle}"), revolve_mesh(&line, FillRule::NonZero, lr, o).unwrap()));
        out.push((format!("revolve holed {angle}"), revolve_mesh(&hole, FillRule::NonZero, hr, o).unwrap()));
    }
    out
}

#[test]
fn every_corner_has_texture_coordinates_and_every_triangle_a_part() {
    for (what, m) in meshes() {
        assert!(!m.is_empty(), "{what}");
        check(&m, &what);
        assert_eq!(m.uvs.len(), m.positions.len(), "{what}");
        assert_eq!(m.parts.len(), m.triangles.len(), "{what}");
    }
}

#[test]
fn each_triangle_names_the_part_of_the_solid_it_comes_from() {
    let (p, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    let both = Extrude { bevel: Some(Bevel { both_sides: true, ..bevel(BevelShape::Rounded) }), ..Extrude::default() };
    let m = extrude_mesh(&p, FillRule::NonZero, r, both, front()).unwrap();
    let mut found = parts(&m);
    found.sort_by_key(|p| *p as u8);
    assert_eq!(found, [MeshPart::Front, MeshPart::Back, MeshPart::Side, MeshPart::FrontBevel, MeshPart::BackBevel]);
    // Straight on, the front cap faces the viewer and the back cap away; the walls face sideways.
    for (t, part) in m.triangles.iter().zip(&m.parts) {
        let n = geometric(&m, *t);
        match part {
            MeshPart::Front => assert!(n[2] > 1.0 - 1e-9),
            MeshPart::Back => assert!(n[2] < -1.0 + 1e-9),
            MeshPart::Side => assert!(n[2].abs() < 1e-9),
            MeshPart::FrontBevel => assert!(n[2] > 0.0),
            MeshPart::BackBevel => assert!(n[2] < 0.0),
            other => panic!("{other:?}"),
        }
    }
    assert!(plane_mesh(&p, FillRule::NonZero, r, turned()).unwrap().parts.iter().all(|p| *p == MeshPart::Front));
    let flat = extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: 0.0, ..Extrude::default() }, front()).unwrap();
    assert!(flat.parts.iter().all(|p| *p == MeshPart::Front));
    let deep = Inflate { depth: 10.0, ..Inflate::default() };
    let puffed = inflate_meshes(&[(&p, FillRule::NonZero)], r, deep, front()).next().unwrap().unwrap();
    let mut found = parts(&puffed);
    found.sort_by_key(|p| *p as u8);
    assert_eq!(found, [MeshPart::Front, MeshPart::Back, MeshPart::Side]);
    let (line, lr) = line_profile();
    let o = Revolve { angle: 90.0, offset: 10.0, ..Revolve::default() };
    let mut found = parts(&revolve_mesh(&line, FillRule::NonZero, lr, o).unwrap());
    found.sort_by_key(|p| *p as u8);
    assert_eq!(found, [MeshPart::Side, MeshPart::Cap, MeshPart::Cut]);
}

#[test]
fn caps_take_their_art_points_and_the_side_runs_around_the_outline_and_front_to_back() {
    let (p, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    let m = extrude_mesh(&p, FillRule::NonZero, r, Extrude::default(), front()).unwrap();
    let depth = Extrude::default().depth;
    let (mut seams, mut ends) = (HashMap::<[u64; 3], Vec<f64>>::new(), [false; 2]);
    for (t, part) in m.triangles.iter().zip(&m.parts) {
        for &i in t {
            let (q, uv) = (m.positions[i as usize], m.uvs[i as usize]);
            match part {
                // The art at (x, y) lies at (x - 50, 30 - y) in the mesh: the back is not mirrored.
                MeshPart::Front | MeshPart::Back => {
                    assert!((uv[0] - (q[0] + 50.0) / 100.0).abs() < 1e-9 && (uv[1] - (30.0 - q[1]) / 60.0).abs() < 1e-9, "{q:?} {uv:?}");
                }
                MeshPart::Side => {
                    seams.entry(q.map(f64::to_bits)).or_default().push(uv[0]);
                    // v runs from the front cap's edge to the back cap's.
                    let v = if q[2] > 0.0 { 0.0 } else { 1.0 };
                    assert!((q[2].abs() - depth / 2.0).abs() < 1e-9 && (uv[1] - v).abs() < 1e-9, "{q:?} {uv:?}");
                    ends[usize::from(q[2] < 0.0)] = true;
                }
                other => panic!("{other:?}"),
            }
        }
    }
    assert_eq!(ends, [true, true]);
    // The outline's first point carries both ends of the side's u: the seam's corners are kept apart.
    assert!(seams.values().any(|u| u.contains(&0.0) && u.contains(&1.0)));
    // Along the 320 pt outline each corner of the rectangle lies 100, 160 and 260 pt from the first.
    let mut along: Vec<f64> = seams.values().flatten().copied().collect();
    along.sort_by(f64::total_cmp);
    along.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert_eq!(along.len(), 5, "{along:?}");
    for (u, expected) in along.iter().zip([0.0, 100.0, 160.0, 260.0, 320.0]) {
        assert!((u - expected / 320.0).abs() < 1e-9, "{along:?}");
    }
}

#[test]
fn the_parts_of_one_object_place_their_caps_and_planes_in_its_shared_bounds() {
    // A small part inside the 100 by 60 pt bounds of its object: its front, back and plane corners
    // take their art points in the object's bounds, as a larger part's do, not in its own.
    let (big, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    let (small, _) = rectangle(10.0, 10.0, 20.0, 20.0);
    let on_shared = |m: &Mesh, what: &str| {
        let mut seen = 0;
        for (t, part) in m.triangles.iter().zip(&m.parts) {
            if !matches!(part, MeshPart::Front | MeshPart::Back) {
                continue;
            }
            for &i in t {
                let (q, uv) = (m.positions[i as usize], m.uvs[i as usize]);
                // The art at (x, y) lies at (x - 50, 30 - y) in the mesh.
                assert!((uv[0] - (q[0] + 50.0) / 100.0).abs() < 1e-9 && (uv[1] - (30.0 - q[1]) / 60.0).abs() < 1e-9, "{what} {q:?} {uv:?}");
                seen += 1;
            }
        }
        assert!(seen > 0, "{what}");
    };
    on_shared(&extrude_mesh(&small, FillRule::NonZero, r, Extrude::default(), front()).unwrap(), "extrude");
    on_shared(&plane_mesh(&small, FillRule::NonZero, r, front()).unwrap(), "plane");
    let deep = Inflate { depth: 10.0, ..Inflate::default() };
    for (k, m) in inflate_meshes(&[(&big, FillRule::NonZero), (&small, FillRule::NonZero)], r, deep, front()).enumerate() {
        on_shared(&m.unwrap(), &format!("inflate {k}"));
    }
}

#[test]
fn revolves_side_runs_around_the_sweep_and_along_the_profile() {
    let (line, lr) = line_profile();
    let o = Revolve { offset: 10.0, segments: 16, ..Revolve::default() };
    let m = revolve_mesh(&line, FillRule::NonZero, lr, o).unwrap();
    let mut around = HashMap::<[i64; 3], Vec<f64>>::new();
    for (t, part) in m.triangles.iter().zip(&m.parts) {
        for &i in t {
            let (q, uv) = (m.positions[i as usize], m.uvs[i as usize]);
            if *part == MeshPart::Side {
                // The sweep's end lands on its start within rounding.
                around.entry(q.map(|v| (v * 1e6).round() as i64)).or_default().push(uv[0]);
                // The profile runs from y 0 (the mesh's top, at 50) to y 100 (at -50).
                assert!((uv[1] - (50.0 - q[1]) / 100.0).abs() < 1e-9, "{q:?} {uv:?}");
            } else {
                assert_eq!(*part, MeshPart::Cap);
            }
        }
    }
    assert!(around.values().any(|u| u.contains(&0.0) && u.contains(&1.0)), "the sweep's seam is kept apart");
}

/// The corners of the bottom edge's front bevel of `m` (an extruded rectangle seen straight on),
/// by position, with their normals.
fn bottom_bevel(m: &Mesh) -> HashMap<[u64; 3], Vec<[f64; 3]>> {
    let mut out = HashMap::<[u64; 3], Vec<[f64; 3]>>::new();
    for (t, part) in m.triangles.iter().zip(&m.parts) {
        let n = geometric(m, *t);
        if *part != MeshPart::FrontBevel || n[0].abs() > 1e-9 || n[1] >= 0.0 {
            continue;
        }
        for &i in t {
            out.entry(m.positions[i as usize].map(f64::to_bits)).or_default().push(m.normals[i as usize]);
        }
    }
    out
}

/// The angle between `a` and `b` in degrees, from their cross and dot products, so that two equal
/// vectors give 0 exactly.
fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
    let cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    length(cross).atan2(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).to_degrees()
}

#[test]
fn a_rounded_or_custom_bevels_normals_turn_smoothly_across_its_bands() {
    let (p, r) = rectangle(0.0, 0.0, 200.0, 100.0);
    for shape in [BevelShape::Rounded, custom(), BevelShape::Ogee] {
        let o = Extrude { bevel: Some(bevel(shape)), ..Extrude::default() };
        let m = extrude_mesh(&p, FillRule::NonZero, r, o, front()).unwrap();
        let corners = bottom_bevel(&m);
        let mut normals: Vec<[f64; 3]> = vec![];
        for (at, list) in &corners {
            // Every band that meets at a corner gives it the same normal.
            assert!(list.iter().all(|n| angle(*n, list[0]) < 1e-9), "{shape:?} {at:?} {list:?}");
            assert!(list.iter().all(|n| (length(*n) - 1.0).abs() < 1e-12));
            if !normals.iter().any(|n| angle(*n, list[0]) < 1e-9) {
                normals.push(list[0]);
            }
        }
        // The normals turn from the front toward the bottom in steps smaller than the profile's.
        normals.sort_by(|a, b| b[2].total_cmp(&a[2]));
        assert!(normals.len() >= 3, "{shape:?} {normals:?}");
        assert!(normals.windows(2).all(|w| angle(w[0], w[1]) < 45.0), "{shape:?} {normals:?}");
        // The vector facets keep their flat shading.
        let facets = crate::extrude(&p, FillRule::NonZero, r, o, front()).unwrap();
        let built = crate::extrude_meshed(&p, FillRule::NonZero, r, o, front()).unwrap();
        assert_eq!(facets, built.facets);
    }
}

#[test]
fn a_twisted_rounded_bevel_keeps_its_normals_smoothed_on_every_piece_of_a_cut_edge() {
    let (p, r) = rectangle(0.0, 0.0, 200.0, 100.0);
    // A twist small enough that the bevel's bands do not fold over one another.
    let o = Extrude { twist: 10.0, bevel: Some(bevel(BevelShape::Rounded)), ..Extrude::default() };
    let m = extrude_mesh(&p, FillRule::NonZero, r, o, front()).unwrap();
    let mut at = HashMap::<[u64; 3], Vec<[f64; 3]>>::new();
    for (t, part) in m.triangles.iter().zip(&m.parts) {
        if *part == MeshPart::FrontBevel {
            for &i in t {
                let normal = m.normals[i as usize];
                assert!((length(normal) - 1.0).abs() < 1e-12);
                at.entry(m.positions[i as usize].map(f64::to_bits)).or_default().push(normal);
            }
        }
    }
    // The twist cuts each edge into pieces. Where two pieces of one band meet on a row between two
    // bands, each band's two pieces would give four normals; smoothed, the two bands agree on each
    // piece's normal, which leaves two.
    let mut most = 0;
    for (q, list) in &at {
        let mut distinct: Vec<[f64; 3]> = vec![];
        for n in list {
            if !distinct.iter().any(|d| angle(*d, *n) < 1e-9) {
                distinct.push(*n);
            }
        }
        assert!(distinct.len() <= 2, "{q:?} {distinct:?}");
        most = most.max(list.len());
    }
    // Some corners lie where the pieces of two bands meet.
    assert!(most >= 4, "{most}");
}

#[test]
fn a_steps_sharp_turns_and_a_classic_bevel_stay_hard_edges() {
    let (p, r) = rectangle(0.0, 0.0, 200.0, 100.0);
    let step = extrude_mesh(&p, FillRule::NonZero, r, Extrude { bevel: Some(bevel(BevelShape::Step)), ..Extrude::default() }, front()).unwrap();
    for (t, part) in step.triangles.iter().zip(&step.parts) {
        if *part == MeshPart::FrontBevel {
            let n = geometric(&step, *t);
            for &i in t {
                assert!(angle(step.normals[i as usize], n) < 1e-6, "a Step's flat bands keep their own normals");
            }
        }
    }
    let classic = extrude_mesh(&p, FillRule::NonZero, r, Extrude { bevel: Some(bevel(BevelShape::Classic)), ..Extrude::default() }, front()).unwrap();
    for t in &classic.triangles {
        let n = geometric(&classic, *t);
        for &i in t {
            assert!(angle(classic.normals[i as usize], n) < 1e-6);
        }
    }
}

#[test]
fn inflates_puffed_front_takes_the_surfaces_own_normals() {
    let (p, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    let m = inflate_meshes(&[(&p, FillRule::NonZero)], r, Inflate::default(), front()).next().unwrap().unwrap();
    check(&m, "inflate");
    // Each point of the front has one normal, whichever pieces meet there.
    let mut at = HashMap::<[u64; 3], Vec<[f64; 3]>>::new();
    for (t, part) in m.triangles.iter().zip(&m.parts) {
        if *part == MeshPart::Front {
            for &i in t {
                at.entry(m.positions[i as usize].map(f64::to_bits)).or_default().push(m.normals[i as usize]);
            }
        }
    }
    assert!(at.values().filter(|list| list.len() > 1).count() > 10);
    for list in at.values() {
        assert!(list.iter().all(|n| angle(*n, list[0]) < 1e-9), "{list:?}");
    }
}

/// Whether `p` lies inside the polygon `ring` (even-odd), or within `d` of one of its edges.
fn inside_or_near(rings: &[Vec<Point>], p: Point, d: f64) -> bool {
    let mut odd = false;
    for ring in rings {
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()) {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let l2 = dx * dx + dy * dy;
            let t = if l2 < 1e-18 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0) };
            if p.distance(Point::new(a.x + dx * t, a.y + dy * t)) <= d {
                return true;
            }
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                odd = !odd;
            }
        }
    }
    odd
}

/// Every corner of `m` seen from `origin` without perspective lies within `d` of some polygon of
/// `drawn`, and every corner of `drawn` within `d` of some triangle of `m`: the mesh's outline
/// and the vector facets' differ by no more than `d`.
fn outlines_match(m: &Mesh, drawn: &[Vec<Vec<Point>>], origin: Point, d: f64, what: &str) {
    let page = |q: [f64; 3]| Point::new(origin.x + q[0], origin.y - q[1]);
    let triangles: Vec<Vec<Point>> = m.triangles.iter().map(|t| t.iter().map(|&i| page(m.positions[i as usize])).collect()).collect();
    for q in &m.positions {
        let p = page(*q);
        assert!(drawn.iter().any(|f| inside_or_near(f, p, d)), "{what}: mesh corner {p:?}");
    }
    for p in drawn.iter().flatten().flatten() {
        assert!(triangles.iter().any(|t| inside_or_near(std::slice::from_ref(t), *p, d)), "{what}: facet corner {p:?}");
    }
}

/// The largest distance between the outline of a mesh and that of the vector facets of the same
/// solid, in points.
const OUTLINE_TOLERANCE: f64 = 0.05;

#[test]
fn the_meshs_outline_stays_within_a_twentieth_of_a_point_of_the_vector_facets() {
    let (p, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    let (hole, hr) = holed();
    for shape in [BevelShape::Classic, BevelShape::Rounded, BevelShape::Step, custom()] {
        for o in [
            Extrude { bevel: Some(bevel(shape)), ..Extrude::default() },
            Extrude { bevel: Some(Bevel { inside: true, both_sides: true, ..bevel(shape) }), ..Extrude::default() },
        ] {
            for (path, bounds) in [(&p, r), (&hole, hr)] {
                let m = extrude_mesh(path, FillRule::NonZero, bounds, o, turned()).unwrap();
                let facets = crate::extrude(path, FillRule::NonZero, bounds, o, turned()).unwrap();
                let drawn: Vec<Vec<Vec<Point>>> = facets.into_iter().map(|f| f.contours).collect();
                outlines_match(&m, &drawn, bounds.center(), OUTLINE_TOLERANCE, &format!("{shape:?} {o:?}"));
            }
        }
    }
    let (line, lr) = line_profile();
    for angle in [360.0, 120.0] {
        let o = Revolve { angle, offset: 10.0, rotation_x: -20.0, rotation_y: 30.0, ..Revolve::default() };
        let m = revolve_mesh(&line, FillRule::NonZero, lr, o).unwrap();
        let drawn: Vec<Vec<Vec<Point>>> = crate::revolve(&line, lr, o).unwrap().into_iter().map(|f| vec![f.points]).collect();
        let origin = crate::revolve_projection(lr, o.sanitized()).origin;
        outlines_match(&m, &drawn, origin, OUTLINE_TOLERANCE, &format!("revolve {angle}"));
    }
}

#[test]
fn hostile_input_gives_an_error_or_a_consistent_mesh() {
    let (p, r) = rectangle(0.0, 0.0, 100.0, 60.0);
    // A closed outline with no area.
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    b.line_to((50.0, 0.0));
    b.line_to((100.0, 0.0));
    b.close_path();
    let flat = PathData::from_bezpath(&b);
    let fr = Rect::new(0.0, 0.0, 100.0, 0.0);
    for m in [
        extrude_mesh(&flat, FillRule::NonZero, fr, Extrude::default(), turned()),
        extrude_mesh(&flat, FillRule::NonZero, fr, Extrude { bevel: Some(bevel(BevelShape::Rounded)), ..Extrude::default() }, turned()),
        plane_mesh(&flat, FillRule::NonZero, fr, turned()),
        extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: 0.0, ..Extrude::default() }, turned()),
        extrude_mesh(&p, FillRule::NonZero, r, Extrude { depth: f64::NAN, twist: f64::NAN, ..Extrude::default() }, turned()),
        extrude_mesh(&p, FillRule::NonZero, r, beveled(Bevel { width: f64::NAN, height: f64::INFINITY, ..bevel(custom()) }), turned()),
        extrude_mesh(&p, FillRule::NonZero, r, beveled(Bevel { repeat: usize::MAX, ..bevel(BevelShape::Rounded) }), turned()),
        revolve_mesh(&p, FillRule::NonZero, r, Revolve { segments: usize::MAX, ..Revolve::default() }),
        revolve_mesh(&flat, FillRule::NonZero, fr, Revolve::default()),
    ]
    .into_iter()
    .flatten()
    {
        check(&m, "hostile");
    }
    let nan = Rect::new(f64::NAN, 0.0, 100.0, 60.0);
    assert!(extrude_mesh(&p, FillRule::NonZero, nan, Extrude::default(), turned()).is_err());
    assert!(plane_mesh(&p, FillRule::NonZero, nan, turned()).is_err());
    assert!(revolve_mesh(&p, FillRule::NonZero, nan, Revolve::default()).is_err());
}
