use super::*;

#[test]
fn sample_endpoints_and_mid() {
    let g = Gradient::default();
    assert_eq!(g.sample(0.0).0.to_hex(), "#ffffff");
    assert_eq!(g.sample(1.0).0.to_hex(), "#000000");
    let mid = g.sample(0.5).0.to_rgb()[0];
    assert!((mid - 0.5).abs() < 1e-5);
}

/// Sampling increasing positions in one pass gives what sampling each alone does, out-of-order
/// and repeated stop offsets included.
#[test]
fn increasing_samples_match_single_samples() {
    let stop = |offset: f32, hex: &str, midpoint: f32| GradientStop { midpoint, ..GradientStop::new(offset, Color::from_hex(hex).unwrap()) };
    let g = Gradient::new(
        GradientKind::Linear,
        vec![stop(0.1, "#ff0000", 0.3), stop(0.4, "#00ff00", 0.7), stop(0.4, "#0000ff", 0.5), stop(0.2, "#ffff00", 0.5), stop(0.9, "#000000", 0.5)],
    );
    let ts: Vec<f32> = (0..=40).map(|k| k as f32 / 40.0).collect();
    let one_by_one: Vec<_> = ts.iter().map(|&t| g.sample_with(t, Color::lerp_keep_model)).collect();
    assert_eq!(g.sample_increasing_with(ts, Color::lerp_keep_model), one_by_one);
}

/// The prepared sampler gives what sampling each position alone does, out-of-order stops included.
#[test]
fn the_sampler_matches_single_samples() {
    let stop = |offset: f32, hex: &str| GradientStop::new(offset, Color::from_hex(hex).unwrap());
    let g = Gradient::new(
        GradientKind::Linear,
        vec![stop(0.1, "#ff0000"), stop(0.4, "#00ff00"), stop(0.4, "#0000ff"), stop(0.2, "#ffff00"), stop(0.9, "#000000")],
    );
    let sample = g.sampler();
    for k in (0..=40).rev() {
        let t = k as f32 / 40.0;
        assert_eq!(sample(t), g.sample(t), "{t}");
    }
}

/// Looking stops up by their reach finds what scanning them does, for any order of offsets,
/// repeated, out-of-order and NaN ones included, and for positions outside and between them.
#[test]
fn stop_reach_matches_scanning_the_stops() {
    let colors = ["#ff0000", "#00ff00", "#0000ff", "#ffff00", "#00ffff", "#000000"];
    let offsets: [&[f32]; 5] = [
        &[0.1, 0.4, 0.4, 0.2, 0.9, 0.5],
        &[0.5, 0.5, 0.5, 0.5],
        &[0.9, 0.1, 0.8, 0.2, 0.7, 0.3],
        &[0.2, f32::NAN, 0.6, 0.1, f32::NAN, 1.0],
        &[f32::NAN, 0.3, 0.3, 0.8],
    ];
    for offsets in offsets {
        let stops: Vec<GradientStop> = offsets
            .iter()
            .zip(colors.iter().cycle())
            .map(|(&o, hex)| GradientStop { midpoint: 0.3, ..GradientStop::new(o, Color::from_hex(hex).unwrap()) })
            .collect();
        let g = Gradient::new(GradientKind::Linear, stops);
        let reach = StopReach::new(&g.stops);
        for t in (-4..=44).map(|k| k as f32 / 40.0).chain([f32::NAN, f32::INFINITY, f32::NEG_INFINITY]) {
            // Compared as printed: a NaN offset mixes NaN colours, which are equal only so.
            let (got, want) = (reach.sample_with(&g.stops, t, &Color::lerp_keep_model), g.sample_with(t, Color::lerp_keep_model));
            assert_eq!(format!("{got:?}"), format!("{want:?}"), "{offsets:?} at {t}");
        }
    }
}

/// A gradient's stops color what draws where they color a stretch that draws, for stops in any
/// order: the sampler mixes stop `k` with stop `k + 1` above every offset of the stops before it,
/// so an out-of-order stop's pair can have no width.
#[test]
fn drawn_stops_follow_the_sampled_stretches() {
    let stop = |offset: f32, opacity: f32| GradientStop { opacity, ..GradientStop::new(offset, Color::BLACK) };
    let g = |stops: Vec<GradientStop>| Gradient::new(GradientKind::Linear, stops);
    // In order: the clear stop next to the opaque one, not the one after it.
    assert_eq!(g(vec![stop(0.0, 1.0), stop(0.5, 0.0), stop(1.0, 0.0)]).drawn_stops(), vec![true, true, false]);
    // Out of order: the pair (0.8, 0.3) has no width, so the clear stop at 0.3 mixes only with
    // the clear stop at 0.8 before it, and the pair (0.3, 1.0) is sampled above 0.8.
    assert_eq!(g(vec![stop(0.0, 0.0), stop(0.8, 0.0), stop(0.3, 0.0), stop(1.0, 1.0)]).drawn_stops(), vec![false, false, true, true]);
    // An opaque stop out of order: the pairs (0, 0.8) and (0.7, 1) cover every offset, so the
    // pairs next to the stop at 0.3 have no width and it colors nothing.
    let out_of_order = |o: f32| g(vec![stop(0.0, 0.0), stop(0.8, o), stop(0.3, 1.0), stop(0.7, 0.0), stop(1.0, 0.0)]);
    assert_eq!(out_of_order(0.0).drawn_stops(), vec![false; 5]);
    // With the stop at 0.8 opaque, the pairs next to it with width are drawn.
    assert_eq!(out_of_order(1.0).drawn_stops(), vec![true, true, false, false, false]);
    // The last stop colors the offsets above every stop's, even when it is out of order.
    assert_eq!(g(vec![stop(0.0, 0.0), stop(0.6, 0.0), stop(0.5, 1.0)]).drawn_stops(), vec![false, false, true]);
    // The first stop colors the offsets up to its own.
    assert_eq!(g(vec![stop(0.0, 1.0), stop(0.0, 0.0), stop(1.0, 0.0)]).drawn_stops(), vec![true, false, false]);
    // One stop, every stop clear, and no stops at all.
    assert_eq!(g(vec![stop(0.5, 1.0)]).drawn_stops(), vec![true]);
    assert_eq!(g(vec![stop(0.0, 0.0), stop(1.0, 0.0)]).drawn_stops(), vec![false, false]);
    assert!(g(vec![]).drawn_stops().is_empty());
    // A freeform gradient draws from its points: its stops count while any is above opacity 0.
    let mut free = g(vec![stop(0.0, 1.0), stop(0.5, 0.0), stop(1.0, 0.0)]);
    free.kind = GradientKind::Freeform;
    assert_eq!(free.drawn_stops(), vec![true, true, true]);
    free.stops.iter_mut().for_each(|s| s.opacity = 0.0);
    assert_eq!(free.drawn_stops(), vec![false; 3]);
}

/// Taken as opaque, every stop that colors a stretch of positive width counts, and the first and
/// last stops, whatever their opacity; an out-of-order stop whose pairs have no width does not.
#[test]
fn reached_stops_leave_out_only_stops_without_width() {
    let stop = |offset: f32, opacity: f32| GradientStop { opacity, ..GradientStop::new(offset, Color::BLACK) };
    let g = |stops: Vec<GradientStop>| Gradient::new(GradientKind::Linear, stops);
    // Clear stops in order color their stretches once opaque.
    assert_eq!(g(vec![stop(0.0, 1.0), stop(0.5, 0.0), stop(1.0, 0.0)]).reached_stops(), vec![true; 3]);
    assert_eq!(g(vec![stop(0.0, 0.0), stop(1.0, 0.0)]).reached_stops(), vec![true, true]);
    // The stop at 0.3 sits inside the stretch the stops at 0 and 0.8 cover.
    let out_of_order = g(vec![stop(0.0, 0.0), stop(0.8, 0.0), stop(0.3, 1.0), stop(0.7, 0.0), stop(1.0, 0.0)]);
    assert_eq!(out_of_order.reached_stops(), vec![true, true, false, true, true]);
    // A stop at the offset of the stop before it: its pair has no width, the next one does.
    assert_eq!(g(vec![stop(0.0, 1.0), stop(0.0, 1.0), stop(1.0, 1.0)]).reached_stops(), vec![true, true, true]);
    assert_eq!(
        g(vec![stop(0.0, 1.0), stop(0.6, 1.0), stop(0.6, 1.0), stop(0.2, 1.0), stop(1.0, 1.0)]).reached_stops(),
        vec![true, true, false, true, true]
    );
    assert!(g(vec![]).reached_stops().is_empty());
    // Every pair the sampler picks between the ends has both its stops counted.
    let stops: Vec<GradientStop> = [0.1, 0.4, 0.4, 0.2, 0.9, 0.5].iter().map(|&o| stop(o, 1.0)).collect();
    let g = g(stops);
    let (reached, reach) = (g.reached_stops(), StopReach::new(&g.stops));
    for t in (5..=95).map(|k| k as f32 / 100.0).filter(|&t| t > 0.1 && t <= 0.9) {
        let k = reach.0.partition_point(|r| *r < t);
        assert!(reached[k] && reached[k + 1], "pair {k} at {t}: {reached:?}");
    }
}

/// Repeated offsets turn off the direct midpoint mix, and the midpoints still come out as
/// `sample` gives them, with work that stays near linear in the stops (thousands of stops at one
/// offset used to scan them all for each midpoint).
#[test]
fn expanded_with_repeated_offsets_samples_midpoints() {
    let stop = |offset: f32, hex: &str| GradientStop { midpoint: 0.25, ..GradientStop::new(offset, Color::from_hex(hex).unwrap()) };
    let g = Gradient::new(GradientKind::Linear, vec![stop(0.0, "#ff0000"), stop(0.5, "#00ff00"), stop(0.5, "#0000ff"), stop(1.0, "#000000")]);
    for (t, c, o, mid) in g.expanded().filter(|e| e.3.is_some()) {
        assert_eq!(mid, Some(0.25));
        assert_eq!((c, o), g.sample(t), "midpoint at {t}");
    }
    let many = Gradient::new(GradientKind::Linear, (0..200_000).map(|i| stop(if i == 0 { 0.0 } else { 0.5 }, "#808080")).collect());
    assert_eq!(many.expanded().count(), 2 * 200_000 - 1);
}

#[test]
fn midpoint_shifts() {
    let mut g = Gradient::default();
    g.stops[0].midpoint = 0.25;
    // At t = 0.25 we should be half way.
    assert!((g.sample(0.25).0.to_rgb()[0] - 0.5).abs() < 1e-5);
    assert_eq!(g.expanded_stops().len(), 3);
}

#[test]
fn fit_linear_horizontal() {
    let g = GradientGeom::fit(GradientKind::Linear, Rect::new(0.0, 0.0, 100.0, 50.0), 0.0);
    assert_eq!(g.start, Point::new(0.0, 25.0));
    assert_eq!(g.end, Point::new(100.0, 25.0));
    assert!((g.angle_deg()).abs() < 1e-9);
    let v = GradientGeom::fit(GradientKind::Linear, Rect::new(0.0, 0.0, 100.0, 50.0), 90.0);
    assert!((v.angle_deg() - 90.0).abs() < 1e-9);
    assert!((v.length() - 50.0).abs() < 1e-9);
}

#[test]
fn reverse_stops() {
    let mut g = Gradient::default();
    g.reverse();
    assert_eq!(g.stops[0].color.to_hex(), "#000000");
    assert_eq!(g.stops[0].offset, 0.0);
}

/// #1007: reversing mirrors off-centre midpoints with their segments, so the ramp is the same
/// the other way round.
#[test]
fn reverse_mirrors_midpoints() {
    let stop = |offset, hex, midpoint| GradientStop { midpoint, ..GradientStop::new(offset, Color::from_hex(hex).unwrap()) };
    let two = Gradient::new(GradientKind::Linear, vec![stop(0.0, "#ff0000", 0.2), stop(1.0, "#0000ff", 0.5)]);
    let three = Gradient::new(GradientKind::Linear, vec![stop(0.0, "#ff0000", 0.2), stop(0.3, "#00ff00", 0.8), stop(1.0, "#0000ff", 0.5)]);
    for g in [two, three] {
        let mut r = g.clone();
        r.reverse();
        let mids: Vec<f32> = r.stops.iter().map(|s| s.midpoint).collect();
        let want: Vec<f32> = g.stops.iter().rev().skip(1).map(|s| 1.0 - s.midpoint).chain([0.5]).collect();
        assert!(mids.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-6), "{mids:?} {want:?}");
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let ((a, ao), (b, bo)) = (g.sample(t), r.sample(1.0 - t));
            let same = a.to_rgba8(1.0).iter().zip(b.to_rgba8(1.0)).all(|(x, y)| x.abs_diff(y) <= 1);
            assert!(same && (ao - bo).abs() < 1e-6, "t {t}: {a:?} vs {b:?}");
        }
        r.reverse();
        let back = r.stops.iter().zip(&g.stops).all(|(a, b)| (a.offset - b.offset).abs() < 1e-6 && (a.midpoint - b.midpoint).abs() < 1e-6);
        assert!(back, "reversed twice, it is as it was: {:?}", r.stops);
    }
}

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-9
}

fn linear(start: (f64, f64), end: (f64, f64)) -> GradientGeom {
    GradientGeom { start: start.into(), end: end.into(), aspect: 1.0, focal: None }
}

#[test]
fn rotate_90_gives_vertical_vector() {
    let mut g = linear((0.0, 0.0), (100.0, 0.0));
    g.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), GradientKind::Linear);
    assert!(close(g.start, Point::ZERO) && close(g.end, Point::new(0.0, 100.0)), "{g:?}");
    assert!((g.angle_deg() + 90.0).abs() < 1e-9);
}

#[test]
fn reflect_swaps_start_and_end() {
    let mut g = linear((0.0, 25.0), (100.0, 25.0));
    // Reflect across the vertical axis through x = 50.
    let flip = Affine::translate((50.0, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate((-50.0, 0.0));
    g.transform(flip, GradientKind::Linear);
    assert!(close(g.start, Point::new(100.0, 25.0)) && close(g.end, Point::new(0.0, 25.0)), "{g:?}");
}

#[test]
fn non_uniform_scale_turns_a_circle_into_an_ellipse() {
    let mut g = GradientGeom { start: Point::new(10.0, 10.0), end: Point::new(20.0, 10.0), aspect: 1.0, focal: None };
    g.transform(Affine::scale_non_uniform(2.0, 1.0), GradientKind::Radial);
    assert!(close(g.start, Point::new(20.0, 10.0)) && close(g.end, Point::new(40.0, 10.0)), "{g:?}");
    assert!((g.aspect - 0.5).abs() < 1e-9);
    // A vertical vector keeps its axis: the ellipse is twice as wide as the vector is long.
    let mut v = GradientGeom { start: Point::ZERO, end: Point::new(0.0, -10.0), aspect: 1.0, focal: None };
    v.transform(Affine::scale_non_uniform(2.0, 1.0), GradientKind::Radial);
    assert!(close(v.end, Point::new(0.0, -10.0)) && (v.aspect - 2.0).abs() < 1e-9, "{v:?}");
}

#[test]
fn samples_follow_shear_and_rotation() {
    let shear = Affine::new([1.0, 0.3, 0.7, 1.2, 15.0, -4.0]) * Affine::rotate(0.4);
    for kind in [GradientKind::Linear, GradientKind::Radial] {
        let g = GradientGeom { start: Point::new(30.0, 40.0), end: Point::new(80.0, 55.0), aspect: 0.6, focal: None };
        let mut m = g;
        m.transform(shear, kind);
        for p in [Point::new(30.0, 40.0), Point::new(55.0, 70.0), Point::new(90.0, 10.0), Point::new(-20.0, 48.0)] {
            let (a, b) = (g.param_at(kind, p), m.param_at(kind, shear * p));
            assert!((a - b).abs() < 1e-9, "{kind:?} at {p:?}: {a} vs {b}");
        }
    }
}

#[test]
fn radial_fit_uses_the_angle_and_param_honours_aspect() {
    let b = Rect::new(0.0, 0.0, 100.0, 60.0);
    let g = GradientGeom::fit(GradientKind::Radial, b, 90.0);
    assert!(close(g.start, Point::new(50.0, 30.0)) && close(g.end, Point::new(50.0, -20.0)), "end above the centre: {g:?}");
    let e = GradientGeom { start: Point::ZERO, end: Point::new(10.0, 0.0), aspect: 0.5, focal: None };
    assert!((e.param_at(GradientKind::Radial, Point::new(0.0, 5.0)) - 1.0).abs() < 1e-12);
    assert!((e.param_at(GradientKind::Radial, Point::new(5.0, 0.0)) - 0.5).abs() < 1e-12);
    assert!((e.param_at(GradientKind::Linear, Point::new(5.0, 7.0)) - 0.5).abs() < 1e-12);
}

fn g3() -> Gradient {
    let mut g = Gradient::default();
    g.stops.insert(1, GradientStop::new(0.5, Color::rgb(1.0, 0.0, 0.0)));
    g
}

#[test]
fn insert_samples_color_and_sorts() {
    let g = Gradient::default();
    let (stops, i) = insert_stop(&g, 0.25);
    assert_eq!(stops.len(), 3);
    assert_eq!(i, 1);
    let r = stops[1].color.to_rgb()[0];
    assert!((r - 0.75).abs() < 1e-4, "sampled {r}");
    let (stops, i) = insert_stop(&g, 2.0);
    assert_eq!(i, 2);
    assert_eq!(stops[2].offset, 1.0);
}

#[test]
fn remove_keeps_two() {
    let g = g3();
    let v = remove_stop(&g.stops, 1).unwrap();
    assert_eq!(v.len(), 2);
    assert!(remove_stop(&v, 0).is_none());
    assert!(remove_stop(&g.stops, 9).is_none());
}

#[test]
fn move_reorders_and_tracks_index() {
    let g = g3();
    let (v, i) = move_stop(&g.stops, 0, 0.8);
    assert_eq!(i, 1);
    assert_eq!(v[1].color.to_hex(), "#ffffff");
    assert!(v.windows(2).all(|w| w[0].offset <= w[1].offset));
    let (v, i) = move_stop(&g.stops, 1, -3.0);
    // Clamped to 0 and placed after the existing stop at 0.
    assert_eq!((i, v[i].offset, v[i].color.to_hex()), (1, 0.0, "#ff0000".to_string()));
}

#[test]
fn duplicate_copies_the_stop_to_a_new_offset() {
    let g = g3();
    let (v, i) = duplicate_stop(&g.stops, 1, 0.9);
    assert_eq!((v.len(), i, v[i].offset, v[i].color.to_hex()), (4, 2, 0.9, "#ff0000".to_string()));
    assert_eq!(v[1], g.stops[1], "the original stays");
    assert_eq!(duplicate_stop(&g.stops, 7, 0.5).0, g.stops);
}

#[test]
fn swap_exchanges_only_the_colours() {
    let mut g = g3();
    g.stops[0].opacity = 0.25;
    let v = swap_stop_colors(&g.stops, 0, 1);
    assert_eq!((v[0].color.to_hex(), v[1].color.to_hex()), ("#ff0000".to_string(), "#ffffff".to_string()));
    assert_eq!((v[0].offset, v[0].opacity, v[1].offset, v[1].opacity), (0.0, 0.25, 0.5, 1.0));
    assert_eq!(swap_stop_colors(&g.stops, 0, 9), g.stops);
}

#[test]
fn midpoint_math() {
    let g = g3();
    assert_eq!(midpoint_pos(&g.stops, 0), Some(0.25));
    assert_eq!(midpoint_pos(&g.stops, 2), None);
    let m = midpoint_from_pos(&g.stops, 1, 0.6).unwrap();
    assert!((m - 0.2).abs() < 1e-5);
    assert_eq!(midpoint_from_pos(&g.stops, 1, 0.51).unwrap(), 0.13);
    let v = set_midpoint(&g.stops, 0, 0.99);
    assert_eq!(v[0].midpoint, 0.87);
}

#[test]
fn rebase_keeps_relative_positions() {
    let mut g = GradientGeom { start: Point::new(110.0, 150.0), end: Point::new(190.0, 120.0), aspect: 0.5, focal: None };
    g.rebase(Rect::new(100.0, 100.0, 200.0, 200.0), Rect::new(500.0, 0.0, 700.0, 50.0));
    assert!(close(g.start, Point::new(520.0, 25.0)) && close(g.end, Point::new(680.0, 10.0)) && g.aspect == 0.5, "{g:?}");
    // A box without height (a horizontal line) keeps the offset from its centre line.
    let mut h = GradientGeom { start: Point::new(0.0, 12.0), end: Point::new(10.0, 10.0), aspect: 1.0, focal: None };
    h.rebase(Rect::new(0.0, 10.0, 10.0, 10.0), Rect::new(0.0, 0.0, 20.0, 40.0));
    assert!(close(h.start, Point::new(0.0, 22.0)) && close(h.end, Point::new(20.0, 20.0)), "{h:?}");
    // Unplaced paints have nothing to move.
    let mut p = GradientPaint::new(Gradient::default());
    p.rebase(Rect::new(0.0, 0.0, 1.0, 1.0), Rect::new(5.0, 5.0, 9.0, 9.0));
    assert_eq!(p.geom, None);
}

#[test]
fn pin_fixes_the_fit_once() {
    let mut p = GradientPaint::new(Gradient::default());
    p.angle = 90.0;
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    p.pin(b);
    assert_eq!(p.geom, Some(GradientGeom::fit(GradientKind::Linear, b, 90.0)));
    p.pin(Rect::new(0.0, 0.0, 1.0, 1.0));
    assert_eq!(p.geom, Some(GradientGeom::fit(GradientKind::Linear, b, 90.0)));
}

/// A 100 pt circle at (200, 200) with its focal point 40 pt left of the centre.
fn off_centre() -> GradientGeom {
    let mut g = GradientGeom { start: Point::new(200.0, 200.0), end: Point::new(300.0, 200.0), aspect: 1.0, focal: None };
    g.set_focal(Some(Point::new(160.0, 200.0)));
    g
}

#[test]
fn focal_round_trips_through_serde_and_defaults_to_none() {
    let g = off_centre();
    let s = serde_json::to_string(&g).unwrap();
    assert!(s.contains("\"focal\""), "{s}");
    assert_eq!(serde_json::from_str::<GradientGeom>(&s).unwrap(), g);
    // Placements saved before focal points existed load centred, and centred ones write none.
    let old: GradientGeom = serde_json::from_str(r#"{"start":{"x":0.0,"y":0.0},"end":{"x":1.0,"y":0.0}}"#).unwrap();
    assert_eq!((old.focal, old.aspect), (None, 1.0));
    assert!(!serde_json::to_string(&old).unwrap().contains("focal"));
}

#[test]
fn focal_follows_transforms_and_rebases() {
    let mut g = off_centre();
    let a = Affine::translate((10.0, -5.0)) * Affine::rotate(0.7) * Affine::scale_non_uniform(2.0, 0.5);
    let f = a * g.focal.unwrap();
    g.transform(a, GradientKind::Radial);
    assert!(close(g.focal.unwrap(), f), "{g:?}");
    // Still at the same parameter: the first stop.
    assert!(g.param_at(GradientKind::Radial, f).abs() < 1e-9);
    let mut h = off_centre();
    h.rebase(Rect::new(100.0, 100.0, 300.0, 300.0), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(close(h.focal.unwrap(), Point::new(30.0, 50.0)) && close(h.start, Point::new(50.0, 50.0)), "{h:?}");
}

#[test]
fn param_runs_from_the_focal_point_to_the_ellipse() {
    let g = off_centre();
    let t = |x: f64, y: f64| g.param_at(GradientKind::Radial, Point::new(x, y));
    assert!(t(160.0, 200.0).abs() < 1e-9);
    // The extent circle is the last stop all round.
    for a in [0.0f64, 1.0, 2.5, 4.0] {
        let p = Point::new(200.0 + 100.0 * a.cos(), 200.0 + 100.0 * a.sin());
        assert!((g.param_at(GradientKind::Radial, p) - 1.0).abs() < 1e-9, "{a}");
    }
    // The centre lies on the level-2/7 circle: centred 2/7 of the way back to the focal point
    // (0.4 of the radius off), with a radius of 2/7.
    assert!((t(200.0, 200.0) - 2.0 / 7.0).abs() < 1e-9, "{}", t(200.0, 200.0));
    assert!(t(180.0, 200.0) < t(220.0, 200.0), "nearer the focal side is earlier");
    assert_eq!(GradientGeom { focal: None, ..g }.param_at(GradientKind::Radial, Point::new(250.0, 200.0)), 0.5);
}

#[test]
fn focal_is_kept_inside_the_ellipse_and_follows_its_reshaping() {
    let mut g = off_centre();
    g.set_focal(Some(Point::new(500.0, 200.0)));
    assert!(close(g.focal.unwrap(), Point::new(299.0, 200.0)), "pulled inside: {g:?}");
    g.set_focal(Some(Point::new(200.0, 200.0)));
    assert_eq!(g.focal, None, "on the centre: centred");
    // A new vector and aspect keep it in its place in the ellipse.
    let before = off_centre();
    let mut h = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(0.0, 50.0), aspect: 0.5, focal: None };
    h.keep_focal_from(&before);
    assert!(close(h.focal.unwrap(), Point::new(0.0, -20.0)), "{h:?}");
}

#[test]
fn pixels_sample_the_ramp_at_each_cell_centre() {
    // White → black left to right over a 10 × 2 box, a midpoint at 30% and a half-opaque end.
    let mut g = GradientPaint::new(Gradient::default());
    g.gradient.stops[0].midpoint = 0.3;
    g.gradient.stops[1].opacity = 0.5;
    let b = Rect::new(0.0, 0.0, 10.0, 2.0);
    let px: Vec<[u8; 4]> = g.pixels(b, b, 10, 2, true, &Color::to_rgb).collect();
    assert_eq!(px.len(), 20);
    let ramp = Ramp::new(&g.gradient, &Color::to_rgb);
    for (i, p) in px.iter().take(10).enumerate() {
        let c = ramp.at((i as f32 + 0.5) / 10.0).map(|v| (v * 255.0).round() as u8);
        assert_eq!(*p, c, "pixel {i}");
    }
    assert_eq!(px[3], px[13], "rows alike");
    // The midpoint stop: grey at 30%; padded past the ends.
    assert!((ramp.at(0.3)[0] - 0.5).abs() < 0.02, "{:?}", ramp.at(0.3));
    assert_eq!(ramp.at(-1.0), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(ramp.at(2.0), [0.0, 0.0, 0.0, 0.5]);
    assert_eq!(ramp.at(f32::NAN), [1.0, 1.0, 1.0, 1.0]);
}

fn blue_yellow(method: GradientMethod) -> Gradient {
    let mut g = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(0.0, 0.0, 1.0)), GradientStop::new(1.0, Color::rgb(1.0, 1.0, 0.0))],
    );
    g.method = method;
    g
}

/// Illustrator 30's Perceptual method mixes colours in a perceptually even way
/// (https://helpx.adobe.com/illustrator/desktop/paint-and-fill/create-and-edit-gradients/control-perceptual-interpolation-in-gradients.html).
/// Here that is OKLab: blue → yellow passes through a light blue-grey, not the flat mid grey
/// a display RGB mix gives, and its lightness climbs evenly.
#[test]
fn perceptual_blue_to_yellow_is_not_the_grey_rgb_mix() {
    let classic = blue_yellow(GradientMethod::Classic).sample(0.5).0.to_rgb();
    assert!(classic.iter().all(|v| (v - 0.5).abs() < 1e-5), "classic: the RGB mix {classic:?}");
    let p = blue_yellow(GradientMethod::Perceptual);
    let [r, g, b] = p.sample(0.5).0.to_rgb();
    assert!(b > r + 0.1 && b > g + 0.05, "a blue-grey, not grey: {:?}", [r, g, b]);
    assert!(r + g + b > 1.5 + 0.2, "lighter than the RGB mix: {:?}", [r, g, b]);
    let lightness = |t: f32| crate::libraries::to_oklab(&p.sample(t).0)[0];
    let steps: Vec<f64> = (0..4).map(|k| lightness((k + 1) as f32 / 4.0) - lightness(k as f32 / 4.0)).collect();
    assert!(steps.iter().all(|d| (d - steps[0]).abs() < 1e-3), "even lightness steps: {steps:?}");
    // Ends and opacity as before.
    assert_eq!(p.sample(0.0).0.to_hex(), "#0000ff");
    assert_eq!(p.sample(1.0).0.to_hex(), "#ffff00");
}

/// Renderers mix linearly between stops: a Perceptual segment becomes stops on the perceptual
/// curve (the midpoint diamond among them), more where it bends, close enough that the linear
/// ramp between them stays within half a colour level of the curve; a segment that hardly
/// bends needs few.
#[test]
fn perceptual_segments_expand_into_stops_on_the_curve() {
    let pairs = [
        (Color::rgb(0.0, 0.0, 1.0), Color::rgb(1.0, 1.0, 0.0)),
        (Color::BLACK, Color::rgb(1.0, 0.0, 0.0)),
        (Color::rgb(0.0, 1.0, 0.0), Color::rgb(1.0, 0.0, 1.0)),
        (Color::cmyk(0.0, 0.0, 0.0, 1.0), Color::WHITE),
    ];
    for (a, b) in pairs {
        let mut p = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, a), GradientStop::new(1.0, b)]);
        p.method = GradientMethod::Perceptual;
        p.stops[0].midpoint = 0.3;
        p.stops[1].opacity = 0.5;
        let ex: Vec<_> = p.expanded().collect();
        assert!((3..=100).contains(&ex.len()), "{a:?} → {b:?}: {} stops", ex.len());
        assert!(ex.windows(2).all(|w| w[0].0 < w[1].0), "in order");
        assert_eq!(ex.iter().filter(|s| s.3.is_some()).count(), 1, "one stop stands for the midpoint");
        assert!(ex.iter().any(|s| s.3 == Some(0.3) && (s.0 - 0.3).abs() < 1e-6));
        let ramp = Ramp::new(&p, &Color::to_rgb);
        for k in 0..=2000 {
            let t = k as f32 / 2000.0;
            let (c, o) = p.sample(t);
            let [r, g, b] = c.to_rgb();
            let off = [r, g, b, o].iter().zip(ramp.at(t)).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
            assert!(off < 0.5 / 255.0, "{a:?} → {b:?} at {t}: {off}");
        }
    }
    let mut flat = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(0.2, 0.4, 0.6)), GradientStop::new(1.0, Color::rgb(0.25, 0.45, 0.6))],
    );
    flat.method = GradientMethod::Perceptual;
    assert_eq!(flat.expanded().count(), 3, "the ends and the midpoint");
}

/// Classic gradients expand as they always did: their stops, and a stop at each off-centre
/// midpoint, sampled in display RGB (so the renderer draws the same pixels as before).
#[test]
fn classic_gradients_expand_as_before() {
    let mut g = Gradient::new(
        GradientKind::Linear,
        vec![
            GradientStop { midpoint: 0.2, ..GradientStop::new(0.0, Color::rgb(0.0, 0.0, 1.0)) },
            GradientStop { opacity: 0.4, ..GradientStop::new(0.45, Color::cmyk(0.1, 0.8, 0.0, 0.2)) },
            GradientStop { midpoint: 0.87, ..GradientStop::new(0.6, Color::rgb(1.0, 1.0, 0.0)) },
            GradientStop::new(1.0, Color::gray(0.3)),
        ],
    );
    // The expansion gradients had before they had a method.
    let before: Vec<_> = g
        .stops
        .iter()
        .enumerate()
        .flat_map(|(i, s)| {
            let mid = g.stops.get(i + 1).filter(|_| (s.midpoint - 0.5).abs() > 1e-3).map(|n| {
                let t = s.offset + (n.offset - s.offset) * s.midpoint;
                let (c, o) = g.sample_with(t, Color::lerp);
                (t, c, o, Some(s.midpoint))
            });
            std::iter::once((s.offset, s.color, s.opacity, None)).chain(mid)
        })
        .collect();
    assert_eq!(g.expanded().collect::<Vec<_>>(), before);
    assert_eq!(before.len(), 6);
    assert_eq!(g.sample(0.3), g.sample_with(0.3, Color::lerp));
    assert_eq!(g.sample_keep_model(0.5), g.sample_with(0.5, Color::lerp_keep_model));
    g.method = GradientMethod::Perceptual;
    assert_eq!(g.sample_keep_model(0.5), g.sample(0.5), "perceptual: one mix");
}

/// The method is written only when Perceptual, so files from before it (and Classic
/// gradients) read as Classic and new Classic files stay as they were.
#[test]
fn method_is_saved_only_when_perceptual() {
    let classic = serde_json::to_value(Gradient::default()).unwrap();
    assert!(classic.get("method").is_none(), "{classic}");
    let old: Gradient = serde_json::from_value(serde_json::json!({"kind": "Linear", "stops": []})).unwrap();
    assert_eq!(old.method, GradientMethod::Classic);
    let p = blue_yellow(GradientMethod::Perceptual);
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["method"], "Perceptual");
    assert_eq!(serde_json::from_value::<Gradient>(v).unwrap(), p);
    assert_eq!(GradientMethod::parse("perceptual"), Some(GradientMethod::Perceptual));
    assert_eq!(GradientMethod::parse("CLASSIC"), Some(GradientMethod::Classic));
    assert_eq!(GradientMethod::parse("smooth"), None);
}

/// SVG and PDF can't mix colours perceptually: a Perceptual gradient is drawn as pixels there,
/// as Illustrator rasterises perceptual gradients saved as PDF or SVG, as a fill or a stroke,
/// dithered or not.
#[test]
fn perceptual_gradients_draw_as_pixels() {
    for fill in [true, false] {
        assert!(!GradientPaint::new(blue_yellow(GradientMethod::Classic)).draws_as_pixels(fill));
        assert!(GradientPaint::new(blue_yellow(GradientMethod::Perceptual)).draws_as_pixels(fill));
        let mut radial = blue_yellow(GradientMethod::Perceptual);
        radial.kind = GradientKind::Radial;
        radial.dither = true;
        assert!(GradientPaint::new(radial).draws_as_pixels(fill));
    }
}

fn two_stops(a: Color, b: Color, method: GradientMethod) -> Gradient {
    let mut g = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, a), GradientStop::new(1.0, b)]);
    g.method = method;
    g
}

/// The hue methods mix around the hue wheel in one direction: Hue Counterclockwise raises the
/// hue angle from each stop to the next and Hue Clockwise lowers it, by the fixed-direction rules
/// of <https://www.w3.org/TR/css-color-4/#hue-interpolation> (the angles below are that
/// section's own examples), with saturation and brightness changing evenly.
#[test]
fn hue_methods_turn_one_way_around_the_wheel() {
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    let rising = two_stops(red, blue, GradientMethod::HueCounterclockwise);
    assert_eq!(rising.sample(0.0).0.to_hex(), "#ff0000");
    assert_eq!(rising.sample(0.25).0.to_hex(), "#ffff00", "60°");
    assert_eq!(rising.sample(0.5).0.to_hex(), "#00ff00", "120°");
    assert_eq!(rising.sample(1.0).0.to_hex(), "#0000ff");
    let falling = two_stops(red, blue, GradientMethod::HueClockwise);
    assert_eq!(falling.sample(0.0).0.to_hex(), "#ff0000");
    assert_eq!(falling.sample(0.5).0.to_hex(), "#ff00ff", "300°");
    assert_eq!(falling.sample(1.0).0.to_hex(), "#0000ff");
    assert_eq!(two_stops(red, blue, GradientMethod::Classic).sample(0.5).0.to_rgb(), [0.5, 0.0, 0.5], "classic: the RGB mix");
    // 30° to 190° and to 230°: halfway is 110° and 130° rising, 290° and 310° falling.
    for (to, up, down) in [(190.0, 110.0, 290.0), (230.0, 130.0, 310.0)] {
        let (a, b) = (Color::from_hsb(30.0, 0.5, 0.8), Color::from_hsb(to, 0.9, 0.4));
        for (method, want) in [(GradientMethod::HueCounterclockwise, up), (GradientMethod::HueClockwise, down)] {
            let [h, s, v] = method.mix(&a, &b, 0.5).to_hsb();
            assert!((h - want).abs() < 0.05, "{method:?} to {to}: hue {h}, not {want}");
            assert!((s - 0.7).abs() < 1e-3 && (v - 0.6).abs() < 1e-3, "{method:?}: saturation {s}, brightness {v}");
        }
    }
    // The other way round, each method takes the arc the other took.
    let back = two_stops(blue, red, GradientMethod::HueClockwise);
    assert_eq!(back.sample(0.5).0.to_hex(), "#00ff00");
    // A position that is not a number gives the first color.
    assert_eq!(hue_mix(&red, &blue, f32::NAN, true).to_hex(), "#ff0000");
}

/// A color without a hue does not turn the wheel: a gray takes the hue of the stop it mixes
/// with, black its saturation too, two grays mix as grays, and two stops of one hue keep that
/// hue, whichever way the method turns.
#[test]
fn hue_methods_keep_the_hue_where_a_stop_has_none() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let close = |c: Color, want: [f32; 3], what: &str| {
        let got = c.to_rgb();
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3), "{what}: {got:?}, not {want:?}");
    };
    let green = Color::rgb(0.0, 1.0, 0.0);
    for method in [GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise] {
        close(method.mix(&Color::WHITE, &green, 0.5), [0.5, 1.0, 0.5], "white to green: a light green");
        close(method.mix(&green, &Color::WHITE, 0.5), [0.5, 1.0, 0.5], "green to white: a light green");
        close(method.mix(&Color::rgb(0.5, 0.5, 0.5), &green, 0.5), [0.375, 0.75, 0.375], "gray to green");
        close(method.mix(&Color::BLACK, &green, 0.5), [0.0, 0.5, 0.0], "black to green: a dark green, not a gray green");
        close(method.mix(&green, &Color::BLACK, 0.25), [0.0, 0.75, 0.0], "green to black");
        close(method.mix(&Color::BLACK, &Color::WHITE, 0.5), [0.5, 0.5, 0.5], "black to white: gray");
        close(method.mix(&Color::BLACK, &Color::BLACK, 0.5), [0.0, 0.0, 0.0], "black to black");
        let dark = Color::rgb(0.5, 0.0, 0.0);
        close(method.mix(&red, &dark, 0.5), [0.75, 0.0, 0.0], "one hue: a red between the two");
        close(method.mix(&red, &red, 0.5), [1.0, 0.0, 0.0], "one color stays");
        // One hue with saturation and brightness both changing: the hue stays, and the color is
        // the even mix of saturation and brightness, which is not Classic's RGB mix.
        let pink = Color::rgb(1.0, 0.8, 0.8);
        close(method.mix(&dark, &pink, 0.5), [0.75, 0.3, 0.3], "dark red to pink");
        assert!((Color::lerp(&dark, &pink, 0.5).to_rgb()[1] - 0.4).abs() < 1e-3, "classic differs");
        // A color whose RGB numbers are under half a color level apart has no hue of its own: a
        // near gray leaning blue takes green's hue rather than turning from blue.
        let near_gray = Color::rgb(0.5, 0.5, 0.5 + 0.3 / 255.0);
        let [h, _, _] = method.mix(&near_gray, &green, 0.5).to_hsb();
        assert!((h - 120.0).abs() < 0.5, "{method:?}: near gray to green passes hue {h}");
        // One a whole color level apart has its hue (240°) and turns from it.
        let bluish = Color::rgb(128.0 / 255.0, 128.0 / 255.0, 129.0 / 255.0);
        let [h, _, _] = method.mix(&bluish, &green, 0.5).to_hsb();
        let want = if method == GradientMethod::HueClockwise { 180.0 } else { 0.0 };
        assert!(((h - want + 180.0).rem_euclid(360.0) - 180.0).abs() < 0.5, "{method:?}: bluish gray to green passes hue {h}, not {want}");
    }
}

/// Hues that differ only by rounding are one hue: a color and a tint or shade of it, whose
/// computed hue angles differ in their last digits, mix without turning the wheel, with either
/// method, and a stop added to a ramp leaves the ramp's colors as they were. Hues a whole color
/// level apart still turn.
#[test]
fn hue_methods_take_hues_within_half_a_color_level_as_one() {
    let level = |n: u8| f32::from(n) / 255.0;
    let rgb8 = |r: u8, g: u8, b: u8| Color::rgb(level(r), level(g), level(b));
    let methods = [GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise];
    // Colors against their 50% tint toward white and a shade at 40% brightness. At least one
    // pair's computed hues differ, or the test checks nothing.
    let colors = [rgb8(60, 253, 230), rgb8(241, 194, 107), rgb8(68, 32, 130), rgb8(13, 200, 77), rgb8(250, 3, 201), rgb8(90, 91, 200)];
    let mut uneven = 0;
    for c in colors {
        let [h, s, v] = c.to_hsb();
        let tint = Color::lerp(&c, &Color::WHITE, 0.5);
        let shade = Color::from_hsb(h, s, v * 0.4);
        for other in [tint, shade] {
            let [h1, _, _] = other.to_hsb();
            assert!((h1 - h).abs() < 0.01, "{c:?} and {other:?} are one hue: {h} and {h1}");
            uneven += usize::from(h1 != h);
            for method in methods {
                for (a, b) in [(c, other), (other, c)] {
                    for k in 0..=20 {
                        let [hm, _, _] = method.mix(&a, &b, k as f32 / 20.0).to_hsb();
                        let off = (hm - h + 180.0).rem_euclid(360.0) - 180.0;
                        assert!(off.abs() < 0.5, "{method:?} {a:?} → {b:?} at {k}/20: hue {hm}, not {h}");
                    }
                }
            }
        }
    }
    assert!(uneven > 0, "no pair's hues differ by rounding");
    // Across 0°: 359.99° and 0.01° are one hue too.
    for method in methods {
        let [h, _, _] = method.mix(&Color::from_hsb(359.99, 1.0, 1.0), &Color::from_hsb(0.01, 1.0, 1.0), 0.5).to_hsb();
        assert!(!(1.0..=359.0).contains(&h), "{method:?} across 0°: {h}");
    }
    // A stop added to a ramp samples the ramp's color there, and the ramp keeps its colors:
    // white to red, a color to its tint, and a turn from red to blue.
    let ramps = [
        (Color::WHITE, rgb8(255, 0, 0)),
        (rgb8(60, 253, 230), Color::lerp(&rgb8(60, 253, 230), &Color::WHITE, 0.5)),
        (rgb8(255, 0, 0), rgb8(0, 0, 255)),
    ];
    for method in methods {
        for (a, b) in ramps {
            let g = two_stops(a, b, method);
            for at in [0.21, 0.5, 0.83] {
                let (stops, i) = insert_stop(&g, at);
                assert_eq!((stops.len(), i), (3, 1));
                let with = Gradient { stops, ..g.clone() };
                for k in 0..=200 {
                    let t = k as f32 / 200.0;
                    let (was, now) = (g.sample(t).0.to_rgb(), with.sample(t).0.to_rgb());
                    let off = was.iter().zip(now).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
                    assert!(off < 1.0 / 255.0, "{method:?} {a:?} → {b:?}, a stop at {at}: {was:?} became {now:?} at {t}");
                }
            }
        }
    }
    // One color level of hue is a hue of its own: red to a red one level toward yellow turns
    // nearly the whole wheel one way and stays put the other.
    let (red, next) = (rgb8(255, 0, 0), rgb8(255, 1, 0));
    let [h, _, _] = GradientMethod::HueClockwise.mix(&red, &next, 0.5).to_hsb();
    assert!((h - 180.0).abs() < 1.0, "the long way: {h}");
    let [h, _, _] = GradientMethod::HueCounterclockwise.mix(&red, &next, 0.5).to_hsb();
    assert!(h < 1.0, "the short way: {h}");
}

/// Renderers mix linearly between stops: a hue segment becomes stops on its way around the
/// wheel, close enough that the linear ramp between them stays within half a color level of it,
/// through the midpoint and with the opacity changing evenly.
#[test]
fn hue_segments_expand_into_stops_on_the_curve() {
    let pairs = [
        (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0)),
        (Color::rgb(1.0, 0.0, 0.0), Color::rgb(1.0, 0.02, 0.0)),
        (Color::from_hsb(30.0, 0.5, 0.8), Color::from_hsb(190.0, 0.9, 0.4)),
        (Color::cmyk(0.0, 0.9, 0.9, 0.0), Color::WHITE),
        (Color::BLACK, Color::rgb(0.0, 1.0, 0.0)),
    ];
    for method in [GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise] {
        for (a, b) in pairs {
            let mut g = two_stops(a, b, method);
            g.stops[0].midpoint = 0.3;
            g.stops[1].opacity = 0.5;
            let ex: Vec<_> = g.expanded().collect();
            assert!((3..=600).contains(&ex.len()), "{method:?} {a:?} → {b:?}: {} stops", ex.len());
            assert!(ex.windows(2).all(|w| w[0].0 < w[1].0), "in order");
            assert!(ex.iter().any(|s| s.3 == Some(0.3) && (s.0 - 0.3).abs() < 1e-6), "a stop stands for the midpoint");
            let ramp = Ramp::new(&g, &Color::to_rgb);
            for k in 0..=4000 {
                let t = k as f32 / 4000.0;
                let (c, o) = g.sample(t);
                let [r, gr, bl] = c.to_rgb();
                let off = [r, gr, bl, o].iter().zip(ramp.at(t)).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
                assert!(off < 0.5 / 255.0, "{method:?} {a:?} → {b:?} at {t}: {off}");
            }
            assert_eq!(g.sample_keep_model(0.4), g.sample(0.4), "one mix");
        }
    }
}

/// Reversing a gradient gives the same ramp the other way round: a hue method turns the other
/// way with the stops, so every color keeps its place mirrored, and reversing twice gives the
/// gradient back. Classic and Perceptual keep their method. A backward span and the curve as a
/// Classic gradient hold the same colors.
#[test]
fn reversing_a_hue_gradient_keeps_its_colors_mirrored() {
    let near = |a: Color, b: Color, what: &str| {
        assert!(a.to_rgb().iter().zip(b.to_rgb()).all(|(x, y)| (x - y).abs() < 1.0 / 255.0), "{what}: {a:?} vs {b:?}");
    };
    for (method, other) in
        [(GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise), (GradientMethod::HueCounterclockwise, GradientMethod::HueClockwise)]
    {
        let mut g = Gradient::new(
            GradientKind::Linear,
            vec![
                GradientStop { midpoint: 0.3, ..GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)) },
                GradientStop { opacity: 0.5, ..GradientStop::new(0.6, Color::rgb(0.0, 0.0, 1.0)) },
                GradientStop::new(1.0, Color::rgb(0.9, 0.8, 0.1)),
            ],
        );
        g.method = method;
        let mut back = g.clone();
        back.reverse();
        assert_eq!(back.method, other);
        let (span, curve) = (g.span(1.0, 0.0), g.as_classic_curve());
        assert!(span.method.is_classic() && curve.method.is_classic());
        assert!(curve.stops.len() > g.stops.len() && curve.stops.iter().all(|s| s.swatch.is_none() && s.midpoint == 0.5));
        for k in 0..=400 {
            let t = k as f32 / 400.0;
            let (want, opacity) = g.sample(1.0 - t);
            near(want, back.sample(t).0, &format!("{method:?} reversed at {t}"));
            assert!((opacity - back.sample(t).1).abs() < 1e-4, "{method:?} opacity at {t}");
            near(want, span.sample(t).0, &format!("{method:?} backward span at {t}"));
            near(g.sample(t).0, curve.sample(t).0, &format!("{method:?} curve at {t}"));
        }
        back.reverse();
        assert_eq!(back.method, method);
        assert_eq!(back.stops.len(), g.stops.len());
    }
    for method in [GradientMethod::Classic, GradientMethod::Perceptual] {
        assert_eq!(method.mirrored(), method);
        let mut g = blue_yellow(method);
        g.reverse();
        assert_eq!(g.method, method);
    }
}

/// The hue methods are written by name, read back, and parsed from their param names; their
/// gradients draw as pixels in SVG and PDF, and are not mixes of the stops' inks.
#[test]
fn hue_methods_save_parse_and_draw_as_pixels() {
    for (method, name, id) in [
        (GradientMethod::HueClockwise, "HueClockwise", "hue-clockwise"),
        (GradientMethod::HueCounterclockwise, "HueCounterclockwise", "hue-counterclockwise"),
    ] {
        let g = two_stops(Color::cmyk(0.0, 1.0, 1.0, 0.0), Color::cmyk(1.0, 1.0, 0.0, 0.0), method);
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v["method"], name);
        assert_eq!(serde_json::from_value::<Gradient>(v).unwrap(), g);
        assert_eq!(method.id(), id);
        for written in [id.to_string(), id.to_uppercase(), id.replace('-', " "), id.replace('-', "_"), name.to_string(), method.label().to_string()] {
            assert_eq!(GradientMethod::parse(&written), Some(method), "{written}");
        }
        assert!(method.is_hue() && !method.is_classic());
        let paint = GradientPaint::new(g.clone());
        assert!(paint.draws_as_pixels(true) && paint.draws_as_pixels(false));
        assert!(g.ink_mixes().is_none(), "not mixes of the stops' inks");
        assert!(
            paint.ink_pixels(Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(0.0, 0.0, 10.0, 10.0), 4, 4, true).is_none(),
            "an RGB image, not an ink image"
        );
    }
    assert!(!GradientMethod::Perceptual.is_hue() && !GradientMethod::Classic.is_hue());
    for (id, method) in [("classic", GradientMethod::Classic), ("perceptual", GradientMethod::Perceptual)] {
        assert_eq!(method.id(), id);
        assert_eq!(GradientMethod::parse(id), Some(method));
    }
    for bad in [
        "hue",
        "clockwise",
        "hue-",
        "",
        "hue clockwise wise",
        "c-l-a-s-s-i-c",
        "per ceptual",
        "hue--clockwise",
        "hue-clock_wise",
        " classic",
        "hue- clockwise",
    ] {
        assert_eq!(GradientMethod::parse(bad), None, "{bad}");
    }
}

/// A Perceptual gradient of an ink's tints as tints: its stops keep their tints, and each tint on
/// the curve has the lightness of the color mixed there, so the tints between the stops are not
/// a straight line.
#[test]
fn ink_tints_follow_the_perceptual_curve() {
    use crate::libraries::srgb_to_oklab;
    let ink = Color::cmyk(0.0, 1.0, 0.0, 0.0);
    let mut g = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, ink.tinted(0.8)), GradientStop::new(1.0, Color::WHITE)]);
    g.method = GradientMethod::Perceptual;
    assert!(g.fits_ink_tints());
    let tints = g.ink_tints(&ink, &[]).expect("a magenta ink's tints differ in lightness");
    assert!(tints.len() > 2, "the curve's stops: {tints:?}");
    let (first, last) = (tints[0], tints[tints.len() - 1]);
    assert!((first.0, last.0) == (0.0, 1.0) && (first.1 - 0.8).abs() < 1e-4 && last.1 < 1e-4, "{tints:?}");
    // The stops' own tints, when given, are kept exactly; the tints in between are the same.
    let own = g.ink_tints(&ink, &[0.8, 0.0]).expect("tints");
    assert!(own.len() == tints.len() && own[0].1 == 0.8 && own[own.len() - 1].1 == 0.0, "{own:?}");
    assert!(own[1..own.len() - 1] == tints[1..tints.len() - 1], "{own:?}");
    assert!(tints.windows(2).all(|w| w[0].0 <= w[1].0 && w[0].1 >= w[1].1), "in order, lighter along the way: {tints:?}");
    let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
    for (t, tint, _) in &tints {
        let want = lightness(&g.sample(*t).0);
        assert!((lightness(&ink.tinted(*tint)) - want).abs() < 1e-3, "at {t}: tint {tint}");
    }
    let mid = tints.iter().find(|s| (s.0 - 0.5).abs() < 1e-6).expect("the midpoint");
    assert!((mid.1 - 0.4).abs() > 0.01, "not halfway in tint: {mid:?}");
    // A white ink's tints all have paper white's lightness: no tints by lightness.
    assert_eq!(g.ink_tints(&Color::WHITE, &[]), None);
    // More stops on the curve than MAX_INK_TINTS: no tints.
    let n = MAX_INK_TINTS + 1;
    g.stops = (0..n).map(|i| GradientStop::new(i as f32 / (n - 1) as f32, if i % 2 == 0 { ink } else { Color::WHITE })).collect();
    assert_eq!(g.ink_tints(&ink, &[]), None);
    assert!(!g.fits_ink_tints());
}

/// A Perceptual gradient's curve as mixes of its stops' inks: the stops on the curve at their
/// offsets, each weighted so the two stops' colors mixed in display RGB have the lightness of
/// the color on the curve there.
#[test]
fn ink_mixes_follow_the_perceptual_curve() {
    use crate::libraries::srgb_to_oklab;
    let (magenta, yellow) = (Color::cmyk(0.0, 1.0, 0.0, 0.0), Color::cmyk(0.0, 0.0, 0.6, 0.0));
    let mut g = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, magenta), GradientStop::new(1.0, yellow)]);
    g.stops[0].midpoint = 0.3;
    assert_eq!(g.ink_mixes(), None, "a Classic gradient");
    g.method = GradientMethod::Perceptual;
    let mixes = g.ink_mixes().expect("a Perceptual gradient's curve");
    let curve = g.expanded_stops();
    assert_eq!(mixes.len(), curve.len(), "a mix for each stop on the curve");
    assert_eq!((mixes[0], mixes[mixes.len() - 1]), ((0.0, 0, 0.0, 1.0), (1.0, 1, 0.0, 1.0)), "the gradient's own stops");
    let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
    for ((t, i, w, _), (ct, c, _)) in mixes.iter().zip(&curve) {
        assert!((t - ct).abs() < 1e-6, "{t} on the curve at {ct}");
        if *i == 0 && *t > 0.0 {
            let mixed = GradientMethod::Classic.mix(&magenta, &yellow, *w);
            assert!((lightness(&mixed) - lightness(c)).abs() < 1e-3, "at {t}: weight {w}");
        }
    }
    let mid = mixes.iter().find(|m| (m.0 - 0.3).abs() < 1e-6).expect("the midpoint");
    assert!((mid.2 - 0.5).abs() > 0.01, "not halfway at the midpoint: {mid:?}");
    assert!(mixes.windows(2).all(|w| w[0].0 <= w[1].0), "in order: {mixes:?}");
    // Two colors of one lightness: the weights are the mix positions.
    g.stops[1].color = magenta;
    let mixes = g.ink_mixes().expect("the curve");
    let mid = mixes.iter().find(|m| (m.0 - 0.3).abs() < 1e-6).expect("the midpoint");
    assert!((mid.2 - 0.5).abs() < 1e-6, "{mid:?}");
    // More stops on the curve than MAX_INK_TINTS: no mixes.
    let n = MAX_INK_TINTS + 1;
    g.stops = (0..n).map(|i| GradientStop::new(i as f32 / (n - 1) as f32, if i % 2 == 0 { magenta } else { Color::WHITE })).collect();
    assert_eq!(g.ink_mixes(), None);
}

/// From red to blue the display RGB mix darkens past blue's lightness and comes back, so a
/// weight found by lightness near the blue stop would be a red-side one and the inks would jump
/// there. The weights are the mix positions instead, and reach the blue stop's inks steadily.
#[test]
fn ink_mixes_use_the_mix_position_when_the_lightness_turns_back() {
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    let mut g = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]);
    g.method = GradientMethod::Perceptual;
    let mixes = g.ink_mixes().expect("a Perceptual gradient's curve");
    assert_eq!(mixes[mixes.len() - 1], (1.0, 1, 0.0, 1.0), "the blue stop");
    let between: Vec<_> = mixes.iter().filter(|m| m.1 == 0 && m.0 > 0.0).collect();
    assert!(!between.is_empty(), "stops on the curve between red and blue: {mixes:?}");
    // With the midpoint at ½, a stop's offset on the curve is its mix position.
    for (t, _, w, _) in &between {
        assert!((w - t).abs() < 1e-6, "at {t}: weight {w}");
    }
    assert!(mixes.windows(2).filter(|p| p[1].1 == 0).all(|p| p[0].2 <= p[1].2), "the weights rise: {mixes:?}");
}

fn cmyk_stop(offset: f32, c: f32, m: f32, y: f32, k: f32) -> GradientStop {
    GradientStop::new(offset, Color::cmyk(c, m, y, k))
}

/// The `n` × 1 grid of `g` as a left-to-right linear gradient's ink pixels.
fn ink_row(g: Gradient, n: u16, fill: bool) -> Vec<[u8; 5]> {
    let b = Rect::new(0.0, 0.0, f64::from(n), 1.0);
    GradientPaint::new(g).ink_pixels(b, b, n, 1, fill).expect("CMYK stops").collect()
}

#[test]
fn ink_pixels_mix_the_stops_inks_in_cmyk() {
    let mut g = Gradient::new(GradientKind::Linear, vec![cmyk_stop(0.0, 1.0, 0.0, 0.0, 0.0), cmyk_stop(1.0, 0.0, 0.0, 0.0, 1.0)]);
    g.stops[1].opacity = 0.5;
    let px = ink_row(g.clone(), 10, true);
    let at = |x: f32| (x * 255.0).round() as u8;
    for (i, p) in px.iter().enumerate() {
        let t = (i as f32 + 0.5) / 10.0;
        assert_eq!(*p, [at(1.0 - t), 0, 0, at(t), at(1.0 - t * 0.5)], "pixel {i}");
    }
    // An off-center midpoint is halfway between the two stops' inks, not a display RGB mix.
    g.stops[0].midpoint = 0.25;
    let px = ink_row(g.clone(), 4, false);
    assert_eq!(&px[0][..4], &[at(0.75), 0, 0, at(0.25)], "halfway to the midpoint");
    assert_eq!(&px[1][..4], &[at(0.5 * 5.0 / 6.0), 0, 0, at(0.5 + 0.5 / 6.0)], "a sixth of the way past it");
    // A hard stop keeps both sides.
    let hard = Gradient::new(
        GradientKind::Linear,
        vec![
            cmyk_stop(0.0, 1.0, 0.0, 0.0, 0.0),
            cmyk_stop(0.5, 1.0, 0.0, 0.0, 0.0),
            cmyk_stop(0.5, 0.0, 1.0, 0.0, 0.0),
            cmyk_stop(1.0, 0.0, 1.0, 0.0, 0.0),
        ],
    );
    let px = ink_row(hard, 4, false);
    assert_eq!((&px[1][..4], &px[2][..4]), (&[255, 0, 0, 0][..], &[0, 255, 0, 0][..]));
}

#[test]
fn ink_pixels_dither_only_the_inks_the_ramp_has() {
    let mut g = Gradient::new(GradientKind::Linear, vec![cmyk_stop(0.0, 0.2, 0.0, 0.0, 0.4), cmyk_stop(1.0, 0.8, 0.0, 0.0, 0.4)]);
    g.dither = true;
    let noisy = ink_row(g.clone(), 64, true);
    let plain = ink_row(g.clone(), 64, false);
    assert_ne!(noisy, plain, "a fill has noise, a stroke none");
    for (n, p) in noisy.iter().zip(&plain) {
        assert_eq!((n[1], n[2]), (0, 0), "no ink on the empty plates: {n:?}");
        assert!(n[0].abs_diff(p[0]) <= 1 && n[3].abs_diff(p[3]) <= 1, "{n:?} {p:?}");
        assert_eq!(n[4], 255, "opacity has no noise");
    }
    assert!(noisy.iter().zip(&plain).any(|(n, p)| n[3] != p[3]), "K gets its own noise");
    assert!(noisy.iter().zip(&plain).any(|(n, p)| (n[0] != p[0]) != (n[3] != p[3])), "the noise differs from channel to channel");
}

#[test]
fn ink_pixels_follow_the_perceptual_curve_as_ink_mixes() {
    let mut g = Gradient::new(GradientKind::Linear, vec![cmyk_stop(0.0, 0.0, 0.0, 0.0, 0.0), cmyk_stop(1.0, 1.0, 0.0, 0.0, 0.0)]);
    g.method = GradientMethod::Perceptual;
    let mixes = g.ink_mixes().unwrap();
    let ramp = g.ink_ramp().unwrap();
    assert_eq!(ramp.stops.len(), mixes.len());
    for ((t, ink), (o, i, w, _)) in ramp.stops.iter().zip(&mixes) {
        assert_eq!((*t, ink[0]), (*o, if *i == 0 { *w } else { 1.0 }), "cyan at {t}");
    }
    let px = ink_row(g, 2, false);
    assert!(px[0][0] > 0 && px[0][0] < px[1][0], "{px:?}");
    assert!(px.iter().all(|p| p[1] == 0 && p[2] == 0 && p[3] == 0), "{px:?}");
}

#[test]
fn ink_pixels_need_every_stop_in_cmyk() {
    let b = Rect::new(0.0, 0.0, 2.0, 1.0);
    let with = |stop: GradientStop| GradientPaint::new(Gradient::new(GradientKind::Linear, vec![cmyk_stop(0.0, 1.0, 0.0, 0.0, 0.0), stop]));
    assert!(with(cmyk_stop(1.0, 0.0, 0.0, 0.0, 1.0)).ink_pixels(b, b, 2, 1, true).is_some());
    for other in [Color::rgb(1.0, 0.0, 0.0), Color::gray(0.5), Color::lab(50.0, 10.0, 10.0)] {
        assert!(with(GradientStop::new(1.0, other)).ink_pixels(b, b, 2, 1, true).is_none(), "{other:?}");
    }
    assert!(GradientPaint::new(Gradient::new(GradientKind::Linear, vec![])).ink_pixels(b, b, 2, 1, true).is_none(), "no stops");
}

/// A Perceptual curve over the limit of [`Gradient::ink_mixes`] has no inks; a Classic gradient of as many stops does.
#[test]
fn ink_pixels_stop_at_the_ink_mixes_limit() {
    let stops: Vec<_> = (0..=MAX_INK_TINTS).map(|i| cmyk_stop(i as f32 / MAX_INK_TINTS as f32, (i % 2) as f32, 0.0, 0.0, 0.0)).collect();
    let classic = Gradient::new(GradientKind::Linear, stops);
    let perceptual = Gradient { method: GradientMethod::Perceptual, ..classic.clone() };
    assert!(!perceptual.fits_ink_tints());
    let b = Rect::new(0.0, 0.0, 2.0, 1.0);
    assert!(GradientPaint::new(perceptual).ink_pixels(b, b, 2, 1, false).is_none());
    assert!(GradientPaint::new(classic).ink_pixels(b, b, 2, 1, false).is_some());
}

/// A conic placed at `center` with its vector `dx` long to the right and aspect `aspect`.
fn conic_at(center: Point, dx: f64, aspect: f64) -> GradientGeom {
    GradientGeom { start: center, end: center + Vec2::new(dx, 0.0), aspect, focal: None }
}

#[test]
fn a_conic_turns_clockwise_on_the_page_from_its_vector() {
    // The page's y runs down: a quarter turn clockwise from a vector pointing right is straight
    // below the center.
    let g = conic_at(Point::new(100.0, 100.0), 50.0, 1.0);
    let t = |x: f64, y: f64| g.param_at(GradientKind::Conic, Point::new(x, y));
    assert!(t(200.0, 100.0).abs() < 1e-12);
    assert!((t(100.0, 130.0) - 0.25).abs() < 1e-12);
    assert!((t(10.0, 100.0) - 0.5).abs() < 1e-12);
    assert!((t(100.0, 3.0) - 0.75).abs() < 1e-12);
    // Either side of the seam: the start of the ramp just below the vector, its end just above.
    assert!(t(200.0, 100.01) < 1e-3);
    let before = t(200.0, 99.99);
    assert!(before > 0.999 && before <= 1.0, "{before}");
    // A flatter extent ellipse turns faster near the vector's line: half as tall, the point 1/8
    // of a turn around is half as far below it.
    let squashed = conic_at(Point::new(100.0, 100.0), 50.0, 0.5);
    let p = Point::new(150.0, 125.0);
    assert!((squashed.param_at(GradientKind::Conic, p) - 0.125).abs() < 1e-12);
    assert!(g.param_at(GradientKind::Conic, p) < 0.1);
    // Mirrored (a negative aspect), it turns counterclockwise.
    let mirrored = conic_at(Point::new(100.0, 100.0), 50.0, -1.0);
    assert!(mirrored.is_mirrored() && !g.is_mirrored());
    assert!((mirrored.param_at(GradientKind::Conic, Point::new(100.0, 70.0)) - 0.25).abs() < 1e-12);
}

#[test]
fn a_conic_follows_moves_turns_scales_and_mirrors() {
    let c = Point::new(10.0, 20.0);
    let g0 = conic_at(c, 30.0, 1.0);
    let probes = [c + Vec2::new(5.0, 7.0), c + Vec2::new(-9.0, 2.0), c + Vec2::new(3.0, -11.0)];
    for a in [
        Affine::rotate(0.7),
        Affine::scale(2.5),
        Affine::translate((4.0, -9.0)) * Affine::rotate(-2.0),
        Affine::FLIP_X,
        Affine::FLIP_Y * Affine::rotate(1.1),
        Affine::scale_non_uniform(2.0, 0.5),
    ] {
        let mut g = g0;
        g.transform(a, GradientKind::Conic);
        assert!(g.start.distance(a * c) < 1e-9, "{a:?}");
        for p in probes {
            let (want, got) = (g0.param_at(GradientKind::Conic, p), g.param_at(GradientKind::Conic, a * p));
            assert!((want - got).abs() < 1e-9, "{a:?} at {p:?}: {want} vs {got}");
        }
    }
    // A mirror and a mirror back: the same sweep.
    let mut g = g0;
    g.transform(Affine::FLIP_X, GradientKind::Conic);
    assert!(g.is_mirrored());
    g.transform(Affine::FLIP_X, GradientKind::Conic);
    assert!(!g.is_mirrored() && (g.aspect - 1.0).abs() < 1e-12);
    // A conic without a vector, or with a flat extent, turns the other way under a mirror too.
    for flat in [conic_at(c, 0.0, 1.0), conic_at(c, 30.0, 0.0)] {
        let mut g = flat;
        g.transform(Affine::FLIP_Y, GradientKind::Conic);
        assert!(g.is_mirrored(), "{g:?}");
        g.transform(Affine::rotate(0.3), GradientKind::Conic);
        assert!(g.is_mirrored(), "{g:?}");
        g.transform(Affine::FLIP_X, GradientKind::Conic);
        assert!(!g.is_mirrored(), "{g:?}");
    }
    // Under a shear the seam stays where the vector maps.
    let shear = Affine::new([1.0, 0.0, 0.6, 1.0, 0.0, 0.0]);
    let mut g = g0;
    g.transform(shear, GradientKind::Conic);
    let on_seam = shear * (c + Vec2::new(20.0, 0.0));
    assert!(g.param_at(GradientKind::Conic, on_seam) < 1e-9, "{g:?}");
}

#[test]
fn conic_kind_names_and_fit() {
    for s in ["conic", "Conic", "angle", "CONICAL"] {
        assert_eq!(GradientKind::parse(s), Some(GradientKind::Conic), "{s}");
    }
    assert_eq!(GradientKind::parse("spiral"), None);
    assert_eq!(GradientKind::Conic.label(), "Conic");
    assert!(GradientKind::Conic.is_centered() && GradientKind::Radial.is_centered() && !GradientKind::Linear.is_centered());
    let fit = GradientGeom::fit(GradientKind::Conic, Rect::new(0.0, 0.0, 200.0, 100.0), 90.0);
    assert_eq!(fit.start, Point::new(100.0, 50.0));
    assert!(fit.end.distance(Point::new(100.0, -50.0)) < 1e-9);
}

/// Is `p` inside polygon `poly` (non-zero winding)?
fn inside(poly: &[Point], p: Point) -> bool {
    let mut w = 0i32;
    for (i, &a) in poly.iter().enumerate() {
        let c = poly[(i + 1) % poly.len()];
        let cross = (c - a).cross(p - a);
        if a.y <= p.y && c.y > p.y && cross > 0.0 {
            w += 1;
        } else if a.y > p.y && c.y <= p.y && cross < 0.0 {
            w -= 1;
        }
    }
    w != 0
}

#[test]
fn conic_wedges_cover_the_box_and_meet_at_the_seam() {
    let b = Rect::new(0.0, 0.0, 200.0, 100.0);
    let g = conic_at(Point::new(60.0, 40.0), 30.0, 0.5);
    let wedges = g.conic_wedges(b, 36, false);
    assert_eq!(wedges.len(), 36);
    // Each point of the box lies in the wedge whose share of the turn holds it.
    for (x, y) in [(1.0, 1.0), (199.0, 99.0), (150.0, 30.0), (10.0, 90.0), (61.0, 39.0), (59.0, 41.0)] {
        let p = Point::new(x, y);
        let t = g.param_at(GradientKind::Conic, p);
        let k = ((t * 36.0).floor() as usize).min(35);
        assert!(inside(&wedges[k].0, p), "({x}, {y}) at {t} is not in wedge {k}");
        assert!((wedges[k].1 - (k as f64 + 0.5) / 36.0).abs() < 1e-12);
    }
    // Overlapping wedges run on from their own start to the seam.
    let over = g.conic_wedges(b, 36, true);
    let frame = g.unit_frame().unwrap();
    let on = |turn: f64| frame * Point::new(0.5 * (turn * std::f64::consts::TAU).cos(), 0.5 * (turn * std::f64::consts::TAU).sin());
    assert!(inside(&over[0].0, on(0.5)) && inside(&over[0].0, on(35.9 / 36.0)));
    assert!(inside(&over[20].0, on(35.9 / 36.0)) && !inside(&over[20].0, on(19.9 / 36.0)));
    // The count is kept to what a file can hold.
    assert_eq!(g.conic_wedges(b, 0, false).len(), 1);
    assert_eq!(g.conic_wedges(b, usize::MAX, false).len(), MAX_CONIC_WEDGES);
}

/// A conic whose extent can't be drawn (a vector without length, an aspect of 0, or numbers out
/// of range, as a file can hold them) sweeps from the right about its start, or the origin; the
/// parameter, the frame and the wedges stay finite.
#[test]
fn a_conic_with_no_extent_or_bad_numbers_still_sweeps() {
    let c = Point::new(50.0, 50.0);
    for g in [
        conic_at(c, 0.0, 1.0),
        conic_at(c, 30.0, 0.0),
        conic_at(c, 30.0, f64::NAN),
        conic_at(c, 30.0, f64::INFINITY),
        conic_at(c, f64::INFINITY, 1.0),
        GradientGeom { end: Point::new(f64::NAN, 3.0), ..conic_at(c, 30.0, 1.0) },
    ] {
        assert!(g.conic_frame().as_coeffs().iter().all(|v| v.is_finite()), "{g:?}");
        let t = |x: f64, y: f64| g.param_at(GradientKind::Conic, Point::new(x, y));
        assert!(t(90.0, 50.0).abs() < 1e-12, "{g:?}");
        assert!((t(50.0, 90.0) - 0.25).abs() < 1e-12, "{g:?}");
        let wedges = g.conic_wedges(Rect::new(0.0, 0.0, 100.0, 100.0), 4, false);
        assert!(wedges.iter().flat_map(|(pts, _)| pts).all(|p| p.is_finite()), "{g:?}");
    }
    // A flat extent keeps the vector's direction and the sign of its aspect.
    let down = GradientGeom { end: Point::new(50.0, 80.0), ..conic_at(c, 30.0, 0.0) };
    assert!(down.param_at(GradientKind::Conic, Point::new(50.0, 90.0)).abs() < 1e-12);
    assert!((down.param_at(GradientKind::Conic, Point::new(10.0, 50.0)) - 0.25).abs() < 1e-12);
    for g in [conic_at(c, 0.0, -1.0), conic_at(c, 30.0, -0.0), conic_at(c, 30.0, f64::NEG_INFINITY)] {
        assert!(g.is_mirrored(), "{g:?}");
        assert!((g.param_at(GradientKind::Conic, Point::new(50.0, 10.0)) - 0.25).abs() < 1e-12, "{g:?}");
    }
    let lost = GradientGeom { start: Point::new(f64::NAN, 0.0), ..conic_at(c, 30.0, 1.0) };
    assert_eq!(lost.conic_frame(), Affine::IDENTITY);
    for p in [Point::new(3.0, 4.0), Point::new(f64::NAN, 1.0), Point::new(f64::INFINITY, 0.0)] {
        let t = lost.param_at(GradientKind::Conic, p);
        assert!((0.0..=1.0).contains(&t), "{t}");
    }
}

/// A conic's pixels (what SVG and PDF draw a dithered or Perceptual one as) follow the sweep,
/// with one stop as well as many.
#[test]
fn conic_pixels_follow_the_sweep() {
    let b = Rect::new(0.0, 0.0, 4.0, 4.0);
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Conic, Gradient::default().stops));
    g.geom = Some(conic_at(Point::new(2.0, 2.0), 1.0, 1.0));
    let px: Vec<[u8; 4]> = g.pixels(b, b, 4, 4, true, &Color::to_rgb).collect();
    // Below right of the center, early in the sweep: light. Above right, late: dark.
    assert!(px[3 * 4 + 3][0] > 180 && px[3][0] < 75, "{px:?}");
    let one = GradientPaint { gradient: Gradient::new(GradientKind::Conic, vec![GradientStop::new(0.3, Color::rgb(1.0, 0.0, 0.0))]), ..g.clone() };
    assert!(one.pixels(b, b, 4, 4, true, &Color::to_rgb).all(|p| p == [255, 0, 0, 255]));
    let many = (0..=1000).map(|i| GradientStop::new(i as f32 / 1000.0, Color::gray((i % 2) as f32))).collect();
    let many = GradientPaint { gradient: Gradient::new(GradientKind::Conic, many), ..g };
    assert_eq!(many.pixels(b, b, 4, 4, true, &Color::to_rgb).count(), 16);
}

/// A diamond placed at `center` with its vector `dx` long to the right and aspect `aspect`.
fn diamond_at(center: Point, dx: f64, aspect: f64) -> GradientGeom {
    GradientGeom { start: center, end: center + Vec2::new(dx, 0.0), aspect, focal: None }
}

#[test]
fn a_diamond_runs_from_its_center_out_to_a_square_with_a_corner_at_the_end_of_its_vector() {
    let g = diamond_at(Point::new(100.0, 100.0), 50.0, 1.0);
    let t = |x: f64, y: f64| g.param_at(GradientKind::Diamond, Point::new(x, y));
    assert!(t(100.0, 100.0).abs() < 1e-12);
    // The square's corners: at the end of the vector and a quarter turn on from it each way.
    for (x, y) in [(150.0, 100.0), (100.0, 150.0), (50.0, 100.0), (100.0, 50.0)] {
        assert!((t(x, y) - 1.0).abs() < 1e-12, "({x}, {y})");
    }
    // Halfway along an edge, and halfway out along a diagonal (the middle of the edge's half size).
    assert!((t(125.0, 125.0) - 1.0).abs() < 1e-12);
    assert!((t(112.5, 87.5) - 0.5).abs() < 1e-12);
    // Past the square: more than 1 (the last stop pads on).
    assert!((t(200.0, 200.0) - 4.0).abs() < 1e-12);
    // The aspect stretches the square across the vector into a rhombus.
    let tall = diamond_at(Point::new(100.0, 100.0), 50.0, 2.0);
    assert!((tall.param_at(GradientKind::Diamond, Point::new(100.0, 200.0)) - 1.0).abs() < 1e-12);
    assert!((tall.param_at(GradientKind::Diamond, Point::new(150.0, 100.0)) - 1.0).abs() < 1e-12);
    // Turned: the corner follows the vector.
    let turned = GradientGeom { end: Point::new(130.0, 140.0), ..g };
    assert!((turned.param_at(GradientKind::Diamond, Point::new(130.0, 140.0)) - 1.0).abs() < 1e-12);
    assert!((turned.param_at(GradientKind::Diamond, Point::new(115.0, 120.0)) - 0.5).abs() < 1e-12);
}

#[test]
fn diamond_kind_names_and_fit() {
    for s in ["diamond", "Diamond", "DIAMOND"] {
        assert_eq!(GradientKind::parse(s), Some(GradientKind::Diamond), "{s}");
    }
    assert_eq!(GradientKind::Diamond.label(), "Diamond");
    assert!(GradientKind::Diamond.is_centered());
    assert!(GradientKind::ALL.contains(&GradientKind::Diamond));
    // Level: the square's edges pass through the box's corners, so it covers the box exactly.
    let b = Rect::new(0.0, 0.0, 200.0, 100.0);
    let fit = GradientGeom::fit(GradientKind::Diamond, b, 0.0);
    assert_eq!(fit.start, Point::new(100.0, 50.0));
    assert!(fit.end.distance(Point::new(250.0, 50.0)) < 1e-9, "{fit:?}");
    for c in [Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 100.0), Point::new(0.0, 100.0)] {
        assert!((fit.param_at(GradientKind::Diamond, c) - 1.0).abs() < 1e-9, "{c:?}");
    }
    // At any angle the square covers the box and runs through its farthest corner, on a square
    // box too.
    for b in [b, Rect::new(0.0, 0.0, 100.0, 100.0)] {
        for angle in [30.0, 45.0, 90.0, -120.0] {
            let fit = GradientGeom::fit(GradientKind::Diamond, b, angle);
            let levels = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]
                .map(|c| fit.param_at(GradientKind::Diamond, c));
            assert!(levels.iter().all(|&t| t <= 1.0 + 1e-9), "{angle}: {levels:?}");
            assert!(levels.iter().any(|&t| (t - 1.0).abs() < 1e-9), "{angle}: {levels:?}");
        }
    }
}

#[test]
fn a_diamond_follows_moves_turns_scales_and_mirrors() {
    let c = Point::new(10.0, 20.0);
    let g0 = diamond_at(c, 30.0, 0.5);
    let probes = [c + Vec2::new(5.0, 7.0), c + Vec2::new(-9.0, 2.0), c + Vec2::new(3.0, -11.0), c + Vec2::new(-20.0, -4.0)];
    for a in [
        Affine::rotate(0.7),
        Affine::scale(2.5),
        Affine::translate((4.0, -9.0)) * Affine::rotate(-2.0),
        Affine::FLIP_X,
        Affine::FLIP_Y * Affine::rotate(1.1),
        // Along and across the vector.
        Affine::scale_non_uniform(2.0, 0.5),
        Affine::scale_non_uniform(-3.0, 0.25),
    ] {
        let mut g = g0;
        g.transform(a, GradientKind::Diamond);
        assert!(g.start.distance(a * c) < 1e-9, "{a:?}");
        // A mirror changes nothing a diamond shows: its aspect stays a size.
        assert!(g.aspect > 0.0, "{a:?}: {g:?}");
        for p in probes {
            let (want, got) = (g0.param_at(GradientKind::Diamond, p), g.param_at(GradientKind::Diamond, a * p));
            assert!((want - got).abs() < 1e-9, "{a:?} at {p:?}: {want} vs {got}");
        }
    }
    // Under a shear the corner stays where the vector maps.
    let shear = Affine::new([1.0, 0.0, 0.6, 1.0, 0.0, 0.0]);
    let mut g = g0;
    g.transform(shear, GradientKind::Diamond);
    assert!((g.param_at(GradientKind::Diamond, shear * (c + Vec2::new(30.0, 0.0))) - 1.0).abs() < 1e-9, "{g:?}");
    // Out-of-range numbers: a finite aspect after any transform.
    for bad in [diamond_at(c, 30.0, f64::NAN), diamond_at(c, f64::INFINITY, 1.0), diamond_at(c, 0.0, 1.0)] {
        let mut g = bad;
        g.transform(Affine::rotate(0.4), GradientKind::Diamond);
        assert!(g.aspect.is_finite(), "{g:?}");
    }
}

/// A diamond without a frame (no vector, a flat or a not-finite extent) has no inside: every
/// point is past its edge and takes the last stop. Points that aren't numbers do too.
#[test]
fn a_diamond_with_no_extent_or_bad_numbers_is_its_last_stop() {
    let c = Point::new(50.0, 50.0);
    for g in [
        diamond_at(c, 0.0, 1.0),
        diamond_at(c, 30.0, 0.0),
        diamond_at(c, 30.0, f64::NAN),
        diamond_at(c, f64::INFINITY, 1.0),
        GradientGeom { start: Point::new(f64::NAN, 0.0), ..diamond_at(c, 30.0, 1.0) },
    ] {
        assert!(g.diamond_frame().is_none(), "{g:?}");
        assert!(g.diamond_quadrants(Rect::new(0.0, 0.0, 100.0, 100.0)).is_none(), "{g:?}");
        assert_eq!(g.param_at(GradientKind::Diamond, Point::new(60.0, 40.0)), 1.0, "{g:?}");
    }
    let g = diamond_at(c, 30.0, 1.0);
    assert_eq!(g.param_at(GradientKind::Diamond, Point::new(f64::NAN, 1.0)), 1.0);
    assert!(g.param_at(GradientKind::Diamond, Point::new(f64::INFINITY, 1.0)) >= 1.0);
}

/// The four linear gradients a format without diamond gradients gets, drawn in order, equal the
/// diamond everywhere, and each edge drawn lies on the quadrant drawn before it.
#[test]
fn diamond_quadrants_are_linear_gradients_that_equal_the_diamond() {
    let b = Rect::new(0.0, 0.0, 200.0, 100.0);
    let probes: Vec<Point> = (0..=20).flat_map(|i| (0..=10).map(move |j| Point::new(i as f64 * 10.0 + 0.3, j as f64 * 10.0 + 0.7))).collect();
    // Squashed and turned, stretched twice as tall as wide, and stretched by a shear.
    let mut turned = diamond_at(Point::new(60.0, 40.0), 30.0, 0.5);
    turned.transform(Affine::rotate_about(0.4, Point::new(60.0, 40.0)), GradientKind::Diamond);
    let mut sheared = diamond_at(Point::new(90.0, 50.0), 25.0, 1.0);
    sheared.transform(Affine::new([2.0, 0.0, 0.7, 1.0, -60.0, 0.0]), GradientKind::Diamond);
    for g in [turned, diamond_at(Point::new(100.0, 50.0), 20.0, 2.0), sheared] {
        let quadrants = g.diamond_quadrants(b).expect("a frame");
        for p in &probes {
            // The last quadrant drawn that holds the point paints it.
            let Some((_, linear)) = quadrants.iter().rev().find(|(poly, _)| inside(poly, *p)) else { panic!("{p:?} is in no quadrant") };
            let (want, got) = (g.param_at(GradientKind::Diamond, *p), linear.param_at(GradientKind::Linear, *p));
            assert!((want - got).abs() < 1e-9, "{g:?} at {p:?}: {want} vs {got}");
        }
    }
    let g = turned;
    let quadrants = g.diamond_quadrants(b).expect("a frame");
    // Next to each edge of a quadrant drawn later, on either side, the quadrant drawn before it
    // lies underneath (the first quadrant's edges are covered by the second and the last).
    let frame = g.diamond_frame().expect("a frame");
    for (unit, under) in [
        // The second quadrant's first edge, on the first.
        (Point::new(0.01, 0.5), 0),
        (Point::new(-0.01, 0.5), 0),
        // The third's, on the second.
        (Point::new(-0.5, 0.01), 1),
        (Point::new(-0.5, -0.01), 1),
        // The last's two, on the third and on the first.
        (Point::new(-0.01, -0.5), 2),
        (Point::new(0.01, -0.5), 2),
        (Point::new(0.5, -0.01), 0),
        (Point::new(0.5, 0.01), 0),
    ] {
        assert!(inside(&quadrants[under].0, frame * unit), "{unit:?} lies on quadrant {under}");
    }
}

/// A diamond's pixels (what SVG and PDF draw a dithered or Perceptual one as, and what the canvas
/// draws) follow the squares.
#[test]
fn diamond_pixels_follow_the_squares() {
    let b = Rect::new(0.0, 0.0, 5.0, 5.0);
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Diamond, Gradient::default().stops));
    g.geom = Some(diamond_at(Point::new(2.5, 2.5), 2.5, 1.0));
    let px: Vec<[u8; 4]> = g.pixels(b, b, 5, 5, false, &Color::to_rgb).collect();
    // White at the center, black in the corners, and the same on each side of it.
    assert!(px[2 * 5 + 2][0] > 200 && px[0][0] < 10 && px[24][0] < 10, "{px:?}");
    assert_eq!(px[2 * 5 + 1], px[2 * 5 + 3]);
    assert_eq!(px[5 + 2], px[3 * 5 + 2]);
    assert_eq!(px[2 * 5 + 1], px[5 + 2]);
}
