use crate::*;

fn cylinder() -> (PathData, Rect) {
    let mut path = BezPath::new();
    path.move_to((40.0, 10.0));
    path.line_to((40.0, 90.0));
    (PathData::from_bezpath(&path), Rect::new(0.0, 10.0, 40.0, 90.0))
}

fn options() -> Revolve {
    Revolve { cap_ends: true, rotation_x: 90.0, shade: false, ..Default::default() }
}

fn caps(faces: &[Face]) -> impl Iterator<Item = &Face> {
    faces.iter().filter(|face| face.cap)
}

fn area(face: &Face) -> f64 {
    use vectorcraft_geom::Shape;
    face.path().to_bezpath().area().abs()
}

#[test]
fn full_turn_caps_cover_both_circular_ends_and_follow_depth_order() {
    let (path, bounds) = cylinder();
    let faces = revolve(&path, bounds, options()).unwrap();
    assert_eq!(caps(&faces).count(), 2);
    let expected = 2.0 * std::f64::consts::PI * 40.0f64.powi(2);
    assert!((caps(&faces).map(area).sum::<f64>() - expected).abs() < 0.05);
    assert!(caps(&faces).any(|f| (f.depth - 40.0).abs() < 1e-8));
    assert!(caps(&faces).any(|f| (f.depth + 40.0).abs() < 1e-8));
    assert!(faces.windows(2).all(|pair| pair[0].depth <= pair[1].depth));
    assert!(faces.iter().all(|face| face.brightness == 1.0));
}

#[test]
fn partial_turn_caps_are_sectors_and_zero_turn_has_no_geometry() {
    let (path, bounds) = cylinder();
    let faces = revolve(&path, bounds, Revolve { angle: 90.0, ..options() }).unwrap();
    assert_eq!(caps(&faces).count(), 2);
    let expected = 0.5 * std::f64::consts::PI * 40.0f64.powi(2);
    assert!((caps(&faces).map(area).sum::<f64>() - expected).abs() < 0.05);
    assert!(caps(&faces).flat_map(|face| &face.points).all(|point| point.x >= -1e-8 && point.y <= 50.0 + 1e-8));
    assert!(revolve(&path, bounds, Revolve { angle: 0.0, ..options() }).unwrap().is_empty());
}

#[test]
fn offsets_and_both_axis_sides_use_the_endpoint_ring_radius() {
    let (path, bounds) = cylinder();
    for (edge, offset, radius) in [(Edge::Left, 20.0, 60.0f64), (Edge::Right, 20.0, 20.0f64)] {
        let faces = revolve(&path, bounds, Revolve { edge, offset, ..options() }).unwrap();
        let expected = 2.0 * std::f64::consts::PI * radius.powi(2);
        assert!((caps(&faces).map(area).sum::<f64>() - expected).abs() < 0.05);
    }
}

#[test]
fn closed_coincident_and_on_axis_ends_do_not_gain_caps() {
    for data in ["M0 10 L40 30 L40 70 L0 90", "M40 10 L60 40 L40 10", "M40 10 L60 40 L40 90 Z"] {
        let path = PathData::from_bezpath(&BezPath::from_svg(data).unwrap());
        let bounds = path.bounds().unwrap();
        let uncapped = revolve(&path, bounds, Revolve { cap_ends: false, ..options() }).unwrap();
        assert_eq!(revolve(&path, bounds, options()).unwrap(), uncapped, "{data}");
    }
    let path = PathData::from_bezpath(&BezPath::from_svg("M0 10 L40 90").unwrap());
    let faces = revolve(&path, path.bounds().unwrap(), options()).unwrap();
    assert_eq!(caps(&faces).count(), 1, "only the nonzero endpoint needs a cap");
}

#[test]
fn curved_compound_profiles_caps_remain_finite_shaded_and_expandable() {
    let path = PathData::from_bezpath(&BezPath::from_svg("M40 10 C70 30 20 60 50 90 M30 110 L50 150").unwrap());
    let bounds = Rect::new(0.0, 10.0, 70.0, 150.0);
    let faces =
        revolve(&path, bounds, Revolve { rotation_x: 35.0, rotation_y: 27.0, perspective: perspective_from_strength(0.8), shade: true, ..options() })
            .unwrap();
    assert_eq!(caps(&faces).count(), 4);
    assert!(caps(&faces).all(|face| face.brightness.is_finite() && (0.0..=1.0).contains(&face.brightness)));
    assert!(!visible_faces(&faces).unwrap().is_empty());
    for sp in &path.subpaths {
        let single = PathData::new(vec![sp.clone()]);
        let reversed = PathData::from_bezpath(&single.to_bezpath().reverse_subpaths());
        let a = revolve(&single, bounds, options()).unwrap();
        let b = revolve(&reversed, bounds, options()).unwrap();
        assert!((caps(&a).map(area).sum::<f64>() - caps(&b).map(area).sum::<f64>()).abs() < 1e-8);
    }
}

#[test]
fn caps_share_surface_limits_and_malformed_inputs_stay_bounded() {
    let (path, bounds) = cylinder();
    let faces = revolve(&path, bounds, Revolve { segments: usize::MAX, angle: f64::NAN, offset: f64::INFINITY, ..options() }).unwrap();
    assert!(faces.len() <= 384);
    let mut excessive = BezPath::new();
    excessive.move_to((40.0, 0.0));
    for i in 1..=1024 {
        excessive.line_to((40.0, i as f64));
    }
    assert!(revolve(&PathData::from_bezpath(&excessive), Rect::new(0.0, 0.0, 40.0, 1024.0), options()).is_err());
    assert!(revolve(&PathData::default(), bounds, options()).unwrap().is_empty());
    // Viewed along the axis, the side walls disappear but the caps still consume
    // the same face budget. Repeated rings cannot bypass it through degenerate walls.
    let repeated = PathData::new(vec![path.subpaths[0].clone(); 129]);
    let o = Revolve { segments: 128, ..options() };
    assert!(revolve(&repeated, bounds, Revolve { cap_ends: false, ..o }).unwrap().is_empty());
    assert!(revolve(&repeated, bounds, o).unwrap_err().contains("surface is too complex"));
}

#[test]
fn caps_are_single_curved_paths_and_body_rims_share_their_exact_curves() {
    use vectorcraft_geom::PathSeg;
    let (path, bounds) = cylinder();
    for smooth_shading in [false, true] {
        for angle in [90.0, 270.0, 360.0] {
            let faces = revolve(
                &path,
                bounds,
                Revolve {
                    angle,
                    segments: 8,
                    shade: true,
                    smooth_shading,
                    rotation_x: -28.0,
                    perspective: perspective_from_strength(0.7),
                    ..options()
                },
            )
            .unwrap();
            let body: Vec<_> = faces.iter().filter(|f| !f.cap).flat_map(|f| f.path().to_bezpath().segments().collect::<Vec<_>>()).collect();
            for cap in caps(&faces) {
                let outline = cap.path();
                assert_eq!(outline.subpaths.len(), 1);
                assert!(outline.subpaths[0].closed);
                let mut curves = 0;
                for segment in outline.to_bezpath().segments() {
                    if let PathSeg::Cubic(c) = segment {
                        curves += 1;
                        let same = |a: Point, b: Point| a.distance(b) < 1e-10;
                        assert!(
                            body.iter().any(|s| match s {
                                PathSeg::Cubic(b) =>
                                    (same(c.p0, b.p0) && same(c.p1, b.p1) && same(c.p2, b.p2) && same(c.p3, b.p3))
                                        || (same(c.p0, b.p3) && same(c.p1, b.p2) && same(c.p2, b.p1) && same(c.p3, b.p0)),
                                _ => false,
                            }),
                            "cap and body use the same rim"
                        );
                    }
                }
                assert!(curves >= 8, "the cap contains curves instead of a polygon fan");
                assert!(cap.gradient.is_none(), "flat cap lighting has no radial seams");
            }
            let uncut = visible_faces(&faces[faces.len() - 1..]).unwrap();
            assert_eq!(uncut[0].path, faces.last().unwrap().path(), "uncut visibility preserves the curved outline");
        }
    }
}

#[test]
fn cap_centers_follow_the_projected_axis_at_wide_perspective_angles() {
    let (path, bounds) = cylinder();
    for perspective in [0.0, perspective_from_strength(0.7), 160.0] {
        let o = Revolve { perspective, rotation_x: -28.0, rotation_y: 27.0, shade: true, ..options() };
        let faces = revolve(&path, bounds, o).unwrap();
        let centers: Vec<_> = caps(&faces).map(|face| face.points[0]).collect();
        assert_eq!(centers.len(), 2);
        for center in o.projected_axis(bounds) {
            assert!(centers.iter().any(|p| p.distance(center) < 1e-10), "cap and axis share the perspective projection");
        }
        assert!(faces.iter().flat_map(|face| &face.points).all(|p| p.x.is_finite() && p.y.is_finite()));
    }
}

#[test]
fn capped_visibility_matches_painted_surfaces_above_below_and_for_major_sectors() {
    use vectorcraft_geom::Shape;
    let (path, bounds) = cylinder();
    for edge in [Edge::Left, Edge::Right] {
        for (angle, rotation_x, perspective, smooth_shading) in
            [(360.0, -28.0, 0.0, false), (360.0, 28.0, perspective_from_strength(0.7), true), (270.0, -32.0, perspective_from_strength(0.7), true)]
        {
            let faces = revolve(
                &path,
                bounds,
                Revolve { angle, edge, offset: 20.0, shade: true, rotation_x, rotation_y: 17.0, perspective, smooth_shading, ..options() },
            )
            .unwrap();
            let visible = visible_faces(&faces).unwrap();
            let all: Vec<_> = faces.iter().map(|f| (f.path().to_bezpath(), f.depth)).collect();
            let cut: Vec<_> = visible.iter().map(|f| (f.path.to_bezpath(), f.depth)).collect();
            for x in 0..40 {
                for y in 0..40 {
                    let p = Point::new(-100.0 + (x as f64 + 0.337) * 6.0, -50.0 + (y as f64 + 0.413) * 6.0);
                    let expected = all.iter().rev().find(|(bp, _)| bp.winding(p) != 0).map(|(_, depth)| depth);
                    let actual = cut.iter().rev().find(|(bp, _)| bp.winding(p) != 0).map(|(_, depth)| depth);
                    assert_eq!(actual, expected, "{edge:?}, {angle}, {rotation_x}, {p:?}");
                }
            }
        }
    }
}
