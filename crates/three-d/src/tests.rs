use super::*;

mod caps;
mod classic;
mod lighting;
mod multiple_lights;
mod profile_limits;
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
fn side_faces_lie_on_the_unrolled_surface_and_caps_do_not() {
    let (p, b) = cylinder();
    // Without graphics no face carries a place on the surface.
    let plain = revolve(&p, b, Revolve { rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    assert!(plain.iter().all(|f| f.surface.is_none()));
    let faces = revolve(&p, b, Revolve { map_surface: true, rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    assert_eq!(faces.len(), plain.len());
    assert!(faces.iter().filter(|f| f.cap).all(|f| f.surface.is_none()));
    let side: Vec<_> = faces.iter().filter(|f| !f.cap).collect();
    assert!(!side.is_empty() && side.iter().all(|f| f.surface.as_ref().is_some_and(|c| c.uv.len() == c.page.len() && c.uv.len() == 3)));
    for f in &side {
        let cell = f.surface.as_ref().unwrap();
        // Straight on, x around the axis is the angle from the front times the radius 40: a corner
        // at page x lies at 40 asin(x / 40) on the front half; y runs 40 each way from the middle,
        // downward.
        for (q, uv) in cell.page.iter().zip(&cell.uv) {
            assert!((uv.y - (q.y - 50.0)).abs() < 1e-9, "{q:?} {uv:?}");
            if cell.outward {
                assert!((uv.x - 40.0 * (q.x / 40.0).clamp(-1.0, 1.0).asin()).abs() < 1e-6, "{q:?} {uv:?}");
            }
        }
        // The outside faces the viewer on the front half only.
        let mean = cell.uv.iter().map(|p| p.x).sum::<f64>() / cell.uv.len() as f64;
        assert_eq!(cell.outward, mean.abs() < 40.0 * std::f64::consts::FRAC_PI_2, "{mean}");
    }
    // A quarter sweep's middle is the middle of the sweep, 45° around the axis.
    let quarter = revolve(&p, b, Revolve { map_surface: true, angle: 90.0, rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    let xs: Vec<f64> = quarter.iter().filter_map(|f| f.surface.as_ref()).flat_map(|c| c.uv.iter().map(|p| p.x)).collect();
    let (lo, hi) = xs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
    let reach = 40.0 * std::f64::consts::FRAC_PI_4;
    assert!((lo + reach).abs() < 1e-9 && (hi - reach).abs() < 1e-9, "{lo} {hi}");
    let (w, h) = revolve_surface_size(&p, b, Revolve::default()).unwrap();
    assert!((w - 40.0 * std::f64::consts::PI).abs() < 1e-9 && (h - 80.0).abs() < 1e-3, "{w} {h}");
    assert_eq!(revolve_surface_size(&p, Rect::new(40.0, 10.0, 40.0, 90.0), Revolve::default()), None);
    // A curve that starts and ends on the axis but bows away from it sweeps a side with a length.
    let mut bow = BezPath::new();
    bow.move_to((0.0, 0.0));
    bow.curve_to((40.0, 0.0), (40.0, 80.0), (0.0, 80.0));
    let (w, h) = revolve_surface_size(&PathData::from_bezpath(&bow), Rect::new(0.0, 0.0, 30.0, 80.0), Revolve::default()).unwrap();
    assert!((w - 30.0 * std::f64::consts::PI).abs() < 1e-9 && h > 80.0, "{w} {h}");
}
#[test]
fn the_side_keeps_its_length_around_the_axis_at_every_distance_from_it() {
    // A step: 40 from the axis, 20 between y 40 and 60, then 40 again.
    let mut bp = BezPath::new();
    bp.move_to((40.0, 10.0));
    for p in [(40.0, 40.0), (20.0, 40.0), (20.0, 60.0), (40.0, 60.0), (40.0, 90.0)] {
        bp.line_to(p);
    }
    let (p, b) = (PathData::from_bezpath(&bp), Rect::new(0.0, 10.0, 40.0, 90.0));
    let faces = revolve(&p, b, Revolve { map_surface: true, rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    let (mut narrow, mut wide) = (0.0f64, 0.0f64);
    for cell in faces.iter().filter_map(|f| f.surface.as_ref()).filter(|c| c.outward) {
        // The faces of the narrow part run from y 40 to 60; those of the wide parts reach past
        // them; the steps between lie at y 40 or 60 only.
        let at = |y: f64| cell.page.iter().any(|q| (q.y - y).abs() < 1e-9);
        let radius = if at(40.0) && at(60.0) {
            Some(20.0)
        } else if cell.page.iter().any(|q| q.y < 40.0 - 1e-9 || q.y > 60.0 + 1e-9) {
            Some(40.0)
        } else {
            None
        };
        for (q, uv) in cell.page.iter().zip(&cell.uv) {
            // Straight on, a corner at page x that is r from the axis lies r asin(x / r) around it.
            let fits = |r: f64| (uv.x - r * (q.x / r).clamp(-1.0, 1.0).asin()).abs() < 1e-6;
            match radius {
                Some(r) => {
                    assert!(fits(r), "{r}: {q:?} {uv:?}");
                    let reach = if r < 30.0 { &mut narrow } else { &mut wide };
                    *reach = reach.max(uv.x.abs());
                }
                None => assert!(fits(20.0) || fits(40.0), "{q:?} {uv:?}"),
            }
        }
    }
    // The half seen from the front reaches a quarter turn each way: 10π at 20 from the axis, 20π
    // at 40.
    let quarter = std::f64::consts::FRAC_PI_2;
    assert!((narrow - 20.0 * quarter).abs() < 1.0 && (wide - 40.0 * quarter).abs() < 2.0, "{narrow} {wide}");
}
#[test]
fn with_the_axis_on_the_right_the_middle_of_the_side_faces_the_viewer() {
    let mut bp = BezPath::new();
    bp.move_to((0.0, 10.0));
    bp.line_to((0.0, 90.0));
    let (p, b) = (PathData::from_bezpath(&bp), Rect::new(0.0, 10.0, 40.0, 90.0));
    let q = 40.0 * std::f64::consts::FRAC_PI_4;
    // The side's reach around the axis either way of its middle: for 270° the middle is 225°, in
    // the middle of the part from 180° to 270° that faces the viewer; a quarter turn faces away,
    // and its middle is half the sweep.
    for (angle, lo_expected, hi_expected) in [(270.0, -q, 5.0 * q), (90.0, -q, q)] {
        let o = Revolve { map_surface: true, edge: Edge::Right, angle, rotation_x: 0.0, rotation_y: 0.0, ..Default::default() };
        let xs: Vec<f64> = revolve(&p, b, o).unwrap().iter().filter_map(|f| f.surface.as_ref()).flat_map(|c| c.uv.iter().map(|p| p.x)).collect();
        let (lo, hi) = xs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        assert!((lo - lo_expected).abs() < 1e-6 && (hi - hi_expected).abs() < 1e-6, "{angle}: {lo} {hi}");
    }
    // A new graphic fits the part of the sweep that faces the viewer: 90° of a 270° sweep, 20° of
    // a 200° one, all of a quarter turn that faces away.
    for (angle, seen) in [(270.0f64, 90.0f64), (200.0, 20.0), (90.0, 90.0), (360.0, 180.0)] {
        let (w, _) = revolve_surface_size(&p, b, Revolve { edge: Edge::Right, angle, ..Default::default() }).unwrap();
        assert!((w - 40.0 * seen.to_radians()).abs() < 1e-9, "{angle}: {w}");
    }
    // A full turn: straight on, the axis is at x 40, a corner at page x lies 40 asin((x - 40) / 40)
    // around it, and the front half shows its outside.
    let faces = revolve(&p, b, Revolve { map_surface: true, edge: Edge::Right, rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    let cells: Vec<_> = faces.iter().filter_map(|f| f.surface.as_ref()).collect();
    assert!(cells.iter().any(|c| c.outward));
    for cell in &cells {
        let mean = cell.uv.iter().map(|p| p.x).sum::<f64>() / cell.uv.len() as f64;
        assert_eq!(cell.outward, mean.abs() < 40.0 * std::f64::consts::FRAC_PI_2, "{mean}");
        if cell.outward {
            for (q, uv) in cell.page.iter().zip(&cell.uv) {
                assert!((uv.x - 40.0 * ((q.x - 40.0) / 40.0).clamp(-1.0, 1.0).asin()).abs() < 1e-6, "{q:?} {uv:?}");
            }
        }
    }
}
#[test]
fn only_the_longest_contour_of_a_compound_profile_carries_the_side_surface() {
    let mut bp = BezPath::new();
    bp.move_to((40.0, 10.0));
    bp.line_to((40.0, 90.0));
    bp.move_to((20.0, 30.0));
    bp.line_to((20.0, 50.0));
    let (p, b) = (PathData::from_bezpath(&bp), Rect::new(0.0, 10.0, 40.0, 90.0));
    let faces = revolve(&p, b, Revolve { map_surface: true, rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
    let short = |f: &Face| f.points.iter().all(|q| q.y > 29.0 && q.y < 51.0);
    assert!(faces.iter().any(|f| !f.cap && short(f)) && faces.iter().any(|f| !f.cap && !short(f)));
    for f in faces.iter().filter(|f| !f.cap) {
        assert_eq!(f.surface.is_some(), !short(f), "{:?}", f.points);
    }
    // The side's size is that contour's: with the longest contour 10 from the axis and a shorter
    // one 40 from it, the width is half a turn at 10 and the length the longest contour's 80.
    let mut bp = BezPath::new();
    bp.move_to((10.0, 10.0));
    bp.line_to((10.0, 90.0));
    bp.move_to((40.0, 30.0));
    bp.line_to((40.0, 50.0));
    let (w, h) = revolve_surface_size(&PathData::from_bezpath(&bp), b, Revolve::default()).unwrap();
    assert!((w - 10.0 * std::f64::consts::PI).abs() < 1e-9 && (h - 80.0).abs() < 1e-3, "{w} {h}");
}
/// The square ring of [`a_ring_profile_is_measured_from_its_outer_side_at_mid_height`] with an
/// anchor in the middle of its inner side.
fn split_square() -> [Point; 6] {
    [(20.0, 30.0), (40.0, 30.0), (40.0, 50.0), (20.0, 50.0), (20.0, 40.0), (20.0, 30.0)].map(|(x, y)| Point::new(x, y))
}
#[test]
fn a_ring_profile_is_measured_from_its_outer_side_at_mid_height() {
    // A square from 20 to 40 from the axis, starting at its top inner corner and running to the
    // right first (down the outer side).
    let ring = [(20.0, 30.0), (40.0, 30.0), (40.0, 50.0), (20.0, 50.0), (20.0, 30.0)].map(|(x, y)| Point::new(x, y));
    assert_eq!(along_profile(&ring, 0.0, true), vec![[-30.0, -10.0], [-10.0, 10.0], [10.0, 30.0], [30.0, 50.0]]);
    // Wound the other way the outer side still runs down the page as y increases. The cut is at
    // an inner corner, so each piece's y runs one way across it.
    let back = [(20.0, 30.0), (20.0, 50.0), (40.0, 50.0), (40.0, 30.0), (20.0, 30.0)].map(|(x, y)| Point::new(x, y));
    assert_eq!(along_profile(&back, 0.0, true), vec![[50.0, 30.0], [30.0, 10.0], [10.0, -10.0], [-10.0, -30.0]]);
    // The cut falls half way along a long inner side: Revolve puts an anchor there first, and y
    // then runs half the contour's length each way.
    let tall = [(20.0, 0.0), (40.0, 0.0), (40.0, 100.0), (20.0, 100.0), (20.0, 0.0)].map(|(x, y)| Point::new(x, y));
    assert_eq!(ring_cut(&tall, 0.0), Some((3, 0.5)));
    assert_eq!(ring_cut(&ring, 0.0), Some((3, 0.5)));
    assert_eq!(ring_cut(&split_square(), 0.0), None);
    assert_eq!(along_profile(&split_square(), 0.0, true), vec![[-30.0, -10.0], [-10.0, 10.0], [10.0, 30.0], [30.0, 40.0], [-40.0, -30.0]]);
    let split = [(20.0, 0.0), (40.0, 0.0), (40.0, 100.0), (20.0, 100.0), (20.0, 50.0), (20.0, 0.0)].map(|(x, y)| Point::new(x, y));
    assert_eq!(along_profile(&split, 0.0, true), vec![[-70.0, -50.0], [-50.0, 50.0], [50.0, 70.0], [70.0, 120.0], [-120.0, -70.0]]);
    let mut bp = BezPath::new();
    bp.move_to((20.0, 0.0));
    for p in [(40.0, 0.0), (40.0, 100.0), (20.0, 100.0)] {
        bp.line_to(p);
    }
    bp.close_path();
    let (path, bounds, o) =
        (PathData::from_bezpath(&bp), Rect::new(20.0, 0.0, 40.0, 100.0), Revolve { map_surface: true, offset: 20.0, ..Default::default() });
    let faces = revolve(&path, bounds, o).unwrap();
    // Without graphics the profile takes no anchor at the cut: its inner side stays one edge.
    let plain = revolve(&path, bounds, Revolve { map_surface: false, ..o }).unwrap();
    assert!(plain.len() < faces.len() && plain.iter().all(|f| f.surface.is_none()), "{} {}", plain.len(), faces.len());
    let ys: Vec<f64> = faces.iter().filter_map(|f| f.surface.as_ref()).flat_map(|c| c.uv.iter().map(|p| p.y)).collect();
    assert!(!ys.is_empty() && ys.iter().all(|y| y.abs() <= 120.0 + 1e-9), "{ys:?}");
    let (w, h) = revolve_surface_size(&path, bounds, o).unwrap();
    assert!((h - 240.0).abs() < 1e-6 && (w - 40.0 * std::f64::consts::PI).abs() < 1e-9, "{w} {h}");
}
#[test]
fn malformed_and_degenerate_inputs_are_bounded() {
    let (p, b) = cylinder();
    let f = revolve(&p, b, Revolve { segments: usize::MAX, angle: f64::NAN, offset: f64::INFINITY, ..Default::default() }).unwrap();
    assert!(f.len() <= 258);
    assert!(revolve(&p, Rect::new(f64::NAN, 0.0, 1.0, 1.0), Revolve::default()).is_err());
    assert!(revolve(&PathData::default(), b, Revolve::default()).unwrap().is_empty());
}

/// A revolved face carries its plane: the depth it gives at each corner of its drawing is the
/// corner's view-space depth, with and without perspective. A face whose corners lie off one plane,
/// or with a corner at or past the eye, has none.
#[test]
fn a_revolved_face_measures_its_depth_on_its_plane() {
    let o = Revolve { rotation_x: -30.0, rotation_y: 25.0, ..Default::default() }.sanitized();
    let origin = Point::new(50.0, 80.0);
    let light = V3(0.0, 0.0, 1.0);
    let [a, b, c, d] = [V3(-20.0, -10.0, 5.0), V3(20.0, -12.0, -15.0), V3(25.0, 15.0, -20.0), V3(-18.0, 14.0, 4.0)];
    // The fourth corner moved onto the plane of the first three.
    let n = b.sub(a).cross(c.sub(a));
    let d = V3(d.0, d.1, a.2 - (n.0 * (d.0 - a.0) + n.1 * (d.1 - a.1)) / n.2);
    let flat = [a, b, c, d];
    for eye in [None, Some(400.0)] {
        let face = project_face(&flat, None, o, origin, eye, light).unwrap().unwrap();
        let plane = face.plane.unwrap();
        for (p, v) in face.points.iter().zip(&flat) {
            let depth = plane.depth_at(*p).unwrap();
            assert!((depth - v.2).abs() < 1e-6, "{eye:?}: {depth} against {}", v.2);
        }
        let twisted = [a, b, c, V3(d.0, d.1, d.2 + 1.0)];
        assert!(project_face(&twisted, None, o, origin, eye, light).unwrap().unwrap().plane.is_none(), "{eye:?}");
    }
    let past = [a, b, V3(c.0, c.1, 500.0)];
    assert!(project_face(&past, None, o, origin, Some(400.0), light).unwrap().unwrap().plane.is_none());
    assert!(project_face(&past, None, o, origin, Some(1000.0), light).unwrap().unwrap().plane.is_some());
    // Every face of a revolved profile of straight edges has one.
    let (p, b) = cylinder();
    let faces = revolve(&p, b, Revolve { rotation_x: -30.0, perspective: 35.0, ..Default::default() }).unwrap();
    assert!(!faces.is_empty() && faces.iter().all(|f| f.plane.is_some()));
}
