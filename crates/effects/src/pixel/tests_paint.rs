//! Artistic › Smudge Stick, Sponge and Watercolor.

use serde_json::json;
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, Point};

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

/// The mean of channel 0 over the columns `x0..x1`.
fn mean(d: &[u8], w: usize, x0: usize, x1: usize) -> f64 {
    let rows = d.len() / 4 / w;
    let sum: f64 = (0..rows).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| f64::from(d[(y * w + x) * 4])).sum();
    sum / (rows * (x1 - x0)) as f64
}

/// The summed difference between horizontal neighbours of channel 0 over columns `x0..x1`.
fn roughness(d: &[u8], w: usize, x0: usize, x1: usize) -> f64 {
    let rows = d.len() / 4 / w;
    (0..rows)
        .flat_map(|y| (x0..x1 - 1).map(move |x| (x, y)))
        .map(|(x, y)| (f64::from(d[(y * w + x) * 4]) - f64::from(d[(y * w + x + 1) * 4])).abs())
        .sum()
}

fn spread(d: &[u8]) -> f64 {
    let v: Vec<f64> = d.as_chunks::<4>().0.iter().map(|p| f64::from(p[0])).collect();
    let m = v.iter().sum::<f64>() / v.len() as f64;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
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
        }
    }
    // A size that doesn't match, and an empty raster, do nothing.
    let mut odd = vec![1u8; 10];
    fx(id, json!({})).apply(&mut odd, 3, 3, &space(3, 3));
    assert_eq!(odd, vec![1u8; 10]);
    fx(id, json!({})).apply(&mut [], 0, 0, &space(1, 1));
}

#[test]
fn smudge_stick_smudges_the_dark_and_lightens_the_light() {
    assert_eq!(fx("artistic.smudgeStick", json!({})), PixelFx::SmudgeStick { stroke: 2.0, highlight: 12.0, intensity: 10.0 });
    assert_eq!(
        fx("artistic.smudgeStick", json!({"strokeLength": 1e308, "highlightArea": -5, "intensity": "x"})),
        PixelFx::SmudgeStick { stroke: 10.0, highlight: 0.0, intensity: 10.0 }
    );
    // A fine checkerboard of dark tones on the left, of light tones on the right, and a half
    // transparent bottom.
    let (w, h) = (64, 48);
    let src = image(w, h, |x, y| {
        let on = (x + y) % 2 == 0;
        let v = match (x < 32, on) {
            (true, true) => 20,
            (true, false) => 70,
            (false, true) => 190,
            (false, false) => 240,
        };
        if y < 40 { grey(v) } else { [v / 2, v / 2, v / 2, 128] }
    });
    let out = run("artistic.smudgeStick", json!({}), &src, w, h);
    assert_eq!(out, run("artistic.smudgeStick", json!({}), &src, w, h), "draws the same twice");
    assert!(out.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0).all(|(o, s)| o[3] == s[3]), "coverage kept");
    // The dark detail is smudged away, the light areas brighter.
    assert!(roughness(&out, w, 4, 28) < 0.5 * roughness(&src, w, 4, 28));
    assert!(mean(&out, w, 36, 60) > mean(&src, w, 36, 60) + 10.0);
    // Intensity 0 leaves the light areas unlightened.
    let flat = run("artistic.smudgeStick", json!({"intensity": 0}), &src, w, h);
    assert!(mean(&flat, w, 36, 60) < mean(&out, w, 36, 60) - 5.0);
    survives(
        "artistic.smudgeStick",
        &[json!({}), json!({"strokeLength": 1e308, "highlightArea": 1e308, "intensity": 1e308}), json!({"strokeLength": "NaN"})],
    );
}

#[test]
fn sponge_dabs_contrasting_blobs_and_keeps_the_hue() {
    assert_eq!(fx("artistic.sponge", json!({})), PixelFx::Sponge { brush: 2.0, definition: 12.0, smoothness: 5.0 });
    assert_eq!(
        fx("artistic.sponge", json!({"brushSize": -1, "definition": 1e308, "smoothness": 0})),
        PixelFx::Sponge { brush: 0.0, definition: 25.0, smoothness: 1.0 }
    );
    let (w, h) = (80, 60);
    let flat = image(w, h, |_, y| if y < 50 { [200, 90, 60, 255] } else { [100, 45, 30, 128] });
    let out = run("artistic.sponge", json!({"definition": 25}), &flat, w, h);
    assert_eq!(out, run("artistic.sponge", json!({"definition": 25}), &flat, w, h), "draws the same twice");
    assert!(out.as_chunks::<4>().0.iter().zip(flat.as_chunks::<4>().0).all(|(o, s)| o[3] == s[3]), "coverage kept");
    // A flat colour breaks up into lighter and darker blobs of the same hue.
    let top = &out[..w * 50 * 4];
    assert!(spread(top) > 15.0, "{}", spread(top));
    assert!(top.as_chunks::<4>().0.iter().all(|p| p[0] >= p[1] && p[1] >= p[2]), "red stays the strongest channel");
    // Definition 0 leaves a flat colour as it is; more definition, more contrast.
    let none = run("artistic.sponge", json!({"definition": 0}), &flat, w, h);
    assert!(spread(&none[..w * 50 * 4]) < 1.0);
    let some = run("artistic.sponge", json!({"definition": 8}), &flat, w, h);
    assert!(spread(&some[..w * 50 * 4]) < spread(top));
    survives("artistic.sponge", &[json!({}), json!({"brushSize": 1e308, "definition": 1e308, "smoothness": 1e308}), json!({"brushSize": "NaN"})]);
}

#[test]
fn watercolor_simplifies_detail_and_saturates_edges() {
    assert_eq!(fx("artistic.watercolor", json!({})), PixelFx::Watercolor { detail: 9.0, shadow: 1.0, texture: 1 });
    assert_eq!(
        fx("artistic.watercolor", json!({"brushDetail": 99, "shadowIntensity": -1, "texture": 2.6})),
        PixelFx::Watercolor { detail: 14.0, shadow: 0.0, texture: 3 }
    );
    // Noisy grey on the left, a step from light red to dark blue on the right, a transparent row.
    let (w, h) = (64, 48);
    let src = image(w, h, |x, y| {
        if y >= 44 {
            return [0; 4];
        }
        if x < 32 {
            grey(if (x * 7 + y * 13) % 5 < 2 { 90 } else { 150 })
        } else if x < 48 {
            [200, 120, 120, 255]
        } else {
            [60, 70, 110, 255]
        }
    });
    let few = run("artistic.watercolor", json!({"brushDetail": 1}), &src, w, h);
    let many = run("artistic.watercolor", json!({"brushDetail": 14}), &src, w, h);
    assert_eq!(few, run("artistic.watercolor", json!({"brushDetail": 1}), &src, w, h), "draws the same twice");
    assert!(few.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0).all(|(o, s)| o[3] == s[3]), "coverage kept, nothing painted outside");
    assert!(roughness(&few, w, 4, 28) < 0.5 * roughness(&src, w, 4, 28), "less detail, simpler");
    assert!(roughness(&few, w, 4, 28) < roughness(&many, w, 4, 28));
    // Colour is saturated where the tones change, and shadows deepen with Shadow Intensity.
    let chroma = |d: &[u8], x: usize, y: usize| {
        let i = (y * w + x) * 4;
        f64::from(d[i].max(d[i + 1]).max(d[i + 2])) - f64::from(d[i].min(d[i + 1]).min(d[i + 2]))
    };
    assert!(chroma(&few, 47, 20) > chroma(&src, 47, 20) || chroma(&few, 48, 20) > chroma(&src, 48, 20));
    assert!(mean(&run("artistic.watercolor", json!({"shadowIntensity": 10}), &src, w, h), w, 4, 28) < mean(&few, w, 4, 28));
    // Texture adds paper grain to a flat colour.
    let flat = image(w, h, |_, _| grey(128));
    let (t1, t3) = (run("artistic.watercolor", json!({"texture": 1}), &flat, w, h), run("artistic.watercolor", json!({"texture": 3}), &flat, w, h));
    assert!(spread(&t3) > spread(&t1) && spread(&t1) > 0.2);
    survives(
        "artistic.watercolor",
        &[json!({}), json!({"brushDetail": -1e308, "shadowIntensity": 1e308, "texture": 1e308}), json!({"brushDetail": "NaN"})],
    );
}
