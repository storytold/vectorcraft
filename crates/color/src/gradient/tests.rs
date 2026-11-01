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
    let tints = g.ink_tints(&ink).expect("a magenta ink's tints differ in lightness");
    assert!(tints.len() > 2, "the curve's stops: {tints:?}");
    let (first, last) = (tints[0], tints[tints.len() - 1]);
    assert!((first.0, last.0) == (0.0, 1.0) && (first.1 - 0.8).abs() < 1e-4 && last.1 < 1e-4, "{tints:?}");
    assert!(tints.windows(2).all(|w| w[0].0 <= w[1].0 && w[0].1 >= w[1].1), "in order, lighter along the way: {tints:?}");
    let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
    for (t, tint, _) in &tints {
        let want = lightness(&g.sample(*t).0);
        assert!((lightness(&ink.tinted(*tint)) - want).abs() < 1e-3, "at {t}: tint {tint}");
    }
    let mid = tints.iter().find(|s| (s.0 - 0.5).abs() < 1e-6).expect("the midpoint");
    assert!((mid.1 - 0.4).abs() > 0.01, "not halfway in tint: {mid:?}");
    // A white ink's tints all have paper white's lightness: no tints by lightness.
    assert_eq!(g.ink_tints(&Color::WHITE), None);
    // More stops on the curve than MAX_INK_TINTS: no tints.
    let n = MAX_INK_TINTS + 1;
    g.stops = (0..n).map(|i| GradientStop::new(i as f32 / (n - 1) as f32, if i % 2 == 0 { ink } else { Color::WHITE })).collect();
    assert_eq!(g.ink_tints(&ink), None);
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
