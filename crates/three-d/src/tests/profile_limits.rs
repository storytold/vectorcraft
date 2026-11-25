use crate::*;
use vectorcraft_geom::{Anchor, SubPath};

/// A closed star of `points` tips: twice as many straight edges.
fn star(points: usize) -> PathData {
    let anchors = (0..points * 2)
        .map(|i| {
            let (s, c) = (i as f64 * std::f64::consts::PI / points as f64).sin_cos();
            let r = if i % 2 == 0 { 100.0 } else { 60.0 };
            Anchor::corner(Point::new(200.0 + r * c, 200.0 + r * s))
        })
        .collect();
    PathData::new(vec![SubPath::new(anchors, true)])
}

/// An open wave of `anchors` smooth anchors: one curved segment between each two.
fn wave(anchors: usize) -> PathData {
    let anchors = (0..anchors)
        .map(|i| {
            let p = Point::new(100.0 + if i % 2 == 0 { 0.0 } else { 20.0 }, i as f64);
            Anchor::smooth(p, p + Vec2::new(6.0, 0.3))
        })
        .collect();
    PathData::new(vec![SubPath::new(anchors, false)])
}

fn revolved(path: &PathData, options: Revolve) -> Result<Vec<Face>, String> {
    revolve(path, path.bounds().unwrap(), options)
}

#[test]
fn a_star_of_600_points_revolves_at_the_default_segments() {
    let faces = revolved(&star(600), Revolve::default()).unwrap();
    // The tip on the axis gives two edges with a side of no length.
    assert!((1198 * 64 * 2..=1200 * 64 * 2).contains(&faces.len()), "{}", faces.len());
    let refused = revolved(&star(600), Revolve { segments: 128, ..Revolve::default() }).unwrap_err();
    assert!(refused.contains("reduce Segments"), "{refused}");
}

#[test]
fn a_curved_profile_past_the_old_limit_takes_fewer_samples_for_each_curve() {
    let wave = wave(200);
    assert!(revolved(&wave, Revolve::default()).is_ok());
    let samples = revolve_curve_samples([&wave], Revolve::default()).unwrap();
    assert!((MIN_CURVE_SAMPLES..CURVE_SAMPLES).contains(&samples), "{samples}");
}

#[test]
fn a_profile_within_the_limits_keeps_every_sample() {
    assert_eq!(revolve_curve_samples([&wave(50)], Revolve::default()), Ok(CURVE_SAMPLES));
    assert_eq!(revolve_curve_samples([&star(5)], Revolve::default()), Ok(CURVE_SAMPLES));
}

#[test]
fn two_thousand_curves_fit_at_low_segments_and_are_refused_before_any_face_at_high() {
    let wave = wave(2001);
    let samples = revolve_curve_samples([&wave], Revolve { segments: 8, ..Revolve::default() }).unwrap();
    assert!((MIN_CURVE_SAMPLES..CURVE_SAMPLES).contains(&samples), "{samples}");
    assert!(revolved(&wave, Revolve { segments: 8, ..Revolve::default() }).is_ok());
    let refused = revolve_curve_samples([&wave], Revolve { segments: 128, ..Revolve::default() }).unwrap_err();
    assert!(refused.contains("reduce Segments"), "{refused}");
    assert_eq!(revolved(&wave, Revolve { segments: 128, ..Revolve::default() }), Err(refused));
}

#[test]
fn too_many_anchors_are_refused_as_a_profile() {
    let refused = revolve_curve_samples([&wave(MAX_PROFILE_POINTS + 1)], Revolve { segments: 8, ..Revolve::default() }).unwrap_err();
    assert_eq!(refused, TOO_COMPLEX_PROFILE);
}

#[test]
fn every_contour_takes_a_point_even_an_empty_one() {
    // Contours of one anchor and of none, with no segment between them.
    let lone = |i: usize| SubPath::new(if i.is_multiple_of(2) { vec![] } else { vec![Anchor::corner(Point::new(100.0, i as f64))] }, false);
    let path = PathData::new((0..=MAX_PROFILE_POINTS).map(lone).collect());
    assert_eq!(revolve_curve_samples([&path], Revolve::default()), Err(TOO_COMPLEX_PROFILE.to_string()));
    assert_eq!(hole_segments(&path, FillRule::NonZero), Err(TOO_COMPLEX_PROFILE.to_string()), "before any contour is built");
    let given = Revolve { curve_samples: Some(CURVE_SAMPLES), ..Revolve::default() };
    assert_eq!(revolve_mesh(&path, FillRule::NonZero, path.bounds().unwrap(), given).map(|_| ()), Err(TOO_COMPLEX_PROFILE.to_string()));
}

#[test]
fn a_profile_at_the_point_limit_is_taken_and_one_past_it_is_refused() {
    let straight = |anchors: usize| {
        let anchors = (0..anchors).map(|i| Anchor::corner(Point::new(100.0 + (i % 2) as f64, i as f64))).collect();
        PathData::new(vec![SubPath::new(anchors, false)])
    };
    let narrow = Revolve { angle: 1.0, cap_ends: false, segments: 8, ..Revolve::default() };
    assert_eq!(revolve_curve_samples([&straight(MAX_PROFILE_POINTS)], narrow), Ok(CURVE_SAMPLES));
    assert_eq!(revolve_curve_samples([&straight(MAX_PROFILE_POINTS + 1)], narrow), Err(TOO_COMPLEX_PROFILE.to_string()));
}

#[test]
fn the_profiles_of_one_object_share_one_budget() {
    let (one, other) = (star(600), star(600));
    let o = Revolve::default();
    assert!(revolve_curve_samples([&one], o).is_ok());
    assert!(revolve_curve_samples([&other], o).is_ok());
    let refused = revolve_curve_samples([&one, &other], o).unwrap_err();
    assert!(refused.contains("reduce Segments"), "{refused}");
    // Contours of one profile count together as well.
    let both = PathData::new(one.subpaths.iter().chain(&other.subpaths).cloned().collect());
    assert_eq!(revolve_curve_samples([&both], o), Err(refused));
}

#[test]
fn with_graphics_a_ring_at_the_face_limit_still_takes_the_edge_its_cut_adds() {
    // A closed many-sided ring, 50 clear of the axis, with the middle of an edge at the middle of
    // its outer side and, with an even number of edges, at the middle of its inner side.
    let ring = |edges: usize| {
        let anchors = (0..edges)
            .map(|i| {
                let (s, c) = ((i as f64 + 0.5) * std::f64::consts::TAU / edges as f64).sin_cos();
                Anchor::corner(Point::new(300.0 + 100.0 * c, 100.0 * s))
            })
            .collect();
        PathData::new(vec![SubPath::new(anchors, true)])
    };
    // Tilted, so the flat edges at the top and bottom draw faces too.
    let plain = Revolve { offset: 50.0, rotation_x: -20.0, ..Revolve::default() };
    let mapped = Revolve { map_surface: true, ..plain };
    // 1,280 edges of 128 faces each are the most that fit, with graphics or without.
    for o in [plain, mapped] {
        assert_eq!(revolve_curve_samples([&ring(1280)], o), Ok(CURVE_SAMPLES));
        assert!(revolve_curve_samples([&ring(1281)], o).unwrap_err().contains("reduce Segments"));
    }
    let faces = revolved(&ring(1280), plain).unwrap();
    assert_eq!(faces.len(), MAX_REVOLVE_FACES);
    assert!(faces.iter().all(|f| f.surface.is_none()));
    // With graphics the side is cut in the middle of the inner edge, which makes one edge more:
    // its faces are taken past the limit.
    assert_eq!(revolved(&ring(1280), mapped).unwrap().len(), MAX_REVOLVE_FACES + 128);
}

#[test]
fn deeply_nested_contours_stop_the_search_for_holes() {
    // Concentric triangles: each tests every contour outside it.
    let triangle = |r: f64| {
        let anchors = [(0.0, -r), (r, r), (-r, r)].iter().map(|(x, y)| Anchor::corner(Point::new(200_000.0 + x, 200_000.0 + y))).collect();
        SubPath::new(anchors, true)
    };
    let nested = PathData::new((1..=4000).map(|i| triangle(i as f64)).collect());
    assert_eq!(hole_segments(&nested, FillRule::NonZero), Err(TOO_COMPLEX_PROFILE.to_string()));
    let few = PathData::new((1..=4).map(|i| triangle(i as f64)).collect());
    assert_eq!(hole_segments(&few, FillRule::EvenOdd).map(|h| h.len()), Ok(4));
}

#[test]
fn a_point_is_tested_only_against_the_contours_near_it() {
    // 3,000 separate squares: each segment's middle lies in the bounds of its own square only.
    let squares = (0..3000)
        .map(|i| {
            let (x, y) = ((i % 60) as f64 * 20.0, (i / 60) as f64 * 20.0);
            let corners = [(x, y), (x + 10.0, y), (x + 10.0, y + 10.0), (x, y + 10.0)];
            SubPath::new(corners.iter().map(|(x, y)| Anchor::corner(Point::new(*x, *y))).collect(), true)
        })
        .collect();
    let (holes, work) = hole_segments_counted(&PathData::new(squares), FillRule::NonZero).unwrap();
    assert!(holes.iter().flatten().all(|hole| !hole));
    assert!(work < 200_000, "{work}");
}

#[test]
fn overlapping_contours_are_holes_only_where_they_overlap() {
    let square = |x: f64, y: f64, side: f64| {
        let corners = [(x, y), (x + side, y), (x + side, y + side), (x, y + side)];
        SubPath::new(corners.iter().map(|(x, y)| Anchor::corner(Point::new(*x, *y))).collect(), true)
    };
    let overlapping = PathData::new(vec![square(0.0, 0.0, 100.0), square(40.0, 40.0, 100.0)]);
    let expected = vec![vec![false, true, true, false], vec![true, false, false, true]];
    assert_eq!(hole_segments(&overlapping, FillRule::EvenOdd), Ok(expected));
}

#[test]
fn given_samples_are_used_for_each_curve() {
    let wave = wave(4);
    let at = |samples| revolved(&wave, Revolve { curve_samples: Some(samples), cap_ends: false, segments: 8, ..Revolve::default() }).unwrap().len();
    assert_eq!(at(4) * 2, at(8));
}
