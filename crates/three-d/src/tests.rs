use super::*;

mod caps;
mod lighting;
mod multiple_lights;
mod smooth;

#[test]
fn light_orbits_and_handle_projection_match_the_shading_direction_on_both_sides() {
    for rotation in [-170.0f64, -45.0, 0.0, 83.0, 167.0] {
        for height in [0.0f64, 15.0, 45.0, 80.0] {
            for behind in [false, true] {
                let o = Revolve { light_rotation: rotation, light_height: height, light_behind: behind, ..Default::default() };
                let direction = o.light_direction();
                assert!((direction[0].hypot(direction[1]).hypot(direction[2]) - 1.0).abs() < 1e-12);
                assert_eq!(direction[2] < 0.0, behind && height > 0.0);
                for ([center, u, v], angle) in o.light_orbits().into_iter().zip([rotation, height]) {
                    let (s, c) = angle.to_radians().sin_cos();
                    for i in 0..3 {
                        assert!((center[i] + u[i] * c + v[i] * s - direction[i]).abs() < 1e-12);
                    }
                }
                let [r, h] = o.light_at_on_side(Point::new(direction[0], direction[1]));
                assert!((r - rotation).abs() < 1e-9 && (h - height).abs() < 1e-6, "{rotation}, {height} -> {r}, {h}");
            }
        }
    }
    // Dragged to the centre: straight in front, the Rotation kept; past the rim: at 0°.
    let o = Revolve { light_rotation: 135.0, ..Default::default() };
    assert_eq!(o.light_at_on_side(Point::new(0.0, 0.0)), [135.0, 90.0]);
    let [r, h] = o.light_at_on_side(Point::new(0.0, -3.0));
    assert!((r - 90.0).abs() < 1e-12 && h == 0.0);
    let [r, h] = o.light_at_on_side(Point::new(f64::NAN, f64::INFINITY));
    assert!(r.is_finite() && h.is_finite());
}

/// An effect from before Rotation and Height (azimuth and elevation) gets the same light:
/// the same direction, so the same shading.
#[test]
fn an_earlier_azimuth_and_elevation_give_the_same_light() {
    let old = |azimuth: f64, elevation: f64| {
        let (sl, cl) = elevation.to_radians().sin_cos();
        let (sa, ca) = azimuth.to_radians().sin_cos();
        [cl * sa, -sl, cl * ca]
    };
    for azimuth in [-360.0f64, -170.0, -45.0, 0.0, 90.0, 135.0, 179.0] {
        for elevation in [-90.0f64, -60.0, -1.0, 0.0, 30.0, 45.0, 89.0, 90.0] {
            let (light_rotation, light_height, light_behind) = light_from_azimuth_elevation(azimuth, elevation);
            assert!((-180.0..=180.0).contains(&light_rotation) && (0.0..=90.0).contains(&light_height));
            let new = Revolve { light_rotation, light_height, light_behind, ..Default::default() }.light_direction();
            let want = old(azimuth, elevation);
            for i in 0..3 {
                assert!((new[i] - want[i]).abs() < 1e-12, "{azimuth}, {elevation}: {new:?} against {want:?}");
            }
        }
    }
    // The default light is the earlier default's.
    let (r, h, behind) = light_from_azimuth_elevation(-45.0, 45.0);
    assert!((r - DEFAULT_LIGHT.0).abs() < 1e-12 && (h - DEFAULT_LIGHT.1).abs() < 1e-12 && !behind, "{r} {h}");
}

/// Softness wraps the light further round the object; without it, the earlier shading. Ambient
/// Light runs to 200 %, never past the art's own colour.
#[test]
fn softness_wraps_the_light_and_ambient_reaches_200_percent() {
    let o = Revolve::default().sanitized();
    let l = V3(0.0, 0.0, 1.0);
    let n = |deg: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        V3(s, 0.0, c)
    };
    for deg in [0.0, 30.0, 60.0, 89.0, 120.0] {
        let lambert = (o.ambient + o.light_intensity * n(deg).dot(l).max(0.0)).clamp(0.0, 1.0);
        assert!((brightness(&o, n(deg), l) - lambert).abs() < 1e-12);
    }
    let soft = Revolve { light_softness: 1.0, ..o };
    assert!(brightness(&soft, n(120.0), l) > brightness(&o, n(120.0), l) + 0.1, "lit past 90°");
    assert!((brightness(&soft, n(180.0), l) - o.ambient).abs() < 1e-12, "nothing straight away");
    assert!((brightness(&soft, n(0.0), l) - brightness(&o, n(0.0), l)).abs() < 1e-12, "the same facing the light");
    let bright = Revolve { ambient: 2.0, ..o };
    assert_eq!(brightness(&bright, n(180.0), l), 1.0);
    assert_eq!(
        Revolve { ambient: 5.0, light_softness: -1.0, light_height: 120.0, ..o }.sanitized(),
        Revolve { ambient: 2.0, light_softness: 0.0, light_height: 90.0, ..o }
    );
}

#[test]
fn ring_axes_match_euler_parameter_rotation_and_origin_stays_fixed() {
    let o = Revolve { rotation_x: 27.0, rotation_y: -39.0, rotation_z: 53.0, ..Default::default() };
    let seed = V3(0.3, -0.5, 0.7);
    let before = o.rotate(seed);
    let planes = o.rotation_planes();
    for (axis, [u, v]) in planes.iter().enumerate() {
        let normal = V3(u[0], u[1], u[2]).cross(V3(v[0], v[1], v[2]));
        assert!((normal.dot(normal) - 1.0).abs() < 1e-12);
        let mut changed = o;
        match axis {
            0 => changed.rotation_x += 0.0001,
            1 => changed.rotation_y += 0.0001,
            _ => changed.rotation_z += 0.0001,
        }
        let actual = changed.rotate(seed).sub(before);
        let expected = normal.cross(before);
        let step = 0.0001f64.to_radians();
        for (a, b) in [(actual.0 / step, expected.0), (actual.1 / step, expected.1), (actual.2 / step, expected.2)] {
            assert!((a - b).abs() < 1e-5);
        }
    }
    assert_eq!(Revolve { edge: Edge::Right, offset: 12.0, ..o }.rotation_origin(Rect::new(20.0, 30.0, 80.0, 100.0)), Point::new(92.0, 65.0));
}
#[test]
fn guides_match_projection_and_dragged_light_matches_shading_direction() {
    let o = Revolve { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, offset: 10.0, ..Default::default() };
    assert_eq!(o.view_axes(), [Point::new(1.0, 0.0), Point::new(0.0, 1.0), Point::new(0.0, 0.0)]);
    assert_eq!(o.projected_axis(Rect::new(20.0, 30.0, 80.0, 100.0)), [Point::new(10.0, 30.0), Point::new(10.0, 100.0)]);
    let [light_rotation, light_height] = o.light_at_on_side(Point::new(0.3, -0.4));
    let [x, y, z] = Revolve { light_rotation, light_height, ..o }.light_direction();
    assert!((x - 0.3).abs() < 1e-9 && (y + 0.4).abs() < 1e-9 && z > 0.0);
    let [rotation, height] = o.light_at_on_side(Point::new(2.0, -2.0));
    assert!(rotation.is_finite() && height == 0.0);
}
fn cylinder() -> (PathData, Rect) {
    let mut bp = BezPath::new();
    bp.move_to((40.0, 10.0));
    bp.line_to((40.0, 90.0));
    (PathData::from_bezpath(&bp), Rect::new(0.0, 10.0, 40.0, 90.0))
}
#[test]
fn cylinder_has_expected_silhouette_depth_order_and_shading() {
    let (p, b) = cylinder();
    let faces = revolve(&p, b, Revolve { rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    let points: Vec<_> = faces.iter().flat_map(|f| &f.points).collect();
    assert!((points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min) + 40.0).abs() < 1e-6);
    assert!((points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max) - 40.0).abs() < 1e-6);
    assert!(faces.windows(2).all(|w| w[0].depth <= w[1].depth));
    assert!(faces.iter().any(|f| f.brightness > 0.8) && faces.iter().any(|f| f.brightness < 0.4));
}
#[test]
fn edge_offset_sweep_and_rotation_change_the_surface() {
    let (p, b) = cylinder();
    let base = revolve(&p, b, Revolve::default()).unwrap();
    for o in [
        Revolve { edge: Edge::Right, offset: 20.0, ..Default::default() },
        Revolve { angle: 120.0, ..Default::default() },
        Revolve { rotation_z: 60.0, perspective: 40.0, ..Default::default() },
    ] {
        assert_ne!(base, revolve(&p, b, o).unwrap());
    }
    assert!(revolve(&p, b, Revolve { angle: 0.0, ..Default::default() }).unwrap().is_empty());
}
/// The angle that stands for an earlier perspective strength puts the eye where that strength
/// did (4 × the half size / strength), so files from before the angle look as they did.
#[test]
fn the_angle_for_an_earlier_strength_puts_the_eye_where_it_was() {
    for strength in [0.05, 0.3, 0.6, 1.0] {
        for size in [1.0, 37.5, 900.0] {
            let eye = eye_distance(perspective_from_strength(strength), size).unwrap();
            assert!((eye - 4.0 * size / strength).abs() < 1e-9 * eye, "{strength} {size}: {eye}");
        }
    }
    assert_eq!(perspective_from_strength(0.0), 0.0);
    assert!((perspective_from_strength(1.0) - 51.318).abs() < 1e-3, "{}", perspective_from_strength(1.0));
}

/// 0° to 160°: none at 0°, beyond either end held there, and at 160° the eye is still outside
/// the sphere the object's box fits in.
#[test]
fn perspective_runs_0_to_160_degrees_with_the_eye_outside_the_object() {
    assert_eq!(eye_distance(0.0, 50.0), None);
    assert_eq!(eye_distance(-20.0, 50.0), None);
    assert_eq!(eye_distance(f64::NAN, 50.0), None);
    assert_eq!(eye_distance(500.0, 50.0), eye_distance(160.0, 50.0));
    let widest = eye_distance(160.0, 50.0).unwrap();
    assert!(widest > 3f64.sqrt() * 50.0 && widest < 1.02 * 3f64.sqrt() * 50.0, "{widest}");
    // Wider is closer.
    let eyes: Vec<f64> = [10.0, 45.0, 90.0, 160.0].iter().map(|a| eye_distance(*a, 50.0).unwrap()).collect();
    assert!(eyes.windows(2).all(|w| w[0] > w[1]), "{eyes:?}");
    let (p, b) = cylinder();
    let wide = revolve(&p, b, Revolve { rotation_x: -30.0, perspective: 160.0, ..Default::default() }).unwrap();
    assert!(wide.iter().flat_map(|f| &f.points).all(|q| q.x.is_finite() && q.y.is_finite()));
    assert_eq!(wide, revolve(&p, b, Revolve { rotation_x: -30.0, perspective: 400.0, ..Default::default() }).unwrap());
}
#[test]
fn malformed_and_degenerate_inputs_are_bounded() {
    let (p, b) = cylinder();
    let f = revolve(&p, b, Revolve { segments: usize::MAX, angle: f64::NAN, offset: f64::INFINITY, ..Default::default() }).unwrap();
    assert!(f.len() <= 258);
    assert!(revolve(&p, Rect::new(f64::NAN, 0.0, 1.0, 1.0), Revolve::default()).is_err());
    assert!(revolve(&PathData::default(), b, Revolve::default()).unwrap().is_empty());
}
