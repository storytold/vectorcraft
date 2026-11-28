use super::*;
use crate::{Light, Lights};
use std::collections::{BTreeMap, BTreeSet};
use vectorcraft_geom::kurbo::{Circle, Shape};

#[test]
fn explicit_single_lights_match_legacy_controls_and_white_keeps_polygon_shading() {
    let mut curve = BezPath::new();
    curve.extend(Circle::new((100.0, 100.0), 50.0).path_elements(0.01));
    let path = PathData::from_bezpath(&curve);
    let bounds = path.bounds().unwrap();
    for (color, softness, behind) in [([1.0; 3], 0.0, false), ([1.0, 0.3, 0.1], 0.6, false), ([0.1, 0.4, 1.0], 0.6, true)] {
        let view =
            View { light_color: color, light_softness: softness, light_behind: behind, rotation_x: -25.0, rotation_y: 35.0, ..Default::default() };
        let light = Light { color, softness, behind, ..Default::default() };
        let single = inflate(&path, FillRule::NonZero, bounds, Inflate::default(), view).unwrap();
        let explicit =
            inflate(&path, FillRule::NonZero, bounds, Inflate::default(), View { lights: Some(Lights::new(&[light]).unwrap()), ..view }).unwrap();
        assert_eq!(single.len(), explicit.len());
        let mut lit = false;
        for (a, b) in single.iter().zip(explicit) {
            assert_eq!(a.contours, b.contours);
            if color == [1.0; 3] {
                assert!(a.rgb_gradient.is_none() && b.rgb_gradient.is_none());
                assert!((a.brightness - b.brightness).abs() < 1e-10);
                lit |= a.brightness > view.ambient + 0.1;
                continue;
            }
            let (Some(a), Some(b)) = (a.rgb_gradient, b.rgb_gradient) else { continue };
            for p in [a.channels[0].start, a.channels[0].end] {
                let rgb = a.at(p);
                assert!(rgb.into_iter().zip(b.at(p)).all(|(a, b)| (a - b).abs() < 1e-10), "single light: {rgb:?}, explicit: {:?}", b.at(p));
                lit |= rgb.into_iter().any(|v| v > view.ambient + 0.1);
            }
        }
        assert!(lit, "directional illumination must reach the smooth surface");
    }
}

#[test]
fn an_inflated_disc_has_spherical_lighting_and_continuous_shared_edges() {
    let mut curve = BezPath::new();
    curve.extend(Circle::new((100.0, 100.0), 50.0).path_elements(0.01));
    let path = PathData::from_bezpath(&curve);
    let lights = Lights::new(&[
        Light { color: [1.0, 0.0, 0.0], rotation: 135.0, height: 35.0, ..Default::default() },
        Light { color: [0.0, 0.0, 1.0], rotation: -35.0, height: 25.0, ..Default::default() },
    ])
    .unwrap();
    let view = View { lights: Some(lights), ambient: 0.1, ..Default::default() };
    let camera = Camera::new(view, path.bounds().unwrap(), 100.0);
    let faces = inflate(&path, FillRule::NonZero, path.bounds().unwrap(), Inflate::default(), view).unwrap();
    let mut vertices = BTreeMap::<(i64, i64), [f64; 3]>::new();
    let mut edges = BTreeMap::<[(i64, i64); 2], [f64; 3]>::new();
    let mut checked = 0;
    let mut rim = 0;
    let mut shared = 0;
    for face in faces {
        let ramp = face.rgb_gradient.unwrap_or_else(|| panic!("disc cells are smooth triangles: {:?}", face.contours));
        assert!(ramp.is_finite());
        let points = &face.contours[0];
        assert_eq!(points.len(), 3);
        let key = |p: Point| ((p.x * 1e7).round() as i64, (p.y * 1e7).round() as i64);
        for &p in points {
            let rgb = ramp.at(p);
            if let Some(previous) = vertices.insert(key(p), rgb) {
                assert!(rgb.into_iter().zip(previous).all(|(a, b)| (a - b).abs() < 1e-7), "vertex {p:?}");
            }
            let d = p - Point::new(100.0, 100.0);
            if d.hypot() < 35.0 {
                let normal = V3(d.x, d.y, (2500.0 - d.hypot2()).sqrt()).unit();
                let expected = crate::illumination(&camera.view, normal, camera.light);
                assert!(rgb.into_iter().zip(expected).all(|(a, b)| (a - b).abs() < 0.025), "{p:?}: {rgb:?} vs {expected:?}");
                checked += 1;
            }
            if (d.hypot() - 50.0).abs() < 0.01 {
                let expected = crate::illumination(&camera.view, V3(d.x, d.y, 0.0).unit(), camera.light);
                assert!(rgb.into_iter().zip(expected).all(|(a, b)| (a - b).abs() < 0.025), "rim normal {p:?}: {rgb:?} vs {expected:?}");
                rim += 1;
            }
        }
        for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(3) {
            let mut edge = [key(*a), key(*b)];
            edge.sort();
            let rgb = ramp.at(a.lerp(*b, 0.5));
            if let Some(previous) = edges.insert(edge, rgb) {
                assert!(rgb.into_iter().zip(previous).all(|(a, b)| (a - b).abs() < 1e-7), "edge {edge:?}");
                shared += 1;
            }
        }
    }
    assert!(checked > 1000 && shared > 2000 && rim > 40);
}

#[test]
fn triangulation_keeps_concave_cells_and_every_stitched_border_vertex() {
    let ring = vec![
        Point::new(0.0, 0.0),
        Point::new(1.0, 0.0),
        Point::new(1.0, 0.5),
        Point::new(1.0, 1.0),
        Point::new(0.5, 1.0),
        Point::new(0.5, 0.5),
        Point::new(0.0, 0.5),
    ];
    let triangles = cell_triangles(std::slice::from_ref(&ring)).unwrap();
    let area: f64 = triangles.iter().map(|p| signed_area(p)).sum();
    assert!((area - signed_area(&ring)).abs() < 1e-12);
    let corners: BTreeSet<_> = triangles.iter().flatten().map(|p| (p.x.to_bits(), p.y.to_bits())).collect();
    assert!(ring.iter().all(|p| corners.contains(&(p.x.to_bits(), p.y.to_bits()))));
    let mut hole = ring.clone();
    hole.reverse();
    assert!(cell_triangles(&[ring, hole]).is_none(), "a hole keeps its original multi-contour fill");
}

#[test]
fn puffed_backs_hollow_shells_and_perspective_keep_finite_smooth_ramps() {
    let path = PathData::from_bezpath(&BezPath::from_svg("M0 0C80 0 100 140 20 150C-60 160 -70 15 0 0Z").unwrap());
    let bounds = path.bounds().unwrap();
    for (both_sides, cap, twist, taper, perspective) in [(true, true, 0.0, 1.0, 0.0), (true, true, 80.0, 0.7, 80.0), (false, false, 0.0, 1.0, 120.0)]
    {
        let o = Inflate { both_sides, cap, twist, taper, ..Default::default() };
        let view = View { light_color: [1.0, 0.3, 0.1], rotation_x: -25.0, rotation_y: 145.0, perspective, ..Default::default() };
        let faces = inflate(&path, FillRule::NonZero, bounds, o, view).unwrap();
        assert!(faces.len() < MAX_FACES);
        assert!(faces.iter().filter_map(|f| f.rgb_gradient).count() > 1000);
        assert!(faces.iter().filter_map(|f| f.rgb_gradient).all(crate::RgbGradient::is_finite));
    }
}
