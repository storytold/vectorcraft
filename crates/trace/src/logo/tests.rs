//! End-to-end tests: synthetic logos made in code, with known geometry, traced and measured.

use crate::{Method, Mode, Raster, TraceError, TraceParams, TraceResult, trace_within};
use kurbo::{ParamCurve, Shape};
use vectorcraft_geom::Point;

/// A soft, anti-aliased render of `shade` (`Some(colour)` where the shape is), supersampled `ss × ss`
/// and blurred by `blur` pixels, on a transparent background.
fn render(w: u32, h: u32, blur: f64, shade: impl Fn(f64, f64) -> Option<[u8; 3]>) -> Raster {
    let ss = 8usize;
    let (wu, hu) = (w as usize, h as usize);
    let mut px = vec![[0.0f64; 4]; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            let mut acc = [0.0f64; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let (fx, fy) = (x as f64 + (sx as f64 + 0.5) / ss as f64, y as f64 + (sy as f64 + 0.5) / ss as f64);
                    if let Some(c) = shade(fx, fy) {
                        acc[0] += f64::from(c[0]) / 255.0;
                        acc[1] += f64::from(c[1]) / 255.0;
                        acc[2] += f64::from(c[2]) / 255.0;
                        acc[3] += 1.0;
                    }
                }
            }
            let n = (ss * ss) as f64;
            px[y * wu + x] = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
        }
    }
    if blur > 0.0 {
        let r = (3.0 * blur).ceil() as i64;
        let k: Vec<f64> = (-r..=r).map(|i| (-0.5 * (i as f64 / blur).powi(2)).exp()).collect();
        let sum: f64 = k.iter().sum();
        let k: Vec<f64> = k.iter().map(|v| v / sum).collect();
        for horizontal in [true, false] {
            let src = px.clone();
            for y in 0..hu as i64 {
                for x in 0..wu as i64 {
                    let mut acc = [0.0f64; 4];
                    for (j, kv) in k.iter().enumerate() {
                        let d = j as i64 - r;
                        let (xx, yy) = if horizontal { ((x + d).clamp(0, wu as i64 - 1), y) } else { (x, (y + d).clamp(0, hu as i64 - 1)) };
                        let p = src[(yy as usize) * wu + xx as usize];
                        for c in 0..4 {
                            acc[c] += kv * p[c];
                        }
                    }
                    px[(y as usize) * wu + x as usize] = acc;
                }
            }
        }
    }
    let mut rgba = Vec::with_capacity(wu * hu * 4);
    for p in px {
        let a = p[3];
        let un = |v: f64| if a > 1e-9 { ((v / a) * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    Raster::new(w, h, rgba).unwrap()
}

const RED: [u8; 3] = [210, 40, 50];
const BLUE: [u8; 3] = [40, 80, 200];

fn logo_params() -> TraceParams {
    TraceParams {
        mode: Mode::Logo,
        colors: 6,
        paths: 60.0,
        corners: 50.0,
        noise: 6,
        snap_curves_to_lines: true,
        method: Method::Abutting,
        ..TraceParams::default()
    }
}

fn trace(img: &Raster, p: &TraceParams) -> TraceResult {
    trace_within(img, p, 100_000).expect("trace")
}

fn samples(res: &TraceResult) -> Vec<Point> {
    let mut out = Vec::new();
    for p in &res.paths {
        for seg in p.path.to_bezpath().segments() {
            out.extend((0..=40).map(|i| seg.eval(f64::from(i) / 40.0)));
        }
    }
    out
}

fn covered(res: &TraceResult, p: Point) -> bool {
    res.paths.iter().any(|t| t.path.to_bezpath().winding(p) != 0)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= tol)
}

#[test]
fn a_blurred_disc_becomes_a_true_circle_within_a_tenth_of_a_pixel() {
    let (cx, cy, r) = (31.7, 32.4, 19.3);
    let img = render(64, 64, 0.7, |x, y| ((x - cx).hypot(y - cy) <= r).then_some(RED));
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1, "{} paths", res.paths.len());
    assert!(near(res.paths[0].color, RED, 3), "colour {:?}", res.paths[0].color);
    let errs: Vec<f64> = samples(&res).iter().map(|p| (p.distance(Point::new(cx, cy)) - r).abs()).collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    let max = errs.iter().copied().fold(0.0, f64::max);
    assert!(mean < 0.06 && max < 0.15, "mean {mean:.3} max {max:.3}");
    assert!(res.anchor_count() <= 6, "{} anchors for a circle", res.anchor_count());
}

#[test]
fn a_blurred_rectangle_keeps_square_corners_and_four_anchors() {
    let (x0, x1, y0, y1) = (10.4, 77.9, 8.7, 38.2);
    let img = render(90, 48, 0.8, |x, y| ((x0..=x1).contains(&x) && (y0..=y1).contains(&y)).then_some(BLUE));
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.anchor_count(), 4, "a rectangle is four corners");
    let b = res.paths[0].path.bounds().expect("bounds");
    for (got, want, what) in [(b.x0, x0, "left"), (b.x1, x1, "right"), (b.y0, y0, "top"), (b.y1, y1, "bottom")] {
        // Sub-pixel: within 0.15 px of the true edge, from a blurred 90 px image.
        assert!((got - want).abs() < 0.15, "{what} edge {got:.3} vs {want}");
    }
}

#[test]
fn touching_colours_leave_no_gap_between_them() {
    let edge = 40.35;
    let img = render(80, 50, 0.7, |x, y| {
        if !(10.2..=40.6).contains(&y) {
            None
        } else if (10.3..=edge).contains(&x) {
            Some(RED)
        } else if (edge..=70.6).contains(&x) {
            Some(BLUE)
        } else {
            None
        }
    });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 2);
    assert!(!near(res.paths[0].color, res.paths[1].color, 20));
    for i in 0..200 {
        let y = 12.0 + 27.0 * f64::from(i) / 199.0;
        for dx in [-0.03, 0.0, 0.03] {
            assert!(covered(&res, Point::new(edge + dx, y)), "gap at ({}, {y})", edge + dx);
        }
    }
}

#[test]
fn letters_knocked_out_of_a_badge_become_white_or_holes() {
    let ring = |x: f64, y: f64| {
        let d = (x - 32.0).hypot(y - 32.0);
        (12.0..=25.0).contains(&d).then_some(BLUE)
    };
    let img = render(64, 64, 0.7, ring);
    // Default: the knockout is painted white, on top of the badge.
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 2, "badge + white knockout");
    assert!(res.paths.iter().any(|p| p.color == [255, 255, 255]));
    assert!(covered(&res, Point::new(32.0, 32.0)));
    // Ignore white: the knockout is a real hole in the badge.
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].path.subpaths.len(), 2, "outer outline and a hole");
    assert!(!covered(&res, Point::new(32.0, 32.0)), "the hole must be empty");
    assert!(covered(&res, Point::new(32.0, 32.0 + 18.0)), "the ring itself is filled");
}

#[test]
fn a_gradient_is_refused_not_posterised() {
    let img = Raster::from_fn(64, 32, |x, y| [(x * 4) as u8, 255 - (x * 4) as u8, (y * 6) as u8, 255]);
    match trace_within(&img, &logo_params(), 100_000) {
        Err(TraceError::NotFlat { fraction }) => assert!(fraction < 0.93),
        other => panic!("expected NotFlat, got {other:?}"),
    }
    let msg = TraceError::NotFlat { fraction: 0.69 }.to_string();
    assert!(msg.contains("69%") && msg.contains("Color mode"), "{msg}");
}

#[test]
fn a_white_background_logo_drops_the_white_when_asked() {
    let img = Raster::from_fn(80, 60, |x, y| {
        let d = (x as f64 + 0.5 - 40.0).hypot(y as f64 + 0.5 - 30.0);
        if (10.0..=24.0).contains(&d) { [20, 20, 20, 255] } else { [255, 255, 255, 255] }
    });
    let kept = trace(&img, &logo_params());
    assert!(kept.paths.iter().any(|p| p.color == [255, 255, 255]), "white background kept by default");
    let dropped = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert!(dropped.paths.iter().all(|p| p.color != [255, 255, 255]));
    assert!(!dropped.paths.is_empty());
}

#[test]
fn the_named_palette_is_used_exactly_and_a_bad_one_is_an_error() {
    let img = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(RED));
    let named = TraceParams { logo_colors: vec!["#d22832".into()], ..logo_params() };
    let res = trace(&img, &named);
    assert_eq!(res.paths[0].color, [0xd2, 0x28, 0x32]);
    let bad = TraceParams { logo_colors: vec!["red".into()], ..logo_params() };
    assert!(matches!(trace_within(&img, &bad, 1000), Err(TraceError::BadPalette(s)) if s == "red"));
}

#[test]
fn a_large_image_is_traced_at_reduced_size_and_scaled_back() {
    let (cx, cy, r) = (1200.0, 600.0, 400.0);
    let img = Raster::from_fn(2400, 1200, |x, y| {
        if (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) <= r { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] }
    });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    let b = res.paths[0].path.bounds().expect("bounds");
    for (got, want) in [(b.x0, cx - r), (b.x1, cx + r), (b.y0, cy - r), (b.y1, cy + r)] {
        assert!((got - want).abs() < 2.0, "{got} vs {want}");
    }
}

/// A light square inset 50 px with slanted 2 px black lines across it, on a transparent background.
fn lined_square(n: u32) -> Raster {
    Raster::from_fn(n, n, |x, y| {
        if !(50..n - 50).contains(&x) || !(50..n - 50).contains(&y) {
            [0, 0, 0, 0]
        } else if (i64::from(x) - i64::from(y) / 8).rem_euclid(40) < 2 {
            [0, 0, 0, 255]
        } else {
            [240, 240, 220, 255]
        }
    })
}

#[test]
fn a_trace_averaged_down_says_the_size_it_worked_at() {
    let named = TraceParams { logo_colors: vec!["#000000".into(), "#F0F0DC".into()], ..logo_params() };
    // 1025 px is over the working size: averaged down by 2, to 513 px.
    let res = trace(&lined_square(1025), &named);
    assert!(!res.paths.is_empty());
    assert_eq!(res.worked_at, Some((513, 513)));
    let res = trace(&lined_square(1024), &named);
    assert!(!res.paths.is_empty());
    assert_eq!(res.worked_at, None);
}

#[test]
fn named_colors_a_few_levels_off_trace_an_image_up_to_1024_px_at_full_size() {
    // A light disc in a dark ring with a blue square on it, hard edges, 800 px square, and a dark
    // 2 px dot on the disc at odd pixels: averaged down by 2, the dot would be a quarter of each of
    // four pixels and leave no shape.
    let img = Raster::from_fn(800, 800, |x, y| {
        let (fx, fy) = (f64::from(x) / 5.0, f64::from(y) / 5.0);
        let d = (fx - 80.0).hypot(fy - 80.0);
        if (401..=402).contains(&x) && (201..=202).contains(&y) {
            [4, 4, 3, 255]
        } else if (70.0..=110.0).contains(&fx) && (70.0..=110.0).contains(&fy) {
            [34, 124, 250, 255]
        } else if d <= 60.0 {
            [252, 248, 227, 255]
        } else if d <= 68.0 {
            [4, 4, 3, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    for (off, names) in [(8, ["#0C0C0B", "#F4F0DB", "#1A74F2"]), (12, ["#10100F", "#F0ECD7", "#1670EE"])] {
        let p = TraceParams { logo_colors: names.iter().map(|s| s.to_string()).collect(), ..logo_params() };
        let res = trace(&img, &p);
        let color_at = |p: Point| res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color);
        assert_eq!(res.paths.len(), 4, "names {off} levels off");
        let (dark, cream, blue) = ([4 + off, 4 + off, 3 + off], [252 - off, 248 - off, 227 - off], [34 - off, 124 - off, 250 - off]);
        assert_eq!(color_at(Point::new(402.0, 202.0)), Some(dark), "names {off} levels off");
        assert_eq!(color_at(Point::new(450.0, 450.0)), Some(blue), "names {off} levels off");
        assert_eq!(color_at(Point::new(200.0, 400.0)), Some(cream), "names {off} levels off");
    }
}

/// Every anchor of every path is finite and inside (a small margin around) the image.
fn assert_sane(res: &TraceResult, w: u32, h: u32) {
    for t in &res.paths {
        for sp in &t.path.subpaths {
            for a in &sp.anchors {
                assert!(a.p.x.is_finite() && a.p.y.is_finite() && a.h_in.x.is_finite() && a.h_out.y.is_finite(), "non-finite anchor");
                assert!(a.p.x > -2.0 && a.p.y > -2.0 && a.p.x < f64::from(w) + 2.0 && a.p.y < f64::from(h) + 2.0, "anchor {:?} outside {w}x{h}", a.p);
            }
        }
    }
}

#[test]
fn degenerate_images_are_fine() {
    let p = logo_params();
    for (w, h, img) in [
        (0, 0, Raster::new(0, 0, vec![]).unwrap()),
        (1, 1, Raster::new(1, 1, vec![200, 30, 30, 255]).unwrap()),
        (3, 3, Raster::new(3, 3, vec![0; 36]).unwrap()),
        (2, 2, Raster::new(2, 2, vec![255; 16]).unwrap()),
        (5, 1, Raster::new(5, 1, vec![9; 20]).unwrap()),
        (1, 7, Raster::new(1, 7, vec![130; 28]).unwrap()),
    ] {
        match trace_within(&img, &p, 1000) {
            Ok(res) => assert_sane(&res, w, h),
            Err(TraceError::NotFlat { .. }) => {}
            Err(e) => panic!("unexpected error for {w}x{h}: {e}"),
        }
    }
}

#[test]
fn extreme_parameters_never_panic() {
    let img = render(48, 48, 0.7, |x, y| ((x - 24.0).hypot(y - 24.0) <= 15.0).then_some(RED));
    let weird = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -50.0, 0.0, 100.0, 1e9];
    for &paths in &weird {
        for &corners in &weird {
            let p = TraceParams { paths, corners, ..logo_params() };
            let _ = trace_within(&img, &p, 10_000);
        }
    }
    for colors in [0, 1, 2, 300, u32::MAX] {
        for noise in [0, 1, 1_000_000, u32::MAX] {
            let p = TraceParams { colors, noise, ..logo_params() };
            let _ = trace_within(&img, &p, 10_000);
        }
    }
    // A tiny anchor budget is reported as too complex, not exceeded.
    let r = trace_within(&img, &logo_params(), 0);
    assert!(r.is_err() || r.is_ok_and(|t| t.anchor_count() == 0));
}

#[test]
fn noise_specks_are_dropped() {
    let img = render(64, 64, 0.5, |x, y| if (x - 32.0).hypot(y - 32.0) <= 18.0 || (x - 6.0).hypot(y - 6.0) <= 0.6 { Some(RED) } else { None });
    let res = trace(&img, &TraceParams { noise: 20, ..logo_params() });
    assert_eq!(res.paths.len(), 1, "the speck should be gone");
}

#[test]
fn nested_rings_keep_their_own_holes_under_ignore_white() {
    // Red ring, white band, red ring, white centre.
    let img = render(80, 80, 0.7, |x, y| {
        let d = (x - 40.0).hypot(y - 40.0);
        ((22.0..=28.0).contains(&d) || (8.0..=16.0).contains(&d)).then_some(RED)
    });
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert_eq!(res.paths.len(), 2, "two red rings");
    for p in &res.paths {
        assert_eq!(p.path.subpaths.len(), 2, "each ring is an outline and its own hole, no more");
    }
    assert!(!covered(&res, Point::new(40.0, 40.0)), "centre is empty");
    assert!(covered(&res, Point::new(52.0, 40.0)), "inner ring is filled");
    assert!(!covered(&res, Point::new(59.0, 40.0)), "the band between the rings is empty");
    assert!(covered(&res, Point::new(65.0, 40.0)), "outer ring is filled");
}

#[test]
fn too_many_outlines_are_refused_not_silently_truncated() {
    let specks =
        |x: f64, y: f64| (x.rem_euclid(5.0) >= 1.0 && x.rem_euclid(5.0) < 4.0 && y.rem_euclid(5.0) >= 1.0 && y.rem_euclid(5.0) < 4.0).then_some(RED);
    let img = render(200, 200, 0.0, specks);
    assert!(matches!(trace_within(&img, &logo_params(), 1_000_000), Err(TraceError::TooComplex { .. })));
    // Specks under the noise threshold do not count towards the limit.
    let res = trace_within(&img, &TraceParams { noise: 200, ..logo_params() }, 1_000_000).expect("trace");
    assert!(res.paths.is_empty());
}

#[test]
fn a_large_odd_sized_image_never_overshoots_its_edges() {
    // 1025 wide: the last working pixel is narrower than the others.
    let img = Raster::from_fn(1025, 20, |x, y| if x >= 600 && (2..18).contains(&y) { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    let b = res.paths[0].path.bounds().expect("bounds");
    assert!(b.x1 <= 1025.15 && b.x1 >= 1024.7, "right edge {}", b.x1);
    assert!((b.x0 - 600.0).abs() < 0.6 && (b.y0 - 2.0).abs() < 0.6 && (b.y1 - 18.0).abs() < 0.6, "{b:?}");
    assert_sane(&res, 1025, 20);
}

#[test]
fn a_hostile_palette_is_bounded_and_near_duplicates_merge() {
    let img = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(RED));
    let many: Vec<String> = std::iter::once("#d22832".to_string()).chain((1..200_000u32).map(|i| format!("#{:06x}", i * 37 % 0xfffff0))).collect();
    let started = std::time::Instant::now();
    let res = trace_within(&img, &TraceParams { logo_colors: many, ..logo_params() }, 100_000);
    assert!(started.elapsed().as_secs() < 20, "palette handling took {:?}", started.elapsed());
    if let Ok(r) = res {
        assert!(r.palette.len() <= 11);
    }
    // Ten shades within a couple of units of one red are one colour, so one path, not ten layers.
    let shades: Vec<String> = (0..10).map(|i| format!("#{:02x}{:02x}{:02x}", 210 + i % 3, 40 + i % 2, 50 + i % 3)).collect();
    let res = trace(&img, &TraceParams { logo_colors: shades, ..logo_params() });
    assert_eq!(res.paths.len(), 1, "{} paths", res.paths.len());
    // White cannot be named: naming only white leaves nothing to trace.
    assert!(trace(&img, &TraceParams { logo_colors: vec!["#ffffff".into()], ..logo_params() }).paths.is_empty());
}

#[test]
fn a_small_accent_colour_is_not_lost() {
    let img = render(100, 100, 0.6, |x, y| {
        if (x - 50.0).hypot(y - 50.0) <= 40.0 {
            Some(BLUE)
        } else if (x - 90.0).hypot(y - 90.0) <= 3.5 {
            Some(RED)
        } else {
            None
        }
    });
    let res = trace(&img, &logo_params());
    assert!(res.paths.iter().any(|p| near(p.color, RED, 12)), "palette {:?}", res.palette);
    assert!(res.paths.iter().any(|p| near(p.color, BLUE, 12)));
}

#[test]
fn a_hard_edged_source_still_gets_smooth_circles() {
    // No anti-aliasing at all: every pixel is exactly one colour.
    let img = Raster::from_fn(80, 80, |x, y| {
        let d = (f64::from(x) + 0.5 - 40.0).hypot(f64::from(y) + 0.5 - 40.0);
        if (10.0..=28.0).contains(&d) { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] }
    });
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert!(res.anchor_count() <= 40, "{} anchors for two circles from a hard-edged source", res.anchor_count());
    let errs: Vec<f64> = samples(&res)
        .iter()
        .map(|p| {
            let d = p.distance(Point::new(40.0, 40.0));
            (d - 28.0).abs().min((d - 10.0).abs())
        })
        .collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    assert!(mean < 0.3 && errs.iter().copied().fold(0.0, f64::max) < 0.8, "mean {mean:.3}");
}

#[test]
fn random_small_images_and_settings_never_panic() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let weird = [f64::NAN, f64::INFINITY, -3.0, 0.0, 35.0, 60.0, 100.0, 1e12];
    let colours = [[210u8, 40, 50], [40, 80, 200], [255, 255, 255], [20, 160, 70], [0, 0, 0]];
    for _ in 0..250 {
        let (w, h) = ((next() % 24 + 1) as u32, (next() % 24 + 1) as u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let r = next();
                let c = colours[((u64::from(x) / 3 + u64::from(y) / 4 + r % 2) % 5) as usize];
                rgba.extend_from_slice(&match r % 11 {
                    0 => [0, 0, 0, 0],
                    1 => [(r >> 8) as u8, (r >> 16) as u8, (r >> 24) as u8, (r >> 32) as u8],
                    _ => [c[0], c[1], c[2], 255],
                });
            }
        }
        let img = Raster::new(w, h, rgba).unwrap();
        let p = TraceParams {
            paths: weird[(next() % 8) as usize],
            corners: weird[(next() % 8) as usize],
            noise: [0, 1, 6, 1000, u32::MAX][(next() % 5) as usize],
            colors: [0, 2, 6, 40][(next() % 4) as usize],
            ignore_white: next() % 2 == 0,
            snap_curves_to_lines: next() % 2 == 0,
            ..logo_params()
        };
        if let Ok(res) = trace_within(&img, &p, 20_000) {
            assert_sane(&res, w, h);
        }
    }
}

/// A scan's paper is darker in one corner than in another: it is still one colour, not a band of
/// paths for each tone of it.
#[test]
fn uneven_paper_is_one_colour() {
    let img = Raster::from_fn(160, 120, |x, y| {
        let shade = 1.0 - 0.1 * f64::from(x) / 160.0 - 0.04 * (f64::from(y) / 40.0).sin().abs();
        let ink = (40..120).contains(&x) && (30..90).contains(&y);
        let c = if ink { [40.0, 60.0, 150.0] } else { [236.0, 230.0, 214.0] };
        [(c[0] * shade) as u8, (c[1] * shade) as u8, (c[2] * shade) as u8, 255]
    });
    let res = trace(&img, &crate::preset("Flat Logo").unwrap());
    assert_eq!(res.paths.len(), 2, "{:?}", res.paths.iter().map(|p| (p.color, p.pixels)).collect::<Vec<_>>());
}

/// Line art: thin dark strokes and small red marks (2.4 px wide) on white paper, anti-aliased. No stroke fills a
/// uniform 3 × 3 neighbourhood, so the solid pixels are all paper; the strokes' colours must still
/// be found (from the strokes' cores) and traced, not cut as noise into a blank result.
#[test]
fn thin_line_art_on_paper_is_traced_not_blank() {
    const INK: [u8; 3] = [46, 36, 38];
    const MARK: [u8; 3] = [196, 38, 66];
    // Distance from the point to the segment a–b.
    let seg = |x: f64, y: f64, a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
    };
    let strokes = [((10.0, 12.0), (150.0, 30.0)), ((20.0, 110.0), (140.0, 60.0)), ((80.0, 8.0), (86.0, 112.0)), ((30.0, 40.0), (60.0, 100.0))];
    let marks = [((110.0, 95.0), (128.0, 101.0)), ((40.0, 20.0), (52.0, 18.0))];
    let img = render(160, 120, 0.0, |x, y| {
        Some(if marks.iter().any(|&(a, b)| seg(x, y, a, b) <= 1.2) {
            MARK
        } else if strokes.iter().any(|&(a, b)| seg(x, y, a, b) <= 1.2) {
            INK
        } else {
            [255, 255, 255]
        })
    });
    let res = trace_within(&img, &crate::preset("Flat Logo").unwrap(), 100_000).expect("line art is traced, not refused");
    assert!(res.paths.iter().any(|p| near(p.color, INK, 30)), "no ink path; palette {:?}", res.palette);
    assert!(res.paths.iter().any(|p| near(p.color, MARK, 30)), "no red mark; palette {:?}", res.palette);
    // The middle of each stroke and mark is painted with its colour (the topmost path there).
    let colour_at = |p: Point| res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color);
    for (&(a, b), colour) in strokes.iter().map(|s| (s, INK)).chain(marks.iter().map(|m| (m, MARK))) {
        for k in 1..8 {
            let t = f64::from(k) / 8.0;
            let p = Point::new(a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            let at = colour_at(p);
            assert!(at.is_some_and(|c| near(c, colour, 30)), "{p:?} is {at:?}, not {colour:?}");
        }
    }
}

/// Blank white paper with a light grain (a scan, a JPEG: each channel 243 to 255): the grain's
/// darkest specks are no ink, and the paper traces to nothing (a blank page stays blank).
#[test]
fn grainy_blank_paper_has_no_ink() {
    let grain = |x: u32, y: u32, k: u32| {
        let mut v = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ k.wrapping_mul(83_492_791)).wrapping_mul(2_654_435_761);
        v ^= v >> 15;
        (255 - (v % 13)) as u8
    };
    let img = Raster::from_fn(160, 120, |x, y| [grain(x, y, 1), grain(x, y, 2), grain(x, y, 3), 255]);
    let res = trace(&img, &crate::preset("Flat Logo").unwrap());
    assert!(res.palette.is_empty() && res.paths.is_empty(), "palette {:?}", res.palette);
}

/// White paper with 24 short grey marks (12 × 2 px) of one shade: light (239, 223) or mid grey
/// (191). They cover under 1% of the page, so it is paper with a few specks on it, not line art:
/// no colour is found and the page traces to nothing, as plain white paper does.
#[test]
fn sparse_marks_on_paper_are_not_promoted_to_colours() {
    let marks: Vec<(u32, u32)> = (0..24u32).map(|k| (10 + (k * 37) % 220, 10 + (k * 71) % 220)).collect();
    for shade in [239u8, 223, 191] {
        let img = Raster::from_fn(256, 256, |x, y| {
            let on = marks.iter().any(|&(mx, my)| (mx..mx + 12).contains(&x) && (my..my + 2).contains(&y));
            let v = if on { shade } else { 255 };
            [v, v, v, 255]
        });
        let res = trace(&img, &crate::preset("Flat Logo").unwrap());
        assert!(res.palette.is_empty(), "shade {shade}: palette {:?}", res.palette);
        assert!(res.paths.is_empty(), "shade {shade}: {} paths", res.paths.len());
    }
}

/// Bars two pixels wide (whole pixels of one color) on white beside a black disc, with the colors
/// found: no pixel of a bar has a 3 × 3 neighborhood of its own color, and a gray bar is a blend of
/// black and white. Light grays, a red and a pale yellow are found and traced in their own color. A
/// gray bar darker than about `#9B9B9B` stays black: a black stroke 1 to 1.6 px wide on the pixel
/// grid gives the same pixels.
#[test]
fn a_bar_two_pixels_wide_beside_black_art_is_found_in_its_color() {
    const INK: [u8; 3] = [4, 4, 3];
    for (bar, want) in [
        ([170u8, 170, 170], None),
        ([200, 200, 200], None),
        ([230, 230, 230], None),
        ([210, 40, 50], None),
        ([250, 230, 150], None),
        ([128, 128, 128], Some(INK)),
        ([51, 51, 51], Some(INK)),
    ] {
        let img = Raster::from_fn(120, 60, |x, y| {
            let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let c = if (fx - 25.0).hypot(fy - 28.0) <= 18.0 {
                INK
            } else if ((50..52).contains(&x) && (8..50).contains(&y)) || ((60..110).contains(&x) && (30..32).contains(&y)) {
                bar
            } else {
                [255, 255, 255]
            };
            [c[0], c[1], c[2], 255]
        });
        let res = trace(&img, &crate::preset("Flat Logo").unwrap());
        let colour_at = |p: Point| res.paths.iter().rev().find(|t| t.path.to_bezpath().winding(p) != 0).map(|t| t.color);
        let paths: Vec<_> = res.paths.iter().map(|t| (t.color, t.pixels)).collect();
        let want = want.unwrap_or(bar);
        for at in [Point::new(51.0, 20.0), Point::new(51.0, 40.0), Point::new(70.0, 31.0), Point::new(100.0, 31.0)] {
            assert_eq!(colour_at(at), Some(want), "bar {bar:?}: {at:?}; {paths:?}");
        }
        assert_eq!(colour_at(Point::new(25.0, 28.0)), Some(INK), "bar {bar:?}: the disc; {paths:?}");
    }
}

/// Three rows of thin anti-aliased "lettering" (rings, slants and bars 1.6 px wide) in red, blue
/// and green on white. The lighter shades along the strokes' anti-aliased edges are blends of an
/// ink with the paper, not colours of their own: the palette is the three inks and white.
#[test]
fn anti_aliased_coloured_lettering_has_only_its_inks() {
    const INKS: [[u8; 3]; 3] = [[210, 40, 50], [30, 90, 200], [20, 130, 70]];
    let seg = |x: f64, y: f64, a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
    };
    // One "letter" in the cell at (cx, cy): a ring, a slanted stroke or a bar with a foot.
    let glyph = |x: f64, y: f64, k: usize, cx: f64, cy: f64| match k % 3 {
        0 => ((x - cx).hypot(y - cy) - 6.0).abs(),
        1 => seg(x, y, (cx - 5.0, cy + 8.0), (cx + 4.0, cy - 8.0)).min(seg(x, y, (cx - 1.0, cy), (cx + 6.0, cy + 7.5))),
        _ => seg(x, y, (cx - 0.3, cy - 8.0), (cx + 0.4, cy + 8.0)).min(seg(x, y, (cx - 5.0, cy + 8.2), (cx + 5.0, cy + 7.9))),
    };
    let img = render(256, 256, 0.0, |x, y| {
        for (row, ink) in INKS.iter().enumerate() {
            let cy = 40.0 + 80.0 * row as f64 + 0.37 * row as f64;
            for k in 0..14 {
                let cx = 14.0 + 17.3 * k as f64;
                if glyph(x, y, k + row, cx, cy) <= 0.8 {
                    return Some(*ink);
                }
            }
        }
        Some([255, 255, 255])
    });
    let res = trace(&img, &crate::preset("Flat Logo").unwrap());
    let colours: Vec<[u8; 3]> = res.palette.iter().copied().filter(|c| !c.iter().all(|&v| v >= 245)).collect();
    assert_eq!(colours.len(), 3, "palette {:?}", res.palette);
    for ink in INKS {
        assert!(colours.iter().any(|&c| near(c, ink, 12)), "{ink:?} missing from {:?}", res.palette);
    }
}

/// An outline whose fit has no area is left out. A pixel-sized black and cream noise, with both
/// colors named exactly, fits some specks to two anchors whose segments run along one line and
/// others to one anchor, which has no segment.
#[test]
fn an_outline_fitted_without_area_is_left_out() {
    use kurbo::ParamCurveArea;
    let noise = render(96, 96, 0.0, |x, y| {
        let n = ((x.floor() * 12.9898 + y.floor() * 78.233).sin() * 43758.5453).fract();
        Some(if n > 0.5 { [4, 4, 3] } else { [252, 248, 227] })
    });
    let res = trace(&noise, &TraceParams { logo_colors: vec!["#040403".into(), "#FCF8E3".into()], ..logo_params() });
    assert!(res.paths.len() > 1, "{} paths", res.paths.len());
    for t in &res.paths {
        for sp in &t.path.subpaths {
            let origin = sp.anchors[0].p.to_vec2();
            let area: f64 = (0..sp.segment_count()).map(|i| (kurbo::Affine::translate(-origin) * sp.segment(i)).signed_area()).sum();
            assert!(area.abs() >= 1e-6, "a {:?} subpath of {} anchors with area {area:e}", t.color, sp.anchors.len());
        }
    }
}
