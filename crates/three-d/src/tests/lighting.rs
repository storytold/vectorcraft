use crate::*;

#[test]
fn color_combines_with_neutral_ambient_before_channel_clamping() {
    let n = V3(0.0, 0.0, 1.0);
    let o = Revolve { light_color: [0.0, 0.5, 1.0], ..Default::default() };
    assert_eq!(illumination(&o, n, n), [0.25, 0.65, 1.0]);
    assert_eq!(illumination(&o, V3(0.0, 0.0, -1.0), n), [0.25; 3]);
    assert_eq!(illumination(&Revolve { light_intensity: 0.0, ..o }, n, n), [0.25; 3]);
    assert_eq!(illumination(&Revolve { shade: false, ..o }, n, n), [1.0; 3]);
    assert_eq!(illumination(&Revolve { light_color: [0.0; 3], ..o }, n, n), [0.25; 3]);
}

#[test]
fn white_light_matches_existing_brightness_with_softness_front_and_back() {
    for softness in [0.0, 0.6, 1.0] {
        for behind in [false, true] {
            let o = Revolve { light_softness: softness, light_behind: behind, ..Default::default() };
            let [x, y, z] = o.light_direction();
            let l = V3(x, y, z);
            for angle in [-180.0f64, -90.0, 0.0, 45.0, 90.0, 180.0] {
                let (s, c) = angle.to_radians().sin_cos();
                let n = V3(s, 0.0, c);
                assert_eq!(illumination(&o, n, l), [brightness(&o, n, l); 3]);
            }
        }
    }
}

#[test]
fn every_geometry_type_produces_finite_rgb_illumination_without_changing_geometry() {
    let p = PathData::from_bezpath(&BezPath::from_svg("M0 0L100 0L100 100L0 100Z").unwrap());
    let b = p.bounds().unwrap();
    let view = View { light_height: 90.0, light_color: [1.0, 0.0, 0.0], ..Default::default() };
    let shapes = [
        plane(&p, vectorcraft_geom::FillRule::NonZero, b, view).unwrap(),
        extrude(&p, vectorcraft_geom::FillRule::NonZero, b, Extrude::default(), view).unwrap(),
        inflate(&p, vectorcraft_geom::FillRule::NonZero, b, Inflate::default(), view).unwrap(),
    ];
    for f in shapes.iter().flatten() {
        assert!(f.illumination[0] >= f.illumination[1]);
        assert_eq!(f.illumination[1], 0.25);
        assert_eq!(f.illumination[2], 0.25);
    }
    assert_eq!(shapes[0][0].illumination, [1.0, 0.25, 0.25]);
    let profile = PathData::from_bezpath(&BezPath::from_svg("M0 0L0 100").unwrap());
    let o = Revolve { offset: 50.0, light_color: [1.0, 0.0, 0.0], ..Default::default() };
    let red = revolve(&profile, profile.bounds().unwrap(), o).unwrap();
    let white = revolve(&profile, profile.bounds().unwrap(), Revolve { light_color: [1.0; 3], ..o }).unwrap();
    assert_eq!(red.len(), white.len());
    for (r, w) in red.iter().zip(&white) {
        assert_eq!(r.points, w.points);
        assert_eq!(r.depth, w.depth);
        assert_eq!(r.illumination[0], w.brightness);
        assert_eq!(&r.illumination[1..], &[0.25, 0.25]);
    }
    let visible = visible_faces(&red).unwrap();
    assert!(!visible.is_empty());
    for f in visible {
        assert_eq!(f.illumination, [f.brightness, 0.25, 0.25]);
    }
}

#[test]
fn non_finite_and_out_of_range_light_channels_are_bounded() {
    let o = Revolve { light_color: [f64::NAN, -10.0, f64::INFINITY], ..Default::default() }.sanitized();
    assert_eq!(o.light_color, [1.0, 0.0, 1.0]);
    assert_eq!(illumination(&o, V3(0.0, 0.0, 1.0), V3(0.0, 0.0, 1.0)), [1.0, 0.25, 1.0]);
}

#[test]
fn smooth_color_keeps_channel_saturation_and_shared_edges() {
    let a = Point::new(0.0, 0.0);
    let b = Point::new(100.0, 0.0);
    let c = Point::new(100.0, 100.0);
    let d = Point::new(0.0, 100.0);
    let make = |points: &[Point], values: &[f64]| LightGradient {
        diffuse: ShadeGradient::triangle(points, values).unwrap(),
        ambient: 0.25,
        color: [1.0, 0.5, 0.0],
    };
    let g = make(&[a, b, c], &[0.0, 2.0, 1.0]);
    let h = make(&[a, c, d], &[0.0, 1.0, 0.5]);
    assert_eq!(g.stops().len(), 4, "two distinct channel clipping points");
    for (t, rgb) in g.stops() {
        assert_eq!(rgb, g.at(g.diffuse.start.lerp(g.diffuse.end, f64::from(t))));
    }
    for t in [0.0, 0.2, 0.5, 0.9, 1.0] {
        let p = a.lerp(c, t);
        for (x, y) in g.at(p).into_iter().zip(h.at(p)) {
            assert!((x - y).abs() < 1e-12);
        }
    }
    assert_eq!(g.at(b), [1.0, 1.0, 0.25]);
    assert_eq!(g.at(a.lerp(b, 0.5)), [1.0, 0.75, 0.25], "clipped red must not flatten green");
}

#[test]
fn colored_smooth_surfaces_keep_caps_curves_and_visibility_placement() {
    let p = PathData::from_bezpath(&BezPath::from_svg("M40 0 C70 20 70 60 40 80").unwrap());
    let o = Revolve { offset: 20.0, rotation_x: -25.0, light_color: [1.0, 0.4, 0.1], segments: 24, ..Default::default() };
    let faces = revolve(&p, p.bounds().unwrap(), o).unwrap();
    let white = revolve(&p, p.bounds().unwrap(), Revolve { light_color: [1.0; 3], ..o }).unwrap();
    assert_eq!(faces.len(), white.len());
    assert!(faces.iter().any(|f| f.light_gradient.is_some()));
    let caps: Vec<_> = faces.iter().filter(|f| f.cap).collect();
    assert_eq!(caps.len(), 2);
    assert!(caps.iter().all(|f| f.outline.is_some() && f.light_gradient.is_none()), "caps remain continuous flat-lit paths");
    for (red, white) in faces.iter().zip(&white) {
        assert_eq!(red.path(), white.path());
        assert_eq!(red.depth, white.depth);
        assert_eq!(red.gradient, white.gradient, "the legacy white ramp is unchanged");
    }
    let visible = visible_faces(&faces).unwrap();
    assert!(visible.iter().any(|f| f.light_gradient.is_some()));
    assert!(visible.iter().all(|v| faces.iter().any(|f| f.light_gradient == v.light_gradient && f.illumination == v.illumination)));
}
