use super::*;

#[test]
fn directional_lights_add_before_saturation_and_ambient_is_added_once() {
    let red = Light { height: 90.0, color: [1.0, 0.0, 0.0], intensity: 0.7, ..Default::default() };
    let blue = Light { color: [0.0, 0.0, 1.0], ..red };
    let o = Revolve { ambient: 0.2, lights: Some(Lights::new(&[red, blue]).unwrap()), ..Default::default() };
    let n = V3(0.0, 0.0, 1.0);
    for (actual, expected) in illumination(&o, n, n).into_iter().zip([0.9, 0.2, 0.9]) {
        assert!((actual - expected).abs() < 1e-12);
    }
    assert_eq!(illumination(&o, V3(0.0, 0.0, -1.0), n), [0.2; 3]);
    assert_eq!(illumination(&Revolve { shade: false, ..o }, n, n), [1.0; 3]);
    let same = Revolve { lights: Some(Lights::new(&[red, red]).unwrap()), ..o };
    assert_eq!(illumination(&same, n, n), [1.0, 0.2, 0.2]);
    let behind = Light { behind: true, ..blue };
    let opposite = Revolve { lights: Some(Lights::new(&[red, behind]).unwrap()), ..o };
    assert!(illumination(&opposite, n, n)[2] < 0.21);
}

#[test]
fn explicit_lights_are_bounded_and_sanitized_and_keep_the_legacy_single_light() {
    assert!(Lights::new(&[]).is_err());
    assert!(Lights::new(&[Light::default(); MAX_LIGHTS + 1]).is_err());
    assert_eq!(Lights::new(&[Light::default(); MAX_LIGHTS]).unwrap().iter().count(), MAX_LIGHTS);
    let invalid = Light { color: [f64::NAN, -1.0, f64::INFINITY], height: f64::NAN, ..Default::default() };
    assert_eq!(Lights::new(&[invalid]).unwrap().iter().next().unwrap().color, [1.0, 0.0, 1.0]);
    let o = Revolve { light_color: [0.3, 0.7, 1.0], light_softness: 0.6, ..Default::default() };
    let one = Light { color: o.light_color, softness: o.light_softness, ..Default::default() };
    let explicit = Revolve { lights: Some(Lights::new(&[one]).unwrap()), ..o };
    let [x, y, z] = o.light_direction();
    let l = V3(x, y, z);
    for n in [V3(0.0, 0.0, 1.0), V3(1.0, 0.0, 0.0), V3(0.0, -1.0, 0.0)] {
        assert_eq!(illumination(&o, n, l), illumination(&explicit, n, l));
    }
}

#[test]
fn smooth_rgb_channels_keep_different_axes_and_clip_after_interpolation() {
    let points = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 100.0)];
    let g = RgbGradient::triangle(&points, &[[0.1, 0.1, 0.2], [1.5, 0.1, 0.2], [0.1, 1.5, 0.2]]).unwrap();
    assert_eq!(g.at(points[1]), [1.0, 0.1, 0.2]);
    assert_eq!(g.at(points[2]), [0.1, 1.0, 0.2]);
    let middle = g.at(Point::new(25.0, 25.0));
    assert!((middle[0] - 0.45).abs() < 1e-12 && (middle[1] - 0.45).abs() < 1e-12);
    assert_eq!(middle[2], 0.2);
    assert!(g.is_finite());
}

#[test]
fn multiple_lights_preserve_cap_geometry_and_visibility_ramps() {
    let path = PathData::from_bezpath(&BezPath::from_svg("M40 0C70 20 70 60 40 80").unwrap());
    let bounds = path.bounds().unwrap();
    let options = Revolve { offset: 20.0, rotation_x: -25.0, segments: 24, ..Default::default() };
    let before = revolve(&path, bounds, options).unwrap();
    let lights = Lights::new(&[Light::default(), Light { rotation: -45.0, color: [0.0, 0.0, 1.0], ..Default::default() }]).unwrap();
    let after = revolve(&path, bounds, Revolve { lights: Some(lights), ..options }).unwrap();
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(a.path(), b.path());
    }
    assert_eq!(after.iter().filter(|f| f.cap).count(), 2);
    assert!(after.iter().filter(|f| f.cap).all(|f| f.rgb_gradient.is_none()));
    assert!(after.iter().any(|f| f.rgb_gradient.is_some()));
    for f in visible_faces(&after).unwrap() {
        assert!(after.iter().any(|source| source.rgb_gradient == f.rgb_gradient && source.illumination == f.illumination));
    }
}
