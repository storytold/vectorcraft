use crate::*;

fn path(d: &str) -> PathData {
    PathData::from_bezpath(&BezPath::from_svg(d).unwrap())
}

fn value(f: &Face, p: Point) -> f64 {
    f.gradient.map_or(f.brightness, |g| g.at(p))
}

#[test]
fn gradient_matches_three_vertex_values_and_shared_diagonal() {
    let [a, b, c, d] = [Point::new(10.0, 20.0), Point::new(90.0, 30.0), Point::new(80.0, 95.0), Point::new(0.0, 85.0)];
    let g = ShadeGradient::triangle(&[a, b, c], &[0.2, 0.9, 0.6]).unwrap();
    let h = ShadeGradient::triangle(&[a, c, d], &[0.2, 0.6, 0.4]).unwrap();
    for (p, expected) in [(a, 0.2), (b, 0.9), (c, 0.6)] {
        assert!((g.at(p) - expected).abs() < 1e-12);
    }
    for t in [0.0, 0.1, 0.5, 0.9, 1.0] {
        assert!((g.at(a.lerp(c, t)) - h.at(a.lerp(c, t))).abs() < 1e-12);
    }
    assert!(ShadeGradient::triangle(&[a, a, c], &[0.1, 0.5, 0.9]).is_none());
    assert!(ShadeGradient::triangle(&[a, b, c], &[0.4; 3]).is_none());
}

#[test]
fn curved_profile_shares_lighting_across_radial_and_curve_sample_edges() {
    let p = path("M40 0 C70 20 70 60 40 80");
    let o = Revolve { smooth_shading: true, cap_ends: false, offset: 20.0, angle: 90.0, rotation_y: -45.0, segments: 16, ..Default::default() };
    let faces = revolve(&p, p.bounds().unwrap(), o).unwrap();
    assert!(faces.iter().filter(|f| f.gradient.is_some()).count() > 60);
    let mut edges = 0;
    for (i, a) in faces.iter().enumerate() {
        for b in faces.iter().skip(i + 1) {
            let shared: Vec<_> = a.points.iter().filter(|p| b.points.iter().any(|q| p.distance(*q) < 1e-8)).copied().collect();
            if let [p, q] = shared.as_slice() {
                edges += 1;
                for t in [0.0, 0.5, 1.0] {
                    let p = p.lerp(*q, t);
                    assert!((value(a, p) - value(b, p)).abs() < 1e-8, "lighting seam at {p:?}");
                }
            }
        }
    }
    assert!(edges > 100, "test both curve sample boundaries and triangle diagonals");
}

#[test]
fn profile_corners_and_caps_retain_hard_edges() {
    let p = path("M40 0 L40 40 L70 40");
    let o = Revolve { smooth_shading: true, offset: 30.0, angle: 90.0, rotation_y: -45.0, rotation_x: -20.0, segments: 16, ..Default::default() };
    let faces = revolve(&p, p.bounds().unwrap(), o).unwrap();
    let mut hard = false;
    for a in &faces {
        for b in &faces {
            let shared: Vec<_> = a.points.iter().filter(|p| b.points.iter().any(|q| p.distance(*q) < 1e-8)).copied().collect();
            if shared.len() == 2 && shared.iter().any(|p| (value(a, *p) - value(b, *p)).abs() > 0.1) {
                hard = true;
            }
        }
    }
    assert!(hard, "the profile's ninety-degree corner is not rounded by shading");
    let p = path("M40 0 L40 80");
    let flat = revolve(&p, p.bounds().unwrap(), Revolve { cap_ends: true, smooth_shading: false, ..o }).unwrap();
    let smooth = revolve(&p, p.bounds().unwrap(), Revolve { cap_ends: true, ..o }).unwrap();
    let caps: Vec<_> = flat.iter().filter(|f| f.cap).collect();
    assert_eq!(caps.len(), 2);
    assert!(caps.into_iter().all(|cap| smooth.contains(cap)), "caps keep their exact flat shading");
}

#[test]
fn smooth_vertices_use_the_shared_soft_light_and_back_light() {
    let p = path("M40 0 L40 80");
    let bounds = p.bounds().unwrap();
    for behind in [false, true] {
        let o = Revolve {
            offset: 30.0,
            angle: 90.0,
            rotation_x: -20.0,
            rotation_y: -30.0,
            cap_ends: false,
            light_rotation: 0.0,
            light_height: 30.0,
            light_behind: behind,
            light_softness: 0.8,
            ambient: 0.15,
            ..Default::default()
        };
        let faces = revolve(&p, bounds, o).unwrap();
        let vertex = o.rotate(V3(30.0, -40.0, 0.0));
        let point = Point::new(10.0 + vertex.0, 40.0 + vertex.1);
        let normal = o.rotate(V3(1.0, 0.0, 0.0));
        let [x, y, z] = o.light_direction();
        let expected = (0.15 + 0.8 * ((normal.dot(V3(x, y, z)) + 0.8) / 1.8).max(0.0)).clamp(0.0, 1.0);
        let samples: Vec<_> = faces.iter().filter(|face| face.points.iter().any(|p| p.distance(point) < 1e-10)).collect();
        assert!(!samples.is_empty());
        for face in samples {
            assert!((value(face, point) - expected).abs() < 1e-10, "smooth shading preserves Softness and the light's side");
        }
    }
}

#[test]
fn unshaded_legacy_degenerate_and_high_complexity_surfaces_stay_bounded() {
    let p = path("M40 0 L40 80");
    let b = p.bounds().unwrap();
    let o = Revolve { offset: 30.0, ..Default::default() };
    assert!(revolve(&p, b, o).unwrap().iter().any(|f| f.gradient.is_some()));
    assert!(revolve(&p, b, Revolve { smooth_shading: false, ..o }).unwrap().iter().all(|f| f.gradient.is_none()));
    assert_eq!(revolve(&p, b, Revolve { shade: false, ..o }).unwrap(), revolve(&p, b, Revolve { shade: false, smooth_shading: true, ..o }).unwrap());
    for edge in [Edge::Left, Edge::Right] {
        for perspective in [0.0, perspective_from_strength(1.0), 160.0] {
            let o = Revolve { smooth_shading: true, edge, perspective, rotation_x: -35.0, ..o };
            let faces = revolve(&p, b, o).unwrap();
            assert!(faces.iter().all(|f| f.points.iter().all(|p| value(f, *p).is_finite() && (0.0..=1.0).contains(&value(f, *p)))));
            assert!(visible_faces(&faces).is_ok());
        }
    }
    assert!(revolve(&p, b, Revolve { smooth_shading: true, angle: 0.0, ..o }).unwrap().is_empty());
    let mut p = path("M40 0");
    p.subpaths[0].anchors.extend((1..400).map(|i| vectorcraft_geom::Anchor::corner(Point::new(40.0 + (i % 2) as f64, i as f64))));
    assert!(revolve(&p, p.bounds().unwrap(), Revolve { smooth_shading: true, segments: 128, offset: 30.0, ..o }).is_err());
}
