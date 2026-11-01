//! Sketch › Bas Relief, Charcoal, Conté Crayon, Photocopy and Stamp.

use serde_json::json;
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, Point, Rect};

use super::*;
use crate::{RasterFx, raster_effects};

/// A `w` × `h` raster whose pixels are document points (origin top-left), of an object that fills it.
fn space(w: usize, h: usize) -> PixelSpace {
    let (w, h) = (w as f64, h as f64);
    PixelSpace {
        to_doc: Affine::IDENTITY,
        px: 1.0,
        center: Point::new(w / 2.0, h / 2.0),
        channels: Channels::Rgb,
        line: 1.0,
        extent: 0.5 * w.hypot(h),
    }
}

fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| f(x, y)).collect()
}

fn fx(id: &str, p: serde_json::Value) -> PixelFx {
    let e = Effect { id: id.into(), params: p, visible: true };
    match raster_effects(&[e]).first() {
        Some(RasterFx::Pixel(p)) => p.clone(),
        other => panic!("{id}: {other:?}"),
    }
}

fn run(id: &str, p: serde_json::Value, src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = src.to_vec();
    fx(id, p).apply(&mut out, w, h, &space(w, h));
    out
}

fn grey(v: u8) -> [u8; 4] {
    [v, v, v, 255]
}

/// Channel 0 of pixel (`x`, `y`).
fn at(d: &[u8], w: usize, x: usize, y: usize) -> u8 {
    d[(y * w + x) * 4]
}

/// How many pixels of channel 0 are darker than 128.
fn black(d: &[u8]) -> usize {
    d.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count()
}

/// No panic and the coverage kept on tiny, huge and non-finite spaces and parameters.
fn survives(id: &str, params: &[serde_json::Value]) {
    let (w, h) = (9, 7);
    let src = image(w, h, |x, y| [(x * 25) as u8, (y * 30) as u8, 90, if x == 0 { 0 } else { 255 }]);
    let tiny = PixelSpace { px: 1e-9, ..space(w, h) };
    let huge = PixelSpace { px: 1e9, center: Point::new(f64::MAX, -1e300), ..space(w, h) };
    let nan = PixelSpace { px: f64::NAN, ..space(w, h) };
    for p in params {
        for s in [tiny, huge, nan, space(w, h)] {
            let mut out = src.clone();
            fx(id, p.clone()).apply(&mut out, w, h, &s);
            assert_eq!(out.len(), src.len());
            let alphas = |d: &[u8]| d.as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>();
            if s.px == 1.0 {
                assert_eq!(alphas(&out), alphas(&src), "{id}: coverage kept");
            }
        }
    }
    // A size that doesn't match, and an empty raster, do nothing.
    let mut odd = vec![1u8; 10];
    fx(id, json!({})).apply(&mut odd, 3, 3, &space(3, 3));
    assert_eq!(odd, vec![1u8; 10]);
    fx(id, json!({})).apply(&mut [], 0, 0, &space(1, 1));
}

#[test]
fn photocopy_copies_the_edges_of_dark_areas() {
    assert_eq!(fx("sketch.photocopy", json!({})), PixelFx::Photocopy { detail: 7.0, darkness: 25.0 });
    assert_eq!(fx("sketch.photocopy", json!({"detail": 1e308, "darkness": -5})), PixelFx::Photocopy { detail: 24.0, darkness: 1.0 });
    let (w, h) = (64, 16);
    let src = image(w, h, |x, _| grey(if x < 32 { 30 } else { 230 }));
    let out = run("sketch.photocopy", json!({"detail": 20}), &src, w, h);
    // Black just on the dark side of the edge; white in the dark area's middle and in the light.
    assert!(at(&out, w, 30, 8) < 64, "edge {}", at(&out, w, 30, 8));
    assert!(at(&out, w, 6, 8) > 192, "dark area {}", at(&out, w, 6, 8));
    assert!(at(&out, w, 50, 8) > 192, "light area {}", at(&out, w, 50, 8));
    assert_eq!(out, run("sketch.photocopy", json!({"detail": 20}), &src, w, h), "deterministic");
    // A higher Darkness copies more.
    let more = run("sketch.photocopy", json!({"detail": 20, "darkness": 50}), &src, w, h);
    let less = run("sketch.photocopy", json!({"detail": 20, "darkness": 1}), &src, w, h);
    assert!(black(&more) >= black(&less));
    survives("sketch.photocopy", &[json!({}), json!({"detail": 1e308, "darkness": "NaN"}), json!({"detail": -1e308, "darkness": [1]})]);
}

#[test]
fn stamp_prints_solid_black_and_white() {
    assert_eq!(fx("sketch.stamp", json!({})), PixelFx::Stamp { balance: 25.0, smoothness: 5.0 });
    assert_eq!(fx("sketch.stamp", json!({"lightDarkBalance": 1e308, "smoothness": -1})), PixelFx::Stamp { balance: 50.0, smoothness: 1.0 });
    let (w, h) = (64, 8);
    let src = image(w, h, |x, _| grey((x * 4) as u8));
    let few = run("sketch.stamp", json!({"lightDarkBalance": 0}), &src, w, h);
    let many = run("sketch.stamp", json!({"lightDarkBalance": 50}), &src, w, h);
    assert!(black(&many) > black(&few), "{} vs {}", black(&many), black(&few));
    // Pure black and white, but for the one pixel wide edges.
    let mixed = many.as_chunks::<4>().0.iter().filter(|p| p[0] != 0 && p[0] != 255).count();
    assert!(mixed * 10 < many.len() / 4, "{mixed} grey pixels");
    assert_eq!(at(&many, w, 0, 4), 0);
    assert_eq!(at(&many, w, 63, 4), 255);
    survives(
        "sketch.stamp",
        &[json!({}), json!({"lightDarkBalance": "NaN", "smoothness": 1e308}), json!({"lightDarkBalance": -1e308, "smoothness": null})],
    );
}

#[test]
fn charcoal_draws_bold_edges_and_sketches_the_midtones() {
    assert_eq!(fx("sketch.charcoal", json!({})), PixelFx::Charcoal { thickness: 1.0, detail: 1.0, balance: 50.0 });
    assert_eq!(
        fx("sketch.charcoal", json!({"thickness": 1e308, "detail": -5, "lightDarkBalance": "x"})),
        PixelFx::Charcoal { thickness: 7.0, detail: 0.0, balance: 50.0 }
    );
    let (w, h) = (64, 48);
    // Black on the left, white on the right: a bold line at the edge, nothing in the white.
    let step = image(w, h, |x, _| grey(if x < 32 { 0 } else { 255 }));
    let out = run("sketch.charcoal", json!({}), &step, w, h);
    assert!((28..36).any(|x| at(&out, w, x, 24) < 64), "edge");
    assert!((48..64).all(|x| at(&out, w, x, 24) == 255), "white stays white");
    // A midtone is sketched in strokes, more of it the more the balance reaches into the lights.
    let mid = image(w, h, |_, _| grey(128));
    let sketched = run("sketch.charcoal", json!({}), &mid, w, h);
    let (n, total) = (black(&sketched), w * h);
    assert!(n > total / 50 && n < total * 9 / 10, "{n} of {total}");
    let darker = run("sketch.charcoal", json!({"lightDarkBalance": 100}), &mid, w, h);
    let lighter = run("sketch.charcoal", json!({"lightDarkBalance": 0}), &mid, w, h);
    assert!(black(&darker) > black(&lighter));
    assert_eq!(sketched, run("sketch.charcoal", json!({}), &mid, w, h), "deterministic");
    let white = image(w, h, |_, _| grey(255));
    assert_eq!(black(&run("sketch.charcoal", json!({"lightDarkBalance": 0}), &white, w, h)), 0);
    survives(
        "sketch.charcoal",
        &[json!({}), json!({"thickness": 1e308, "detail": "NaN", "lightDarkBalance": [1]}), json!({"thickness": -1e308, "detail": 1e308})],
    );
}

#[test]
fn bas_relief_lights_the_surfaces_facing_the_light() {
    assert_eq!(fx("sketch.basRelief", json!({})), PixelFx::BasRelief { detail: 13.0, smoothness: 3.0, light: Light::Bottom });
    assert_eq!(
        fx("sketch.basRelief", json!({"detail": 0, "smoothness": 1e308, "lightDirection": "Top"})),
        PixelFx::BasRelief { detail: 1.0, smoothness: 15.0, light: Light::Top }
    );
    // Dark above, light below: the edge is lit from one side and shaded from the other.
    let (w, h) = (16, 48);
    let src = image(w, h, |_, y| grey(if y < 24 { 20 } else { 235 }));
    let below = run("sketch.basRelief", json!({"lightDirection": "bottom"}), &src, w, h);
    let above = run("sketch.basRelief", json!({"lightDirection": "top"}), &src, w, h);
    let edge = (0..h).map(|y| (y, i32::from(at(&below, w, 8, y)) - i32::from(at(&above, w, 8, y)))).max_by_key(|(_, d)| d.abs());
    assert!(edge.is_some_and(|(_, d)| d.abs() > 100), "{edge:?}");
    // Flat areas are grey.
    for y in [2, 45] {
        let v = at(&below, w, 8, y);
        assert!((90..190).contains(&v), "flat {v}");
    }
    survives(
        "sketch.basRelief",
        &[json!({}), json!({"detail": "NaN", "smoothness": 1e308, "lightDirection": 3}), json!({"detail": -1e308, "smoothness": null})],
    );
}

#[test]
fn sketch_filters_reach_beyond_nothing_they_do_not_paint() {
    for id in ["sketch.basRelief", "sketch.charcoal", "sketch.conteCrayon", "sketch.photocopy", "sketch.stamp"] {
        let f = fx(id, json!({}));
        assert_eq!(f.outset(Rect::new(0.0, 0.0, 50.0, 50.0)), 0.0, "{id} keeps the object's coverage");
        assert!(f.reach().is_some_and(|r| r.is_finite() && r > 0.0), "{id} reach");
    }
}

/// The mean of channel 0 over columns `x0..x1` of every row.
fn mean(d: &[u8], w: usize, h: usize, x0: usize, x1: usize) -> f64 {
    let sum: u32 = (0..h).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| u32::from(at(d, w, x, y))).sum();
    f64::from(sum) / ((x1 - x0) * h) as f64
}

/// How many pixels of channel 0 are lighter than 192.
fn white(d: &[u8]) -> usize {
    d.as_chunks::<4>().0.iter().filter(|p| p[0] > 192).count()
}

#[test]
fn conte_crayon_params_take_defaults_and_stay_in_range() {
    let defaults = PixelFx::ConteCrayon {
        foreground: 12.0,
        background: 10.0,
        texture: Texture::Canvas,
        scaling: 1.0,
        relief: 4.0,
        light: Light::Top,
        invert: false,
    };
    assert_eq!(fx("sketch.conteCrayon", json!({})), defaults);
    assert_eq!(
        fx(
            "sketch.conteCrayon",
            json!({"foregroundLevel": -5, "backgroundLevel": 1e308, "texture": "Brick", "scaling": 10, "relief": 99, "lightDirection": "bottomRight", "invert": true})
        ),
        PixelFx::ConteCrayon {
            foreground: 0.0,
            background: 20.0,
            texture: Texture::Brick,
            scaling: 0.5,
            relief: 50.0,
            light: Light::BottomRight,
            invert: true
        }
    );
    for junk in [
        json!({"foregroundLevel": "NaN", "scaling": "1e999"}),
        json!({"backgroundLevel": null, "texture": 3, "relief": [1], "lightDirection": "up", "invert": "yes"}),
    ] {
        assert_eq!(fx("sketch.conteCrayon", junk.clone()), defaults, "{junk}");
    }
    let doc = crate::effect_info("sketch.conteCrayon").unwrap().params;
    assert!(doc.contains("unverified") && doc.contains("foregroundLevel") && doc.contains("backgroundLevel"), "{doc}");
}

/// A grey ramp: its shadows in black crayon, its highlights in white crayon, the midtones on grey
/// paper between them; grey only, coverage kept, the same on every run.
#[test]
fn conte_crayon_draws_the_shadows_black_and_the_highlights_white() {
    let (w, h) = (96, 32);
    let src = image(w, h, |x, _| grey((x * 255 / (w - 1)) as u8));
    let out = run("sketch.conteCrayon", json!({"relief": 0}), &src, w, h);
    for p in out.as_chunks::<4>().0 {
        assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 255, "{p:?}");
    }
    let (shadows, mids, lights) = (mean(&out, w, h, 0, 12), mean(&out, w, h, w / 2 - 6, w / 2 + 6), mean(&out, w, h, w - 12, w));
    assert!(shadows < 40.0, "shadows {shadows}");
    assert!(lights > 215.0, "highlights {lights}");
    assert!(shadows + 30.0 < mids && mids + 30.0 < lights, "{shadows} {mids} {lights}");
    assert_eq!(out, run("sketch.conteCrayon", json!({"relief": 0}), &src, w, h), "deterministic");
    // The relief shades the paper but keeps it grey.
    let lit = run("sketch.conteCrayon", json!({}), &src, w, h);
    assert_ne!(lit, out);
    assert!(lit.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
}

/// Foreground Level spreads the black crayon into the midtones, Background Level the white one.
#[test]
fn conte_crayon_levels_spread_each_crayon_into_the_midtones() {
    let (w, h) = (64, 64);
    let mid = image(w, h, |_, _| grey(128));
    let with = |p: serde_json::Value| run("sketch.conteCrayon", p, &mid, w, h);
    let (few, many) = (with(json!({"foregroundLevel": 0, "relief": 0})), with(json!({"foregroundLevel": 20, "relief": 0})));
    assert!(black(&many) > black(&few) + w * h / 4, "{} vs {}", black(&many), black(&few));
    let (few, many) = (
        with(json!({"foregroundLevel": 0, "backgroundLevel": 0, "relief": 0})),
        with(json!({"foregroundLevel": 0, "backgroundLevel": 20, "relief": 0})),
    );
    assert!(white(&many) > white(&few) + w * h / 4, "{} vs {}", white(&many), white(&few));
}

/// The paper's texture, scale and inversion break the crayon differently; the relief is lit from
/// the chosen side.
#[test]
fn conte_crayon_texture_options_change_the_paper() {
    let (w, h) = (64, 64);
    let mid = image(w, h, |x, y| grey((64 + x + y) as u8));
    let with = |p: serde_json::Value| run("sketch.conteCrayon", p, &mid, w, h);
    let canvas = with(json!({"relief": 0}));
    for other in [
        json!({"relief": 0, "texture": "brick"}),
        json!({"relief": 0, "texture": "sandstone"}),
        json!({"relief": 0, "scaling": 200}),
        json!({"relief": 0, "invert": true}),
    ] {
        assert_ne!(with(other.clone()), canvas, "{other}");
    }
    let top = with(json!({"texture": "brick", "relief": 20, "lightDirection": "top"}));
    let bottom = with(json!({"texture": "brick", "relief": 20, "lightDirection": "bottom"}));
    assert_ne!(top, bottom);
    assert_ne!(top, with(json!({"texture": "brick", "relief": 0, "lightDirection": "top"})));
    survives(
        "sketch.conteCrayon",
        &[
            json!({}),
            json!({"foregroundLevel": "NaN", "backgroundLevel": 1e308, "scaling": -1e308, "relief": 1e308, "texture": "burlap", "invert": true}),
            json!({"foregroundLevel": -1e308, "backgroundLevel": [1], "texture": {}, "lightDirection": 3, "relief": "NaN"}),
        ],
    );
}
