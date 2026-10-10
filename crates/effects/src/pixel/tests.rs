use serde_json::json;
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, Point, Rect};

use super::*;
use crate::{RasterFx, raster_effects};

/// A `w` × `h` raster whose pixels are document points (origin top-left).
fn space(w: usize, h: usize) -> PixelSpace {
    PixelSpace { to_doc: Affine::IDENTITY, px: 1.0, center: Point::new(w as f64 / 2.0, h as f64 / 2.0), channels: Channels::Rgb }
}

fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            out.extend(f(x, y));
        }
    }
    out
}

fn at(d: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

fn fx(id: &str, p: serde_json::Value) -> PixelFx {
    let e = Effect { id: id.into(), params: p, visible: true };
    match raster_effects(&[e]).first() {
        Some(RasterFx::Pixel(p)) => *p,
        other => panic!("{id}: {other:?}"),
    }
}

fn grey(v: u8) -> [u8; 4] {
    [v, v, v, 255]
}

#[test]
fn params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("blur.radial", json!({})), PixelFx::RadialBlur { amount: 10.0, zoom: false, passes: 6 });
    assert_eq!(
        fx("blur.radial", json!({"amount": 1e308, "method": "ZOOM", "quality": "best"})),
        PixelFx::RadialBlur { amount: 100.0, zoom: true, passes: 8 }
    );
    assert_eq!(fx("blur.radial", json!({"amount": -5, "quality": "nonsense"})), PixelFx::RadialBlur { amount: 1.0, zoom: false, passes: 6 });
    assert_eq!(fx("blur.smart", json!({})), PixelFx::SmartBlur { radius: 3.0, threshold: 25.0, samples: 7 });
    assert_eq!(
        fx("blur.smart", json!({"radius": "1e999", "threshold": -1, "quality": "low"})),
        PixelFx::SmartBlur { radius: 3.0, threshold: 0.1, samples: 5 }
    );
    assert_eq!(fx("sharpen.unsharpMask", json!({})), PixelFx::UnsharpMask { amount: 0.5, radius: 1.0, threshold: 0.0 });
    assert_eq!(
        fx("sharpen.unsharpMask", json!({"amount": 1e9, "radius": 0, "threshold": 1e9})),
        PixelFx::UnsharpMask { amount: 5.0, radius: 0.1, threshold: 255.0 }
    );
    assert_eq!(fx("stylize.glowingEdges", json!({})), PixelFx::GlowingEdges { width: 2.0, brightness: 6.0, smoothness: 5.0 });
    for id in PIXEL_EFFECTS {
        assert!(crate::is_raster(id) && crate::effect_info(id).is_some_and(|e| e.raster), "{id}");
    }
}

#[test]
fn outsets_follow_the_object() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let spin = fx("blur.radial", json!({}));
    let far = 0.5 * 100f64.hypot(50.0);
    assert!((spin.outset(b) - (far - 25.0)).abs() < 1e-9);
    assert!(fx("blur.radial", json!({"method": "zoom", "amount": 100})).outset(b) > 0.4 * far);
    assert_eq!(fx("blur.smart", json!({"radius": 4})).outset(b), 4.0);
    assert_eq!(fx("sharpen.unsharpMask", json!({})).outset(b), 0.0);
    assert_eq!(spin.reach(), None);
    assert_eq!(fx("sharpen.unsharpMask", json!({"radius": 2})).reach(), Some(6.0));
}

#[test]
fn wrong_sizes_and_empty_rasters_are_left_alone() {
    for id in PIXEL_EFFECTS {
        let f = fx(id, json!({}));
        let mut d = vec![200u8; 4 * 10];
        f.apply(&mut d, 3, 3, &space(3, 3));
        f.apply(&mut d, 0, 10, &space(0, 10));
        f.apply(&mut [], 0, 0, &space(0, 0));
        assert!(d.iter().all(|v| *v == 200), "{id}");
    }
}

#[test]
fn extreme_parameters_finish() {
    let (w, h) = (40, 30);
    let src = image(w, h, |x, y| if (x / 5 + y / 5) % 2 == 0 { grey(250) } else { [0, 0, 0, 0] });
    let tiny = PixelSpace { px: 1e-9, ..space(w, h) };
    let huge = PixelSpace { px: 1e9, center: Point::new(f64::MAX, -1e300), ..space(w, h) };
    for id in PIXEL_EFFECTS {
        for p in [json!({"amount": 1e308, "radius": 1e308, "threshold": 1e308, "quality": "best"}), json!({"amount": 0, "radius": 0})] {
            for s in [tiny, huge, space(w, h)] {
                let mut d = src.clone();
                fx(id, p.clone()).apply(&mut d, w, h, &s);
            }
        }
    }
}

#[test]
fn every_effect_is_deterministic() {
    let (w, h) = (32, 24);
    let src = image(w, h, |x, y| {
        let a = if x > 4 { 255 } else { 128 };
        [((x * 8) as u8).min(a), ((y * 10) as u8).min(a), ((x * y % 256) as u8).min(a), a]
    });
    for id in PIXEL_EFFECTS {
        let f = fx(id, json!({"amount": 40, "radius": 3, "threshold": 40}));
        let (mut a, mut b) = (src.clone(), src.clone());
        f.apply(&mut a, w, h, &space(w, h));
        f.apply(&mut b, w, h, &space(w, h));
        assert_eq!(a, b, "{id}");
        assert_ne!(a, src, "{id} changes the image");
        // Premultiplied stays premultiplied.
        assert!(a.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]), "{id}");
    }
}

#[test]
fn glowing_edges_draw_a_bright_outline_and_preserve_premultiplication() {
    let (w, h) = (9, 9);
    let src = image(w, h, |x, _| if x < 4 { [0, 0, 0, 200] } else { [200, 200, 200, 200] });
    let mut out = src.clone();
    fx("stylize.glowingEdges", json!({"edgeWidth": 1, "smoothness": 1, "edgeBrightness": 20})).apply(&mut out, w, h, &space(w, h));
    assert!(at(&out, w, 3, 4)[0] > 0 || at(&out, w, 4, 4)[0] > 0);
    assert!(out.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
}

/// Glowing Edges sees transparency beyond every side of the raster: a solid square glows alike at
/// its left and right edges (the right one doesn't read the next row's first pixel).
#[test]
fn glowing_edges_glow_alike_on_every_side() {
    let (w, h) = (9, 9);
    let mut out = image(w, h, |_, _| [180, 180, 180, 255]);
    fx("stylize.glowingEdges", json!({"edgeWidth": 1, "smoothness": 1, "edgeBrightness": 20})).apply(&mut out, w, h, &space(w, h));
    let (left, right, top, bottom) = (at(&out, w, 0, 4), at(&out, w, 8, 4), at(&out, w, 4, 0), at(&out, w, 4, 8));
    assert!(left[0] > 0 && left == right && top == bottom, "{left:?} {right:?} {top:?} {bottom:?}");
    assert!(at(&out, w, 4, 4)[0] < left[0], "the inside stays darker than the edges");
}

#[test]
fn glowing_edges_defaults_keep_the_outline_bright_and_use_source_colour() {
    let (w, h) = (17, 17);
    let src = image(w, h, |x, y| if (4..13).contains(&x) && (4..13).contains(&y) { [180, 0, 0, 255] } else { [0; 4] });
    let mut out = src.clone();
    fx("stylize.glowingEdges", json!({})).apply(&mut out, w, h, &space(w, h));

    let outline = at(&out, w, 4, 8);
    assert!(outline[0] >= 170, "default outline should retain a near-full-color peak: {outline:?}");
    let outside = at(&out, w, 3, 8);
    assert_eq!(outside[1], 0, "transparent halo must not be grey: {outside:?}");
    assert_eq!(outside[2], 0, "transparent halo must retain source hue: {outside:?}");
    // The glow outside keeps the edge's colour as it fades, rather than darkening towards grey.
    assert!(outside[3] > 0 && u32::from(outside[0]) * 255 >= 240 * u32::from(outside[3]), "{outside:?}");
    assert!(out.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
}

#[test]
fn spin_blur_smears_along_arcs_and_keeps_the_centre() {
    let (w, h) = (41, 41);
    // A white dot 12 px right of the centre (20.5, 20.5), and one on the centre.
    let src = image(w, h, |x, y| if (x == 32 || x == 20) && y == 20 { grey(255) } else { [0, 0, 0, 0] });
    let mut d = src.clone();
    fx("blur.radial", json!({"amount": 60, "quality": "best"})).apply(&mut d, w, h, &space(w, h));
    // 60° of arc around the centre: up to 30° either way, 12 px out: (20.5 + 12 cos 20°, 20.5 ± 12 sin 20°).
    assert!(at(&d, w, 31, 16)[3] > 0 && at(&d, w, 31, 24)[3] > 0, "the dot spreads along its circle");
    assert_eq!(at(&d, w, 32, 10)[3], 0, "not beyond the arc");
    assert_eq!(at(&d, w, 26, 20)[3], 0, "not towards the centre");
    assert!(at(&d, w, 20, 20)[3] > 100, "the centre barely moves: {:?}", at(&d, w, 20, 20));
    // Zoom smears along the ray instead.
    let mut z = src.clone();
    fx("blur.radial", json!({"amount": 60, "method": "zoom", "quality": "best"})).apply(&mut z, w, h, &space(w, h));
    assert!(at(&z, w, 35, 20)[3] > 0 && at(&z, w, 29, 20)[3] > 0, "the dot streaks along the ray");
    assert_eq!(at(&z, w, 32, 16)[3], 0, "not across it");
}

#[test]
fn smart_blur_smooths_noise_and_keeps_edges() {
    let (w, h) = (40, 20);
    // Left: grey 100 with ±6 noise; right: white.
    let src = image(w, h, |x, y| if x < 20 { grey(if (x + y) % 2 == 0 { 94 } else { 106 }) } else { grey(255) });
    let mut d = src.clone();
    fx("blur.smart", json!({"radius": 3, "threshold": 25})).apply(&mut d, w, h, &space(w, h));
    let v = at(&d, w, 8, 10)[0];
    assert!((97..=103).contains(&v), "noise smoothed: {v}");
    assert!(at(&d, w, 19, 10)[0] < 120, "the edge's dark side doesn't take the white");
    assert_eq!(at(&d, w, 20, 10)[0], 255, "the edge's white side stays white");
    // A threshold above the step blurs across it.
    let mut all = src.clone();
    fx("blur.smart", json!({"radius": 3, "threshold": 100})).apply(&mut all, w, h, &space(w, h));
    assert_eq!(at(&all, w, 20, 10)[0], 255, "a 155-level step is still past 100");
}

#[test]
fn unsharp_mask_overshoots_at_edges() {
    let (w, h) = (40, 10);
    let src = image(w, h, |x, _| grey(if x < 20 { 100 } else { 150 }));
    let mut d = src.clone();
    fx("sharpen.unsharpMask", json!({"amount": 100, "radius": 2})).apply(&mut d, w, h, &space(w, h));
    assert!(at(&d, w, 19, 5)[0] < 95, "dark side darker: {:?}", at(&d, w, 19, 5));
    assert!(at(&d, w, 20, 5)[0] > 155, "light side lighter: {:?}", at(&d, w, 20, 5));
    assert_eq!(at(&d, w, 5, 5), grey(100), "flat areas stay");
    assert_eq!(at(&d, w, 35, 5), grey(150));
    // Differences under the threshold are left alone.
    let mut t = src.clone();
    fx("sharpen.unsharpMask", json!({"amount": 100, "radius": 2, "threshold": 60})).apply(&mut t, w, h, &space(w, h));
    assert_eq!(t, src);
    // Coverage stays, and a shape's edge against transparency doesn't darken.
    let shape = image(w, h, |x, _| if x < 20 { [200, 0, 0, 255] } else { [0, 0, 0, 0] });
    let mut s = shape.clone();
    fx("sharpen.unsharpMask", json!({"amount": 300, "radius": 3})).apply(&mut s, w, h, &space(w, h));
    assert_eq!(s, shape);
}

#[test]
fn lengths_are_document_units() {
    // The same art at twice the resolution, sharpened with the same radius in points: the
    // overshoot spans twice the pixels, the same distance.
    let edge = |scale: usize| image(40 * scale, 4, move |x, _| grey(if x < 20 * scale { 100 } else { 150 }));
    let run = |scale: usize| {
        let mut d = edge(scale);
        let s = PixelSpace { to_doc: Affine::scale(1.0 / scale as f64), px: 1.0 / scale as f64, center: Point::ZERO, channels: Channels::Rgb };
        fx("sharpen.unsharpMask", json!({"amount": 100, "radius": 2})).apply(&mut d, 40 * scale, 4, &s);
        (0..40 * scale).filter(|x| at(&d, 40 * scale, *x, 2)[0] < 99).count()
    };
    let (one, two) = (run(1), run(2));
    assert!(one > 1 && (two as f64 / one as f64 - 2.0).abs() < 0.5, "{one} px at 1×, {two} px at 2×");
}

#[test]
fn gaussian_and_plane_blurs_keep_their_mass() {
    let (w, h) = (30, 30);
    let mut d = image(w, h, |x, y| if (10..20).contains(&x) && (10..20).contains(&y) { grey(255) } else { [0, 0, 0, 0] });
    let before: u32 = d.as_chunks::<4>().0.iter().map(|p| p[3] as u32).sum();
    gaussian_rgba(&mut d, w, h, 2.0);
    let after: u32 = d.as_chunks::<4>().0.iter().map(|p| p[3] as u32).sum();
    assert!((before as f64 - after as f64).abs() / (before as f64) < 0.02, "{before} → {after}");
    assert!(at(&d, w, 9, 15)[3] > 0 && at(&d, w, 10, 15)[3] < 255);
    let mut plane: Vec<f32> = (0..w * h).map(|i| if i == 15 * w + 15 { 100.0 } else { 0.0 }).collect();
    blur_plane(&mut plane, w, h, 2.0);
    assert!((plane.iter().sum::<f32>() - 100.0).abs() < 0.5 && plane[15 * w + 13] > 0.0 && plane[13 * w + 15] > 0.0);
}

/// Timings on a 2000 × 2000 raster (`cargo test -p vectorcraft-effects --release -- --ignored
/// --nocapture pixel_timings`).
#[test]
#[ignore]
fn pixel_timings() {
    let (w, h) = (2000, 2000);
    let src = image(w, h, |x, y| [(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8, 255]);
    let s = PixelSpace { to_doc: Affine::IDENTITY, px: 1.0, center: Point::new(1000.0, 1000.0), channels: Channels::Rgb };
    let cases = [
        ("blur.radial", json!({"quality": "draft"})),
        ("blur.radial", json!({"quality": "good"})),
        ("blur.radial", json!({"quality": "best", "method": "zoom"})),
        ("blur.smart", json!({"quality": "low"})),
        ("blur.smart", json!({"quality": "high", "radius": 30})),
        ("sharpen.unsharpMask", json!({"radius": 20})),
        ("pixelate.colorHalftone", json!({"maxRadius": 4})),
        ("pixelate.crystallize", json!({"cellSize": 3})),
        ("pixelate.crystallize", json!({"cellSize": 300})),
        ("pixelate.mezzotint", json!({"type": "grainyDots"})),
        ("pixelate.pointillize", json!({"cellSize": 3})),
        ("pixelate.pointillize", json!({"cellSize": 300})),
    ];
    for (id, p) in cases {
        let mut d = src.clone();
        let t = std::time::Instant::now();
        fx(id, p.clone()).apply(&mut d, w, h, &s);
        println!("{id} {p}: {:?}", t.elapsed());
    }
}

#[test]
fn pixelate_params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("pixelate.colorHalftone", json!({})), PixelFx::ColorHalftone { max_radius: 8.0, angles: [108.0, 162.0, 90.0, 45.0] });
    assert_eq!(
        fx("pixelate.colorHalftone", json!({"maxRadius": 1e9, "channel1": -1e9, "channel2": 30, "channel4": "x"})),
        PixelFx::ColorHalftone { max_radius: 127.0, angles: [-360.0, 30.0, 90.0, 45.0] }
    );
    assert_eq!(fx("pixelate.colorHalftone", json!({"maxRadius": 0})), PixelFx::ColorHalftone { max_radius: 4.0, angles: [108.0, 162.0, 90.0, 45.0] });
    assert_eq!(fx("pixelate.crystallize", json!({})), PixelFx::Crystallize { cell: 10.0 });
    assert_eq!(fx("pixelate.crystallize", json!({"cellSize": -4})), PixelFx::Crystallize { cell: 3.0 });
    assert_eq!(fx("pixelate.pointillize", json!({})), PixelFx::Pointillize { cell: 5.0 });
    assert_eq!(fx("pixelate.pointillize", json!({"cellSize": 1e300})), PixelFx::Pointillize { cell: 300.0 });
    assert_eq!(fx("pixelate.mezzotint", json!({})), PixelFx::Mezzotint { kind: Mezzotint::FINE_DOTS });
    assert_eq!(fx("pixelate.mezzotint", json!({"type": "LONGSTROKES"})), PixelFx::Mezzotint { kind: Mezzotint::Lines { length: 24, width: 2 } });
    for junk in [json!({"type": 7}), json!({"type": "nonsense"})] {
        assert_eq!(fx("pixelate.mezzotint", junk), PixelFx::Mezzotint { kind: Mezzotint::FINE_DOTS });
    }
    // Every Type is a pattern of its own, documented in the catalogue.
    let doc = crate::effect_info("pixelate.mezzotint").unwrap().params;
    let kinds: Vec<Mezzotint> = MEZZOTINT_TYPES.iter().map(|(_, v)| Mezzotint::parse(v)).collect();
    for (i, (_, v)) in MEZZOTINT_TYPES.iter().enumerate() {
        assert!(doc.contains(&format!("\"{v}\"")), "{v}");
        assert!(kinds.iter().skip(i + 1).all(|k| *k != kinds[i]), "{v}");
    }
    // Dots and crystals reach beyond the object; screens and grain stay on it.
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    assert_eq!(fx("pixelate.crystallize", json!({"cellSize": 20})).outset(b), 30.0);
    assert_eq!(fx("pixelate.pointillize", json!({"cellSize": 20})).outset(b), 30.0);
    assert_eq!(fx("pixelate.colorHalftone", json!({})).outset(b), 0.0);
    assert_eq!(fx("pixelate.mezzotint", json!({})).reach(), Some(0.0));
}

/// Fraction of `d`'s pixels for which `f` holds.
fn share(d: &[u8], f: impl Fn(&[u8; 4]) -> bool) -> f64 {
    let px = d.as_chunks::<4>().0;
    px.iter().filter(|p| f(p)).count() as f64 / px.len() as f64
}

#[test]
fn color_halftone_screens_midtones_into_dots_and_keeps_white_and_black() {
    let (w, h) = (64, 64);
    let halftone = fx("pixelate.colorHalftone", json!({}));
    for v in [0, 255] {
        let mut d = image(w, h, |_, _| grey(v));
        halftone.apply(&mut d, w, h, &space(w, h));
        assert_eq!(d, image(w, h, |_, _| grey(v)), "{v} stays");
    }
    // Mid grey: each channel's dots cover about half its screen, so the tone stays.
    let mut d = image(w, h, |_, _| grey(128));
    halftone.apply(&mut d, w, h, &space(w, h));
    let px = d.as_chunks::<4>().0;
    for c in 0..3 {
        let mean = px.iter().map(|p| f64::from(p[c])).sum::<f64>() / px.len() as f64;
        assert!((mean - 128.0).abs() < 16.0, "channel {c}: {mean}");
    }
    assert!(share(&d, |p| p[0] == 255) > 0.25 && share(&d, |p| p[0] == 0) > 0.25, "dots, not grey");
    assert!(share(&d, |p| p[0] != p[1]) > 0.1, "the channels' screens lie at different angles");
    assert!(px.iter().all(|p| p[3] == 255), "coverage stays");
    // Transparency stays transparent.
    let mut half = image(w, h, |x, _| if x < 32 { grey(160) } else { [0; 4] });
    halftone.apply(&mut half, w, h, &space(w, h));
    assert!((0..h).all(|y| (32..w).all(|x| at(&half, w, x, y) == [0; 4])));
    // Dots are bigger where the image is brighter.
    let mut ramp = image(w, h, |x, _| grey((x * 4) as u8));
    halftone.apply(&mut ramp, w, h, &space(w, h));
    let lit = |x0: usize| (0..h).flat_map(|y| (x0..x0 + 16).map(move |x| (x, y))).filter(|(x, y)| at(&ramp, w, *x, *y)[0] > 127).count();
    assert!(lit(8) < lit(40), "{} < {}", lit(8), lit(40));
}

#[test]
fn color_halftone_screens_inks_in_cmyk_documents() {
    let (w, h) = (64, 64);
    let halftone = fx("pixelate.colorHalftone", json!({}));
    let cmyk = PixelSpace { channels: Channels::Cmyk, ..space(w, h) };
    // A neutral grey prints with all four inks: dots of each on white paper, black where they
    // all overlap.
    let mut d = image(w, h, |_, _| grey(128));
    halftone.apply(&mut d, w, h, &cmyk);
    assert!(share(&d, |p| *p == grey(255)) > 0.02 && share(&d, |p| *p == grey(0)) > 0.02);
    for (ink, colour) in [("cyan", [0, 255, 255, 255]), ("magenta", [255, 0, 255, 255]), ("yellow", [255, 255, 0, 255])] {
        assert!(share(&d, |p| *p == colour) > 0.0, "{ink} dots alone");
    }
    // White paper stays bare; pure cyan ink fills its cells.
    let mut white = image(w, h, |_, _| grey(255));
    halftone.apply(&mut white, w, h, &cmyk);
    assert_eq!(white, image(w, h, |_, _| grey(255)));
    let mut cyan = image(w, h, |_, _| [0, 255, 255, 255]);
    halftone.apply(&mut cyan, w, h, &cmyk);
    assert_eq!(cyan, image(w, h, |_, _| [0, 255, 255, 255]));
    // Ink planes: the complemented K plane screens one grey channel at the black angle (45°,
    // the same in every quarter turn).
    let mut k = image(w, h, |_, _| grey(128));
    halftone.apply(&mut k, w, h, &PixelSpace { channels: Channels::KPlane, ..space(w, h) });
    assert!(k.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]), "one grey channel");
    assert!(share(&k, |p| p[0] == 255) > 0.25 && share(&k, |p| p[0] == 0) > 0.25);
    let mut turned = image(w, h, |_, _| grey(128));
    fx("pixelate.colorHalftone", json!({"channel4": 135})).apply(&mut turned, w, h, &PixelSpace { channels: Channels::KPlane, ..space(w, h) });
    let mut other = image(w, h, |_, _| grey(128));
    fx("pixelate.colorHalftone", json!({"channel4": 30})).apply(&mut other, w, h, &PixelSpace { channels: Channels::KPlane, ..space(w, h) });
    assert_eq!(turned, k, "channel 4 sets the K plane's angle");
    assert_ne!(other, k);
}

#[test]
fn crystallize_redraws_in_solid_polygons() {
    let (w, h) = (48, 48);
    let crystals = fx("pixelate.crystallize", json!({"cellSize": 6}));
    // One colour stays that colour (away from the raster's edges, where crystals around points
    // beyond it take the transparency there).
    let mut d = image(w, h, |_, _| [200, 100, 50, 255]);
    crystals.apply(&mut d, w, h, &space(w, h));
    assert!((9..39).all(|y| (9..39).all(|x| at(&d, w, x, y) == [200, 100, 50, 255])));
    // Two halves: every pixel takes one of the colours, and the border between them zigzags.
    let (red, blue) = ([200, 0, 0, 255], [0, 0, 200, 255]);
    let src = image(w, h, |x, _| if x < 24 { red } else { blue });
    let mut d = src.clone();
    crystals.apply(&mut d, w, h, &space(w, h));
    assert!((9..39).all(|y| (9..39).all(|x| [red, blue].contains(&at(&d, w, x, y)))));
    let crossed = (9..39).flat_map(|y| (9..39).map(move |x| (x, y))).filter(|(x, y)| at(&d, w, *x, *y) != at(&src, w, *x, *y)).count();
    assert!(crossed > 10, "{crossed}");
    let border: std::collections::BTreeSet<usize> = (9..39).map(|y| (9..39).find(|x| at(&d, w, *x, y) == blue).unwrap()).collect();
    assert!(border.len() > 2, "{border:?}");
}

#[test]
fn mezzotint_turns_channels_fully_on_or_off() {
    let (w, h) = (64, 64);
    let grey_square = image(w, h, |x, y| if (8..56).contains(&x) && (8..56).contains(&y) { grey(128) } else { [0; 4] });
    let mut seen = Vec::new();
    for (_, kind) in MEZZOTINT_TYPES {
        let f = fx("pixelate.mezzotint", json!({ "type": kind }));
        for v in [0, 255] {
            let mut d = image(w, h, |_, _| grey(v));
            f.apply(&mut d, w, h, &space(w, h));
            assert_eq!(d, image(w, h, |_, _| grey(v)), "{kind}: {v} stays");
        }
        let mut d = grey_square.clone();
        f.apply(&mut d, w, h, &space(w, h));
        let px = d.as_chunks::<4>().0;
        assert!(px.iter().all(|p| p[..3].iter().all(|c| *c == 0 || *c == p[3])), "{kind}: pure colours");
        let on = px.iter().filter(|p| p[3] > 0 && p[0] > 0).count() as f64 / (48.0 * 48.0);
        assert!((0.25..0.75).contains(&on), "{kind}: {on}");
        assert!(px.iter().any(|p| p[3] > 0 && p[0] != p[1]), "{kind}: each channel its own pattern");
        assert!(d.iter().zip(&grey_square).skip(3).step_by(4).all(|(a, b)| a == b), "{kind}: coverage stays");
        assert!(!seen.contains(&d), "{kind}: a pattern of its own");
        seen.push(d);
    }
}

#[test]
fn pointillize_paints_dots_on_a_white_canvas() {
    let (w, h) = (80, 80);
    let red = [200, 0, 0, 255];
    let src = image(w, h, |x, y| if (20..60).contains(&x) && (20..60).contains(&y) { red } else { [0; 4] });
    let mut d = src.clone();
    fx("pixelate.pointillize", json!({})).apply(&mut d, w, h, &space(w, h));
    let inside: Vec<[u8; 4]> = (26..54).flat_map(|y| (26..54).map(move |x| (x, y))).map(|(x, y)| at(&d, w, x, y)).collect();
    let canvas = inside.iter().filter(|p| **p == grey(255)).count();
    let dots = inside.iter().filter(|p| **p == red).count();
    assert!(canvas > inside.len() / 20 && dots > inside.len() / 3, "{canvas} canvas, {dots} dots of {}", inside.len());
    assert!(inside.iter().all(|p| p[3] == 255 && p[1] == p[2] && p[0] >= p[1]), "red over white only");
    // Dots at the edges reach a little past the object; far from it stays transparent.
    assert!((0..h).any(|y| (14..20).any(|x| at(&d, w, x, y)[3] > 0)));
    assert!((0..h).all(|y| (0..10).all(|x| at(&d, w, x, y) == [0; 4])));
    assert!(d.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
}

/// The patterns lie in document space around the object's centre: the same at twice the
/// resolution, and moving with the object.
#[test]
fn pixelate_patterns_follow_the_object_at_any_resolution() {
    let (w, h) = (40, 40);
    let art = |x: f64, y: f64| -> [u8; 4] {
        if (8.0..32.0).contains(&x) && (8.0..32.0).contains(&y) { [(x * 6.0) as u8, (y * 6.0) as u8, 120, 255] } else { [0; 4] }
    };
    for (id, p) in [
        ("pixelate.colorHalftone", json!({"maxRadius": 4})),
        ("pixelate.crystallize", json!({"cellSize": 4})),
        ("pixelate.mezzotint", json!({"type": "coarseDots"})),
        ("pixelate.pointillize", json!({"cellSize": 4})),
    ] {
        let f = fx(id, p);
        let mut one = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut one, w, h, &space(w, h));
        // Twice the pixels for the same art, averaged back down.
        let mut two = image(2 * w, 2 * h, |x, y| art((x as f64 + 0.5) / 2.0, (y as f64 + 0.5) / 2.0));
        let fine = PixelSpace { to_doc: Affine::scale(0.5), px: 0.5, ..space(w, h) };
        f.apply(&mut two, 2 * w, 2 * h, &fine);
        let down = image(w, h, |x, y| {
            let q = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| at(&two, 2 * w, 2 * x + dx, 2 * y + dy));
            std::array::from_fn(|c| (q.iter().map(|p| u32::from(p[c])).sum::<u32>() / 4) as u8)
        });
        let diff = one.iter().zip(&down).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / one.len() as f64;
        assert!(diff < 24.0, "{id}: {diff}");
        // The object and its raster 7 × 3 points further: the same pixels.
        let moved = PixelSpace { to_doc: Affine::translate((7.0, 3.0)), center: Point::new(27.0, 23.0), ..space(w, h) };
        let mut shifted = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut shifted, w, h, &moved);
        assert_eq!(shifted, one, "{id}");
    }
}
