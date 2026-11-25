use crate::*;
use vectorcraft_geom::Shape;

/// Swept with Cap on, unshaded and seen from the front.
fn options() -> Revolve {
    Revolve { cap_ends: true, shade: false, ..Default::default() }
}

fn profile(data: &str) -> PathData {
    PathData::from_bezpath(&BezPath::from_svg(data).unwrap())
}

fn area(face: &Face) -> f64 {
    face.path().to_bezpath().area().abs()
}

/// The faces of the cut sides among `faces`: the flat faces in a plane through the axis, which these
/// tests keep upright. The strips of the swept side lie off the axis, and the endpoint caps across it.
fn cut_sides(faces: &[Face]) -> Vec<&Face> {
    faces.iter().filter(|f| !f.cap && f.plane.is_some_and(|p| p.offset.abs() < 1e-6 && p.normal[1].abs() < 1e-9)).collect()
}

#[test]
fn a_half_turn_closes_both_cut_sides_with_the_region_between_the_profile_and_the_axis() {
    let path = profile("M40 10 L40 90");
    let bounds = Rect::new(0.0, 10.0, 40.0, 90.0);
    // Seen from the front, a half turn's two cut sides both face the viewer: the profile's plane
    // to the right of the axis and to its left.
    let faces = revolve(&path, bounds, Revolve { angle: 180.0, ..options() }).unwrap();
    let cuts = cut_sides(&faces);
    assert_eq!(cuts.len(), 2);
    for cut in &cuts {
        assert!((area(cut) - 40.0 * 80.0).abs() < 1e-6, "{}", area(cut));
        assert!(cut.points.iter().all(|p| (10.0 - 1e-9..=90.0 + 1e-9).contains(&p.y)));
    }
    assert!(cuts.iter().any(|f| f.points.iter().all(|p| p.x >= -1e-9)));
    assert!(cuts.iter().any(|f| f.points.iter().all(|p| p.x <= 1e-9)));
    assert!(faces.windows(2).all(|pair| pair[0].depth <= pair[1].depth));
}

#[test]
fn cap_off_full_turns_and_no_turn_leave_the_cut_sides_open() {
    let path = profile("M40 10 L40 90");
    let bounds = Rect::new(0.0, 10.0, 40.0, 90.0);
    for (o, what) in [
        (Revolve { angle: 180.0, cap_ends: false, ..options() }, "hollow"),
        (Revolve { angle: 360.0, rotation_x: 90.0, ..options() }, "a full turn"),
        (Revolve { angle: 0.0, ..options() }, "no turn"),
    ] {
        let faces = revolve(&path, bounds, o).unwrap();
        assert!(cut_sides(&faces).is_empty(), "{what}");
    }
}

#[test]
fn a_closed_profile_closes_its_cut_sides_with_its_area_and_leaves_its_hole_open() {
    // A square ring away from the axis, its hole wound the other way.
    let path = profile("M20 10 L60 10 L60 50 L20 50 Z M30 20 L30 40 L50 40 L50 20 Z");
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 180.0, offset: 10.0, rotation_y: 20.0, ..options() };
    let faces = revolve(&path, bounds, o).unwrap();
    let cuts = cut_sides(&faces);
    assert_eq!(cuts.len(), 2);
    for cut in &cuts {
        let outline = cut.outline.as_ref().unwrap();
        assert_eq!(outline.subpaths.len(), 2, "the outline and its hole");
        // Turned 20° about the vertical axis, each side is foreshortened by cos 20°.
        let expected = (40.0 * 40.0 - 20.0 * 20.0) * 20f64.to_radians().cos();
        assert!((area(cut) - expected).abs() < 1e-6, "{} != {expected}", area(cut));
    }
    // Visibility measures the face with its hole open.
    assert!(!visible_faces(&faces).unwrap().is_empty());
}

#[test]
fn an_even_odd_hole_wound_with_its_outline_stays_open_in_the_drawn_cut_sides() {
    // The hole runs the same way round as the outline, so only the even-odd rule leaves it open.
    let path = profile("M20 10 L60 10 L60 50 L20 50 Z M30 20 L50 20 L50 40 L30 40 Z");
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 180.0, offset: 10.0, ..options() };
    let faces = revolve(&path, bounds, Revolve { fill_rule: FillRule::EvenOdd, ..o }).unwrap();
    let cuts = cut_sides(&faces);
    assert_eq!(cuts.len(), 2);
    for cut in cuts {
        // The drawn path is filled with the nonzero rule; the hole is in the middle of the side.
        let filled = cut.path().to_bezpath();
        assert!(!filled.contains(filled.bounding_box().center()));
        assert!((area(cut) - (40.0 * 40.0 - 20.0 * 20.0)).abs() < 1e-6, "{}", area(cut));
    }
    // Under the nonzero rule the inner contour fills too: each side is the whole outer square.
    let faces = revolve(&path, bounds, Revolve { fill_rule: FillRule::NonZero, ..o }).unwrap();
    let areas: Vec<f64> = cut_sides(&faces).iter().map(|f| area(f)).collect();
    assert_eq!(areas.len(), 2);
    assert!(areas.iter().all(|a| (a - 1600.0).abs() < 1e-6), "{areas:?}");
}

#[test]
fn a_bow_tie_whose_loops_cancel_out_closes_its_cut_sides_with_both_loops() {
    // Two triangles of 400 pt² that meet at (40, 30), wound opposite ways.
    let path = profile("M20 10 L60 50 L60 10 L20 50 Z");
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 180.0, offset: 10.0, rotation_y: 20.0, ..options() };
    let faces = revolve(&path, bounds, o).unwrap();
    let cuts = cut_sides(&faces);
    // Each side draws each loop as a face.
    assert_eq!(cuts.len(), 4);
    let expected = 400.0 * 20f64.to_radians().cos();
    for cut in cuts {
        assert!((area(cut) - expected).abs() < 1e-6, "{} != {expected}", area(cut));
    }
    let mesh = revolve_mesh(&path, FillRule::NonZero, bounds, Revolve { angle: 90.0, rotation_y: 0.0, ..o }).unwrap();
    let start = area_facing(&mesh, [0.0, 0.0, -1.0]);
    assert!((start - 800.0).abs() < 1e-6, "{start}");
}

#[test]
fn the_mesh_closes_a_cut_side_that_crosses_itself_with_both_of_its_loops() {
    // Loops of 900 pt² and 100 pt² that meet at (50, 40): the mesh covers both, and nothing
    // between them.
    let path = profile("M20 10 L60 50 L60 30 L20 70 Z");
    let bounds = path.bounds().unwrap();
    let mesh = revolve_mesh(&path, FillRule::NonZero, bounds, Revolve { angle: 90.0, offset: 10.0, ..options() }).unwrap();
    let start = area_facing(&mesh, [0.0, 0.0, -1.0]);
    assert!((start - 1000.0).abs() < 1e-6, "{start}");
}

#[test]
fn a_region_thinner_than_a_millionth_of_a_point_is_debris_and_closes_no_cut_side() {
    // 0.0000005 pt wide and 80 pt tall, beside a square 40 pt across.
    let path = profile("M40 10 L40.0000005 10 L40.0000005 90 L40 90 Z M50 10 L90 10 L90 50 L50 50 Z");
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 90.0, offset: 10.0, ..options() };
    let mesh = revolve_mesh(&path, FillRule::NonZero, bounds, o).unwrap();
    let start = area_facing(&mesh, [0.0, 0.0, -1.0]);
    assert!((start - 40.0 * 40.0).abs() < 1e-6, "{start}");
}

#[test]
fn cut_sides_whose_holes_cannot_be_found_within_the_limit_are_refused() {
    // 2,100 squares on top of each other: each test of a point meets every square.
    let square = "M20 10 L60 10 L60 50 L20 50 Z ";
    let path = profile(&square.repeat(2100));
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 90.0, offset: 10.0, ..options() };
    assert_eq!(revolve(&path, bounds, o).err().as_deref(), Some(TOO_COMPLEX_PROFILE));
    // Hollow, the sweep has no cut sides to close.
    assert!(revolve(&path, bounds, Revolve { cap_ends: false, ..o }).is_ok());
}

#[test]
fn visibility_keeps_both_of_two_separate_regions_wound_opposite_ways() {
    let path = profile("M20 10 L30 10 L30 50 L20 50 Z M40 10 L40 50 L50 50 L50 10 Z");
    let bounds = path.bounds().unwrap();
    // Seen from behind, the half turn's walls are behind both cut sides, which nothing covers. Seen
    // from the front, the walls the outer square sweeps cover both sides completely.
    for (rotation_y, all) in [(180.0, 4.0 * 10.0 * 40.0), (0.0, 0.0)] {
        let faces = revolve(&path, bounds, Revolve { angle: 180.0, offset: 10.0, rotation_y, ..options() }).unwrap();
        let cuts = cut_sides(&faces);
        assert_eq!(cuts.len(), 4, "a face for each region on each side");
        let mut depths: Vec<f64> = cuts.iter().map(|f| f.depth).collect();
        depths.dedup();
        let visible = visible_faces(&faces).unwrap();
        let shown: f64 = visible
            .iter()
            .filter(|v| depths.contains(&v.depth))
            .flat_map(|v| v.path.subpaths.iter().map(|s| PathData::new(vec![s.clone()]).to_bezpath().area().abs()))
            .sum();
        assert!((shown - all).abs() < 1e-6, "turned {rotation_y}°, the cut sides show {shown}, not {all}");
    }
}

#[test]
fn every_open_contour_of_a_compound_profile_closes_its_own_region_on_each_side() {
    let path = profile("M40 10 L40 40 M20 60 L50 90");
    let bounds = Rect::new(0.0, 10.0, 50.0, 90.0);
    let faces = revolve(&path, bounds, Revolve { angle: 180.0, ..options() }).unwrap();
    let mut areas: Vec<f64> = cut_sides(&faces).iter().map(|f| area(f)).collect();
    areas.sort_by(f64::total_cmp);
    // The trapezoid between the slanted line and the axis, (20 + 50) / 2 × 30, and 40 × 30.
    let expected = [35.0 * 30.0, 35.0 * 30.0, 40.0 * 30.0, 40.0 * 30.0];
    assert_eq!(areas.len(), 4);
    assert!(areas.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6), "{areas:?}");
}

#[test]
fn a_cut_side_seen_edge_on_is_not_drawn() {
    let path = profile("M40 10 L40 90");
    let bounds = Rect::new(0.0, 10.0, 40.0, 90.0);
    // A quarter turn ends in the plane through the axis toward the viewer.
    let faces = revolve(&path, bounds, Revolve { angle: 90.0, ..options() }).unwrap();
    let cuts = cut_sides(&faces);
    assert_eq!(cuts.len(), 1);
    assert!((area(cuts[0]) - 40.0 * 80.0).abs() < 1e-6);
}

#[test]
fn cut_sides_are_shaded_evenly_by_their_plane() {
    let path = profile("M40 10 L40 90");
    let bounds = Rect::new(0.0, 10.0, 40.0, 90.0);
    let o = Revolve { angle: 180.0, shade: true, rotation_y: 30.0, ..options() };
    let faces = revolve(&path, bounds, o).unwrap();
    let cuts = cut_sides(&faces);
    assert_eq!(cuts.len(), 2);
    for cut in cuts {
        assert!(cut.gradient.is_none(), "a flat face is shaded evenly");
        assert!(cut.plane.is_some());
        assert!((0.0..=1.0).contains(&cut.brightness));
        // Both sides lie in the plane through the axis turned 30°, whose normal faces the viewer.
        let n = cut.normal;
        assert!((n[0].abs() - 30f64.to_radians().sin()).abs() < 1e-9 && n[2] > 0.0, "{n:?}");
    }
}

/// The area of the triangles of `mesh` whose normal is `normal`.
fn area_facing(mesh: &Mesh, normal: [f64; 3]) -> f64 {
    mesh.triangles
        .iter()
        .filter(|t| t.iter().all(|&i| mesh.normals[i as usize].iter().zip(normal).all(|(a, b)| (a - b).abs() < 1e-9)))
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let x = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() / 2.0
        })
        .sum()
}

#[test]
fn the_mesh_closes_both_cut_sides_and_leaves_holes_open_under_either_fill_rule() {
    let ring = "M20 10 L60 10 L60 50 L20 50 Z";
    let o = Revolve { angle: 90.0, offset: 10.0, ..options() };
    let expected = 40.0 * 40.0 - 20.0 * 20.0;
    // The hole wound against the outline, and wound with it under even-odd.
    for (hole, rule) in [("M30 20 L30 40 L50 40 L50 20 Z", FillRule::NonZero), ("M30 20 L50 20 L50 40 L30 40 Z", FillRule::EvenOdd)] {
        let path = profile(&format!("{ring} {hole}"));
        let mesh = revolve_mesh(&path, rule, path.bounds().unwrap(), o).unwrap();
        // The start faces back, against the sweep; a quarter turn ends facing left, along it.
        let start = area_facing(&mesh, [0.0, 0.0, -1.0]);
        let end = area_facing(&mesh, [-1.0, 0.0, 0.0]);
        assert!((start - expected).abs() < 1e-6, "{rule:?}: {start}");
        assert!((end - expected).abs() < 1e-6, "{rule:?}: {end}");
    }
    let path = profile(ring);
    let hollow = revolve_mesh(&path, FillRule::NonZero, path.bounds().unwrap(), Revolve { cap_ends: false, ..o }).unwrap();
    assert_eq!(area_facing(&hollow, [0.0, 0.0, -1.0]), 0.0);
}

#[test]
fn overlapping_contours_close_the_cut_sides_with_what_the_fill_rule_fills() {
    // Two squares of 1,600 pt² that overlap in 300 pt², and only one corner of each lies inside
    // the other. The even-odd rule leaves the overlap open; the nonzero rule fills it once.
    let path = profile("M20 10 L60 10 L60 50 L20 50 Z M50 20 L90 20 L90 60 L50 60 Z");
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 180.0, offset: 10.0, ..options() };
    for (rule, expected) in [(FillRule::EvenOdd, 2.0 * (1600.0 - 300.0)), (FillRule::NonZero, 2.0 * 1600.0 - 300.0)] {
        let faces = revolve(&path, bounds, Revolve { fill_rule: rule, ..o }).unwrap();
        let cuts = cut_sides(&faces);
        let total: f64 = cuts.iter().map(|f| area(f)).sum();
        assert!((total - 2.0 * expected).abs() < 1e-6, "{rule:?}: both sides cover {total}, not twice {expected}");
        let mesh = revolve_mesh(&path, rule, bounds, Revolve { angle: 90.0, ..o }).unwrap();
        let start = area_facing(&mesh, [0.0, 0.0, -1.0]);
        assert!((start - expected).abs() < 1e-6, "{rule:?}: {start}");
    }
}

#[test]
fn the_cut_sides_kept_for_a_mesh_have_a_corner_at_every_corner_of_the_side() {
    // With graphics mapped, the square's side is cut half way round from its outer side, at
    // (20, 30), which is not an anchor: the cut side takes that corner too.
    let path = profile("M20 10 L60 10 L60 50 L20 50 Z");
    let bounds = path.bounds().unwrap();
    let o = Revolve { angle: 90.0, offset: 10.0, map_surface: true, ..options() };
    let mut faces = vec![];
    revolve_faces(&path, FillRule::NonZero, bounds, o, Some(&mut faces)).unwrap();
    // The corners in the plane where the sweep starts, of every face kept for the mesh and of the
    // cut side's alone, which faces back against the sweep.
    let in_plane = |all: bool| {
        let start = |n: &V3| (n.0.abs() + n.1.abs() + (n.2 + 1.0).abs()) < 1e-9;
        let kept = faces.iter().filter(|f| all || start(&f.normal));
        kept.flat_map(|f| f.contours.iter().flatten().copied()).filter(|v| v.2.abs() < 1e-9).collect::<Vec<V3>>()
    };
    let cut = in_plane(false);
    assert!(!cut.is_empty());
    for p in in_plane(true) {
        assert!(cut.iter().any(|q| (p.0 - q.0).hypot(p.1 - q.1) < 1e-9), "{p:?} is not a corner of the cut side");
    }
}

#[test]
fn the_mesh_cut_sides_face_out_of_a_sweep_on_the_left_of_its_axis() {
    // The axis is 40 pt to the right of the line, which sweeps from the left of the axis to its back.
    let path = profile("M40 10 L40 90");
    let o = Revolve { angle: 90.0, edge: Edge::Right, offset: 40.0, ..options() };
    let mesh = revolve_mesh(&path, FillRule::NonZero, Rect::new(40.0, 10.0, 40.0, 90.0), o).unwrap();
    assert!((area_facing(&mesh, [0.0, 0.0, 1.0]) - 40.0 * 80.0).abs() < 1e-6);
    assert!((area_facing(&mesh, [1.0, 0.0, 0.0]) - 40.0 * 80.0).abs() < 1e-6);
}
