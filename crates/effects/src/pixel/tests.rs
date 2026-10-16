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
        Some(RasterFx::Pixel(p)) => p.clone(),
        other => panic!("{id}: {other:?}"),
    }
}

fn grey(v: u8) -> [u8; 4] {
    [v, v, v, 255]
}

#[test]
fn film_grain_parameters_are_bounded_and_neutral_settings_preserve_the_art() {
    assert_eq!(fx("artistic.filmGrain", json!({})), PixelFx::FilmGrain { grain: 4.0, highlight: 0.0, intensity: 10.0 });
    assert_eq!(
        fx("artistic.filmGrain", json!({"grain": 1e308, "highlightArea": -100, "intensity": 200})),
        PixelFx::FilmGrain { grain: 20.0, highlight: 0.0, intensity: 10.0 }
    );
    assert_eq!(fx("artistic.filmGrain", json!({"grain": "1e999", "intensity": []})), fx("artistic.filmGrain", json!({})));
    let (w, h) = (24, 18);
    let src = image(w, h, |x, y| [(3 * x) as u8, (2 * y) as u8, 12, 128]);
    let mut out = src.clone();
    let neutral = fx("artistic.filmGrain", json!({"grain": 0, "highlightArea": 0, "intensity": 0}));
    neutral.apply(&mut out, w, h, &space(w, h));
    assert_eq!(out, src);
    assert_eq!(neutral.outset(Rect::new(0.0, 0.0, w as f64, h as f64)), 0.0);
    assert_eq!(neutral.reach(), Some(0.0));
}

#[test]
fn film_grain_is_monochrome_and_quieter_in_highlights() {
    let (w, h) = (64, 48);
    let grain = fx("artistic.filmGrain", json!({"grain": 20, "intensity": 0}));
    let mut changes = Vec::new();
    for tone in [64, 128, 240] {
        let mut out = image(w, h, |_, _| grey(tone));
        grain.apply(&mut out, w, h, &space(w, h));
        assert!(out.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
        changes.push(out.as_chunks::<4>().0.iter().map(|p| f64::from(p[0].abs_diff(tone))).sum::<f64>() / (w * h) as f64);
    }
    assert!(changes[0] > 3.0 * changes[2] && changes[1] > 3.0 * changes[2], "shadows and midtones take more grain: {changes:?}");
}

#[test]
fn film_grain_highlight_area_and_intensity_change_the_tones() {
    let (w, h) = (16, 16);
    let apply = |tone, area, intensity| {
        let mut out = image(w, h, |_, _| grey(tone));
        fx("artistic.filmGrain", json!({"grain": 0, "highlightArea": area, "intensity": intensity})).apply(&mut out, w, h, &space(w, h));
        out[0]
    };
    assert_eq!(apply(64, 0, 10), 64, "intensity alone doesn't recolour shadows");
    assert_eq!(apply(192, 0, 10), 192, "intensity alone doesn't add global contrast");
    assert!(apply(192, 20, 10) > apply(192, 20, 0), "intensity sets highlight strength");
    let tone = 192;
    let mut strong = image(w, h, |_, _| grey(tone));
    let mut quiet = strong.clone();
    fx("artistic.filmGrain", json!({"grain": 20, "highlightArea": 0, "intensity": 0})).apply(&mut strong, w, h, &space(w, h));
    fx("artistic.filmGrain", json!({"grain": 20, "highlightArea": 0, "intensity": 10})).apply(&mut quiet, w, h, &space(w, h));
    let spread = |data: &[u8]| data.as_chunks::<4>().0.iter().map(|p| u32::from(p[0].abs_diff(tone))).sum::<u32>();
    assert!(spread(&strong) > spread(&quiet), "Intensity also quiets highlight grain with the default Highlight Area");
    let highlights = [0, 10, 20].map(|area| apply(192, area, 10));
    assert!(highlights.windows(2).all(|p| p[0] < p[1]), "wider highlight area lifts more midtones: {highlights:?}");
    assert_eq!(apply(0, 20, 10), 0, "black doesn't become a white background");
}

#[test]
fn film_grain_preserves_coverage_without_a_halo() {
    let (w, h) = (40, 30);
    let src = image(w, h, |x, y| match (x + y) % 5 {
        0 => [0; 4],
        1 => [1, 0, 1, 1],
        2 => [75, 12, 25, 128],
        _ => [180, 60, 90, 255],
    });
    let mut out = src.clone();
    fx("artistic.filmGrain", json!({"grain": 20, "highlightArea": 20})).apply(&mut out, w, h, &space(w, h));
    assert_ne!(out, src);
    for (p, original) in out.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0) {
        assert_eq!(p[3], original[3]);
        assert!(p[..3].iter().all(|c| *c <= p[3]), "premultiplied: {p:?}");
        if original[3] == 0 {
            assert_eq!(p, original);
        }
    }
}

#[test]
fn film_grain_pattern_survives_redraws_moves_resolution_changes_and_tiles() {
    let (w, h) = (32, 24);
    let src = image(w, h, |_, _| grey(128));
    let grain = fx("artistic.filmGrain", json!({"grain": 15}));
    let mut whole = src.clone();
    grain.apply(&mut whole, w, h, &space(w, h));
    let mut again = src.clone();
    grain.apply(&mut again, w, h, &space(w, h));
    assert_eq!(whole, again);
    let moved =
        PixelSpace { to_doc: Affine::translate((17.0, -9.0)), center: space(w, h).center + vectorcraft_geom::Vec2::new(17.0, -9.0), ..space(w, h) };
    let mut shifted = src;
    grain.apply(&mut shifted, w, h, &moved);
    assert_eq!(whole, shifted);
    let mut two = image(2 * w, 2 * h, |_, _| grey(128));
    grain.apply(&mut two, 2 * w, 2 * h, &PixelSpace { to_doc: Affine::scale(0.5), px: 0.5, ..space(w, h) });
    let mut difference = 0u64;
    for y in 0..h {
        for x in 0..w {
            let down = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| u32::from(at(&two, 2 * w, 2 * x + dx, 2 * y + dy)[0]));
            let down = (down.iter().sum::<u32>() / 4) as u8;
            difference += u64::from(at(&whole, w, x, y)[0].abs_diff(down));
        }
    }
    let mean_delta = difference as f64 / (w * h) as f64;
    let fine_deviation = film_grain_deviation(two.as_chunks::<4>().0.iter().map(|p| f64::from(p[0])));
    // Two diagonal samples approximate the four point samples in the finer raster. Bound
    // their error relative to grain strength, so a stronger tonal curve does not fail it.
    assert!(mean_delta < 0.25 * fine_deviation, "the field keeps its document scale: mean delta {mean_delta}, point deviation {fine_deviation}");
    let (tw, th) = (13, 11);
    let mut tile = image(tw, th, |_, _| grey(128));
    grain.apply(&mut tile, tw, th, &PixelSpace { to_doc: Affine::translate((5.0, 3.0)), ..space(w, h) });
    for y in 0..th {
        for x in 0..tw {
            assert_eq!(at(&tile, tw, x, y), at(&whole, w, x + 5, y + 3), "no tile seam");
        }
    }
}

#[test]
fn film_grain_keeps_colour_without_an_extra_contrast_curve_and_scales_with_alpha() {
    let (w, h) = (256, 256);
    let original = [180, 100, 60, 255];
    let mut out = image(w, h, |_, _| original);
    // Intensity with no highlight area must not silently change colour or clip channels.
    fx("artistic.filmGrain", json!({"grain": 0, "intensity": 10})).apply(&mut out, w, h, &space(w, h));
    assert!(out.as_chunks::<4>().0.iter().all(|p| *p == original));
    let grain = fx("artistic.filmGrain", json!({"grain": 4}));
    grain.apply(&mut out, w, h, &space(w, h));
    for p in out.as_chunks::<4>().0 {
        // A neutral floating offset can round either side of a half-byte tie in each channel.
        assert!((i32::from(p[0]) - i32::from(p[1]) - 80).abs() <= 1, "one neutral offset, no channel clipping");
        assert!((i32::from(p[1]) - i32::from(p[2]) - 40).abs() <= 1);
    }
    // These straight colours have exact premultiplied representations at alpha 85 and 170.
    let colour = [180, 120, 60];
    let mut opaque = image(w, h, |_, _| [colour[0], colour[1], colour[2], 255]);
    grain.apply(&mut opaque, w, h, &space(w, h));
    for alpha in [85u8, 170] {
        let mut translucent = image(w, h, |_, _| {
            let [r, g, b] = colour.map(|c| (u32::from(c) * u32::from(alpha) / 255) as u8);
            [r, g, b, alpha]
        });
        grain.apply(&mut translucent, w, h, &space(w, h));
        for (p, full) in translucent.as_chunks::<4>().0.iter().zip(opaque.as_chunks::<4>().0) {
            for c in 0..3 {
                let expected = (u32::from(full[c]) * u32::from(alpha) + 127) / 255;
                assert!(i32::from(p[c]).abs_diff(expected as i32) <= 1, "alpha scales colour consistently: {p:?}, {full:?}");
            }
            assert_eq!(p[3], alpha);
        }
    }
    let mut white = image(w, h, |_, _| grey(255));
    fx("artistic.filmGrain", json!({"grain": 20, "highlightArea": 20, "intensity": 10})).apply(&mut white, w, h, &space(w, h));
    assert!(white.as_chunks::<4>().0.iter().all(|p| *p == grey(255)), "white stays clean");
}

#[test]
fn film_grain_is_visible_on_dark_fills_with_even_shadow_and_midtone_strength() {
    let (w, h) = (128, 96);
    let grain = fx("artistic.filmGrain", json!({}));
    let render = |tone| {
        let mut out = image(w, h, |_, _| grey(tone));
        grain.apply(&mut out, w, h, &space(w, h));
        out
    };
    let black = render(0);
    assert!(black.as_chunks::<4>().0.iter().any(|p| p[0] > 2), "pure black takes light grain speckles");
    assert!(film_grain_deviation(black.as_chunks::<4>().0.iter().map(|p| f64::from(p[0]))) > 2.0);
    let dark = render(26);
    let mid = render(128);
    let [dark, mid] = [dark, mid].map(|out| film_grain_deviation(out.as_chunks::<4>().0.iter().map(|p| f64::from(p[0]))));
    assert!(dark > 4.0 && (dark / mid - 1.0).abs() < 0.02, "default grain must remain even across shadows and midtones: {dark}, {mid}");
}

#[test]
fn film_grain_leaves_soft_but_visible_grain_on_light_fills_at_defaults() {
    let (w, h) = (128, 96);
    let colour = [245, 240, 225, 255];
    let mut cream = image(w, h, |_, _| colour);
    fx("artistic.filmGrain", json!({})).apply(&mut cream, w, h, &space(w, h));
    let deviation = film_grain_deviation(cream.as_chunks::<4>().0.iter().map(|p| f64::from(p[0])));
    let changed = cream.as_chunks::<4>().0.iter().filter(|p| **p != colour).count();
    assert!((0.5..3.0).contains(&deviation), "soft grain must remain visible on cream: {deviation}");
    assert!(changed > w * h / 4, "grain should cover the fill, not just a few outliers: {changed}");
    assert!(cream.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
}

#[test]
fn film_grain_tiles_match_at_fractional_centres_and_rotated_scales() {
    let (w, h) = (40, 32);
    let grain = fx("artistic.filmGrain", json!({"grain": 15}));
    for scale in [0.7, 2.5] {
        let s = PixelSpace { to_doc: Affine::rotate(0.37) * Affine::scale(scale), px: scale, center: Point::new(12.375, -4.625), ..space(w, h) };
        let mut whole = image(w, h, |_, _| grey(128));
        grain.apply(&mut whole, w, h, &s);
        let (tw, th) = (19, 13);
        let mut tile = image(tw, th, |_, _| grey(128));
        grain.apply(&mut tile, tw, th, &PixelSpace { to_doc: s.to_doc * Affine::translate((7.0, 5.0)), ..s });
        for y in 0..th {
            for x in 0..tw {
                assert_eq!(at(&tile, tw, x, y), at(&whole, w, x + 7, y + 5), "fractional rotated tile seam at scale {scale}");
            }
        }
    }
}

#[test]
fn film_grain_minification_quiets_the_noise_instead_of_aliasing_at_full_strength() {
    let (w, h) = (64, 64);
    let grain = fx("artistic.filmGrain", json!({"grain": 20}));
    let spread = |scale| {
        let mut out = image(w, h, |_, _| grey(128));
        grain.apply(&mut out, w, h, &PixelSpace { to_doc: Affine::scale(scale), px: scale, ..space(w, h) });
        out.as_chunks::<4>().0.iter().map(|p| f64::from(p[0].abs_diff(128))).sum::<f64>() / (w * h) as f64
    };
    let normal = spread(1.0);
    assert!(normal > 2.0);
    assert!(spread(2.0) < normal && spread(4.0) < normal / 2.0);
}

#[test]
fn film_grain_sampling_modes_blend_without_a_zoom_threshold_pop() {
    let (w, h) = (64, 48);
    let grain = fx("artistic.filmGrain", json!({"grain": 20}));
    for boundary in [0.5, 0.75, 1.0, 1.5, 2.0] {
        // Hold sample coordinates fixed to isolate the footprint-mode boundary. Changing
        // to_doc as well moves the texture, which can exceed this limit far from the origin.
        let render = |scale| {
            let mut out = image(w, h, |_, _| grey(128));
            grain.apply(&mut out, w, h, &PixelSpace { to_doc: Affine::scale(boundary), px: scale, ..space(w, h) });
            out
        };
        let (left, right) = (render(boundary - 1e-4), render(boundary + 1e-4));
        let change = left.iter().zip(right).map(|(a, b)| f64::from(a.abs_diff(b))).sum::<f64>() / left.len() as f64;
        assert!(change < 0.25, "no sudden grain change around px={boundary}: {change}");
    }
}

#[test]
fn film_grain_coarse_variance_tracks_an_independent_dense_footprint_average() {
    let (w, h, samples) = (32, 32, 16);
    let grain = fx("artistic.filmGrain", json!({"grain": 20}));
    let deviation = |values: &[f64]| {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
    };
    for scale in [1.0, 2.0, 4.0, 8.0] {
        let s = PixelSpace {
            to_doc: Affine::scale(scale),
            px: scale,
            center: Point::new(w as f64 * scale / 2.0 + 0.375, h as f64 * scale / 2.0 - 0.625),
            ..space(w, h)
        };
        let mut actual = image(w, h, |_, _| grey(128));
        grain.apply(&mut actual, w, h, &s);
        // 16×16 independent point samples per coarse pixel; the finest raster never takes
        // the footprint branch, so this does not reproduce the filter's quadrature weights.
        let fine_scale = scale / samples as f64;
        let mut fine = image(w * samples, h * samples, |_, _| grey(128));
        grain.apply(&mut fine, w * samples, h * samples, &PixelSpace { to_doc: Affine::scale(fine_scale), px: fine_scale, ..s });
        let mut reference = Vec::new();
        let mut coarse = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for dy in 0..samples {
                    for dx in 0..samples {
                        sum += u32::from(at(&fine, w * samples, x * samples + dx, y * samples + dy)[0]);
                    }
                }
                reference.push(f64::from(sum) / (samples * samples) as f64);
                coarse.push(f64::from(at(&actual, w, x, y)[0]));
            }
        }
        let ratio = deviation(&coarse) / deviation(&reference);
        assert!((0.75..=1.25).contains(&ratio), "bounded footprint approximation at px={scale}: relative deviation {ratio}");
    }
}

#[test]
fn film_grain_strength_is_stable_under_subpixel_centre_shifts() {
    let (w, h) = (192, 128);
    let grain = fx("artistic.filmGrain", json!({"grain": 20}));
    for scale in [0.25, 0.5, 1.0] {
        let mut deviations = Vec::new();
        for phase in [0.0, 0.25, 0.5, 0.75] {
            let mut out = image(w, h, |_, _| grey(128));
            let s = PixelSpace {
                to_doc: Affine::scale(scale),
                px: scale,
                center: Point::new(w as f64 * scale / 2.0 + phase, h as f64 * scale / 2.0 + phase),
                ..space(w, h)
            };
            grain.apply(&mut out, w, h, &s);
            deviations.push(film_grain_deviation(out.as_chunks::<4>().0.iter().map(|p| f64::from(p[0]))));
        }
        let min = deviations.iter().copied().fold(f64::INFINITY, f64::min);
        let max = deviations.iter().copied().fold(0.0, f64::max);
        assert!(min > 1.0 && max / min < 1.06, "centre phase must not change grain strength at px={scale}: {deviations:?}");
    }
}

fn film_grain_deviation(values: impl Iterator<Item = f64>) -> f64 {
    let (mut count, mut sum, mut square_sum) = (0.0, 0.0, 0.0);
    for v in values {
        count += 1.0;
        sum += v;
        square_sum += v * v;
    }
    (square_sum / count - (sum / count).powi(2)).max(0.0).sqrt()
}

#[test]
fn film_grain_preserves_byte_values_when_the_grain_is_too_small_to_round() {
    let (w, h) = (256, 256);
    let src = image(w, h, |x, y| {
        let alpha = y as u8;
        [(x as u8).min(alpha), (255 - x as u8).min(alpha), (x / 2) as u8 % alpha.max(1), alpha]
    });
    let mut out = src.clone();
    // Even the maximum field value cannot change half a premultiplied byte at this strength.
    fx("artistic.filmGrain", json!({"grain": 0.01})).apply(&mut out, w, h, &space(w, h));
    assert_eq!(out, src, "invisible noise preserves every colour/coverage combination");
}

#[test]
fn film_grain_cmy_and_k_planes_share_the_noise_and_preserve_coverage() {
    let (w, h) = (64, 48);
    let grain = fx("artistic.filmGrain", json!({"grain": 14}));
    // Complemented inks of a rich black, premultiplied by the same coverage.
    let mut cmy = image(w, h, |_, _| [70, 80, 90, 200]);
    let mut k = image(w, h, |_, _| [40, 40, 40, 200]);
    grain.apply(&mut cmy, w, h, &PixelSpace { channels: Channels::CmyPlane, ..space(w, h) });
    grain.apply(&mut k, w, h, &PixelSpace { channels: Channels::KPlane, ..space(w, h) });
    let mut changed = 0;
    for (cmy, k) in cmy.as_chunks::<4>().0.iter().zip(k.as_chunks::<4>().0) {
        assert_eq!((cmy[3], k[3]), (200, 200));
        assert!(cmy[..3].iter().all(|v| *v <= 200) && k[..3].iter().all(|v| *v <= 200));
        assert_eq!(k[0], k[1]);
        assert_eq!(k[1], k[2]);
        let (dc, dk) = (i16::from(cmy[0]) - 70, i16::from(k[0]) - 40);
        if dc != 0 && dk != 0 {
            assert_eq!(dc.signum(), dk.signum(), "the same field modulates both ink planes");
            changed += 1;
        }
    }
    assert!(changed > w * h / 2);
}

#[test]
fn film_grain_does_not_imprint_a_periodic_contrast_grid_at_close_zoom() {
    let (w, h) = (256, 256);
    let grain = fx("artistic.filmGrain", json!({"grain": 20}));
    for (scale, period) in [(0.25, 4), (0.125, 8)] {
        let mut out = image(w, h, |_, _| grey(128));
        grain.apply(&mut out, w, h, &PixelSpace { to_doc: Affine::scale(scale), px: scale, center: Point::new(32.0, 32.0), ..space(w, h) });
        // Bin by location within a document-unit lattice cell. An axis-aligned value field
        // puts stronger noise at corners than between them; rotated normalised fields do not.
        let mut deviations = Vec::new();
        for dy in 0..period {
            for dx in 0..period {
                let values = (dy..h).step_by(period).flat_map(|y| (dx..w).step_by(period).map(move |x| (x, y)));
                deviations.push(film_grain_deviation(values.map(|(x, y)| f64::from(at(&out, w, x, y)[0]))));
            }
        }
        let min = deviations.iter().copied().fold(f64::INFINITY, f64::min);
        let max = deviations.iter().copied().fold(0.0, f64::max);
        assert!(max / min < 1.12, "no visible cell-contrast grid at px={scale}: min={min}, max={max}");
    }
}

#[test]
fn film_grain_strength_falls_smoothly_across_noninteger_zoom_levels() {
    let (w, h) = (128, 96);
    let grain = fx("artistic.filmGrain", json!({"grain": 20}));
    let mut previous = f64::INFINITY;
    for scale in [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0] {
        let mut out = image(w, h, |_, _| grey(128));
        grain.apply(
            &mut out,
            w,
            h,
            &PixelSpace {
                to_doc: Affine::scale(scale),
                px: scale,
                center: Point::new(w as f64 * scale / 2.0 + 0.375, h as f64 * scale / 2.0 - 0.625),
                ..space(w, h)
            },
        );
        let deviation = film_grain_deviation(out.as_chunks::<4>().0.iter().map(|p| f64::from(p[0])));
        assert!(deviation <= previous * 1.06, "grain must not gain contrast when zooming out at px={scale}: {deviation} after {previous}");
        previous = deviation;
    }
}

#[test]
fn film_grain_leaves_invalid_coordinate_spaces_alone() {
    let (w, h) = (8, 8);
    let src = image(w, h, |_, _| grey(128));
    for invalid in [
        PixelSpace { px: 0.0, ..space(w, h) },
        PixelSpace { px: f64::NAN, ..space(w, h) },
        PixelSpace { to_doc: Affine::scale(0.0), ..space(w, h) },
        PixelSpace { center: Point::new(f64::INFINITY, 0.0), ..space(w, h) },
    ] {
        let mut out = src.clone();
        fx("artistic.filmGrain", json!({})).apply(&mut out, w, h, &invalid);
        assert_eq!(out, src);
    }
    let mut out = src.clone();
    PixelFx::FilmGrain { grain: f64::NAN, highlight: 0.0, intensity: 10.0 }.apply(&mut out, w, h, &space(w, h));
    assert_eq!(out, src, "directly constructed non-finite parameters are harmless too");
}

#[test]
fn params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("blur.radial", json!({})), PixelFx::RadialBlur { amount: 10.0, zoom: false, passes: 6 });
    assert_eq!(
        fx("blur.radial", json!({"amount": 1e308, "method": "ZOOM", "quality": "best"})),
        PixelFx::RadialBlur { amount: 100.0, zoom: true, passes: 8 }
    );
    assert_eq!(fx("blur.radial", json!({"amount": -5, "quality": "nonsense"})), PixelFx::RadialBlur { amount: 1.0, zoom: false, passes: 6 });
    assert_eq!(fx("blur.smart", json!({})), PixelFx::SmartBlur { radius: 3.0, threshold: 25.0, samples: 7, mode: SmartBlurMode::Normal });
    assert_eq!(
        fx("blur.smart", json!({"radius": "1e999", "threshold": -1, "quality": "low", "blurMode": "EDGEONLY"})),
        PixelFx::SmartBlur { radius: 3.0, threshold: 0.1, samples: 5, mode: SmartBlurMode::EdgeOnly }
    );
    assert_eq!(
        fx("blur.smart", json!({"blurMode": "overlayEdge"})),
        PixelFx::SmartBlur { radius: 3.0, threshold: 25.0, samples: 7, mode: SmartBlurMode::OverlayEdge }
    );
    assert_eq!(fx("blur.smart", json!({"blurMode": "overlay"})), fx("blur.smart", json!({})));
    // Every Mode menu value is documented in the catalogue and parses to a mode of its own.
    let doc = crate::effect_info("blur.smart").unwrap().params;
    for (i, (_, v)) in SMART_BLUR_MODES.iter().enumerate() {
        assert!(doc.contains(&format!("\"{v}\"")), "{v}");
        assert_eq!(SmartBlurMode::parse(v), [SmartBlurMode::Normal, SmartBlurMode::EdgeOnly, SmartBlurMode::OverlayEdge][i], "{v}");
    }
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

/// FNV-1a of `bytes`.
fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

#[test]
fn smart_blur_normal_mode_keeps_its_pixels() {
    let (w, h) = (48, 40);
    let src = image(w, h, |x, y| {
        let a = if x > 4 && y < 36 { 255 } else { 128 };
        [((x * 5 + y) as u8).min(a), ((y * 6) as u8).min(a), (((x * y) % 97) as u8).min(a), a]
    });
    let mut sums = Vec::new();
    for p in [json!({}), json!({"radius": 2, "threshold": 10, "quality": "high"}), json!({"radius": 12, "threshold": 40, "quality": "low"})] {
        let mut d = src.clone();
        fx("blur.smart", p.clone()).apply(&mut d, w, h, &space(w, h));
        let mut normal = src.clone();
        let mut p = p;
        p["blurMode"] = json!("normal");
        fx("blur.smart", p).apply(&mut normal, w, h, &space(w, h));
        assert_eq!(d, normal);
        sums.push(digest(&d));
    }
    // The pixels Smart Blur gave before it had a mode.
    assert_eq!(sums, [11495339692959753241, 14281410954765351056, 5111933201350526354]);
}

/// Smart Blur's test image: grays 56 and 64 in a checkerboard for x < 3 and 66 and 74 for x in 3..6
/// (steps of 2 or 18 levels at x = 3, under the threshold), gray 200 from x = 6 on (steps of 126 or
/// 134 levels). Its edges at threshold 25 are the pixels on the raster's border (transparency is
/// beyond it) and column 6, the step's lighter side.
fn smart_blur_steps() -> (usize, usize, Vec<u8>, impl Fn(usize, usize) -> bool) {
    let (w, h) = (12, 8);
    let src = image(w, h, |x, y| match x {
        0..3 => grey(if (x + y) % 2 == 0 { 56 } else { 64 }),
        3..6 => grey(if (x + y) % 2 == 0 { 66 } else { 74 }),
        _ => grey(200),
    });
    (w, h, src, move |x, y| x == 0 || y == 0 || x == w - 1 || y == h - 1 || x == 6)
}

#[test]
fn smart_blur_edge_only_draws_the_edges_white_on_black() {
    let (w, h, src, edge) = smart_blur_steps();
    let mut d = src.clone();
    fx("blur.smart", json!({"blurMode": "edgeOnly"})).apply(&mut d, w, h, &space(w, h));
    let want = image(w, h, |x, y| if edge(x, y) { [255; 4] } else { [0, 0, 0, 255] });
    assert_eq!(d, want);
    // The radius changes only the blur.
    let mut wide = src.clone();
    fx("blur.smart", json!({"blurMode": "edgeOnly", "radius": 40})).apply(&mut wide, w, h, &space(w, h));
    assert_eq!(wide, want);
    // A threshold under the checkerboard's 8 levels makes its lighter squares edges too.
    let mut low = src.clone();
    fx("blur.smart", json!({"blurMode": "edgeOnly", "threshold": 5})).apply(&mut low, w, h, &space(w, h));
    for y in 1..h - 1 {
        for x in 1..6 {
            let lighter = matches!(at(&src, w, x, y)[0], 64 | 74);
            assert_eq!(at(&low, w, x, y)[0] == 255, lighter, "({x}, {y})");
        }
    }
    // White and black keep each pixel's coverage, and transparency stays.
    let (w, h) = (8, 8);
    let shape = image(w, h, |x, y| if (2..6).contains(&x) && (2..6).contains(&y) { [64, 0, 0, 128] } else { [0; 4] });
    let mut s = shape.clone();
    fx("blur.smart", json!({"blurMode": "edgeOnly"})).apply(&mut s, w, h, &space(w, h));
    let ring = |x: usize, y: usize| (2..6).contains(&x) && (2..6).contains(&y) && !((3..5).contains(&x) && (3..5).contains(&y));
    let inside = |x: usize, y: usize| (3..5).contains(&x) && (3..5).contains(&y);
    let want = image(w, h, |x, y| match (ring(x, y), inside(x, y)) {
        (true, _) => [128; 4],
        (_, true) => [0, 0, 0, 128],
        _ => [0; 4],
    });
    assert_eq!(s, want);
}

#[test]
fn smart_blur_overlay_edge_draws_the_edges_white_over_the_blur() {
    let (w, h, src, edge) = smart_blur_steps();
    let mut normal = src.clone();
    fx("blur.smart", json!({})).apply(&mut normal, w, h, &space(w, h));
    assert_ne!(normal, src, "the checkerboard is blurred");
    let mut d = src.clone();
    fx("blur.smart", json!({"blurMode": "overlayEdge"})).apply(&mut d, w, h, &space(w, h));
    for y in 0..h {
        for x in 0..w {
            let want = if edge(x, y) { [255; 4] } else { at(&normal, w, x, y) };
            assert_eq!(at(&d, w, x, y), want, "({x}, {y})");
        }
    }
    // Parameters past their ranges, at any resolution, still finish.
    for mode in ["edgeOnly", "overlayEdge"] {
        for s in [PixelSpace { px: 1e-9, ..space(w, h) }, PixelSpace { px: 1e9, ..space(w, h) }] {
            let mut e = src.clone();
            fx("blur.smart", json!({"blurMode": mode, "radius": 1e308, "threshold": 1e308, "quality": "high"})).apply(&mut e, w, h, &s);
        }
    }
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
        let s = PixelSpace {
            to_doc: Affine::scale(1.0 / scale as f64),
            px: 1.0 / scale as f64,
            center: Point::ZERO,
            channels: Channels::Rgb,
            line: 1.0,
            extent: 0.0,
        };
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
    let s = PixelSpace { to_doc: Affine::IDENTITY, px: 1.0, center: Point::new(1000.0, 1000.0), channels: Channels::Rgb, line: 1.0, extent: 0.0 };
    let cases = [
        ("blur.radial", json!({"quality": "draft"})),
        ("blur.radial", json!({"quality": "good"})),
        ("blur.radial", json!({"quality": "best", "method": "zoom"})),
        ("blur.smart", json!({"quality": "low"})),
        ("blur.smart", json!({"quality": "high", "radius": 30})),
        ("sharpen.unsharpMask", json!({"radius": 20})),
        ("brushStrokes.accentedEdges", json!({"edgeWidth": 14})),
        ("brushStrokes.angledStrokes", json!({"strokeLength": 50})),
        ("brushStrokes.crosshatch", json!({"strokeLength": 50, "strength": 3})),
        ("brushStrokes.darkStrokes", json!({})),
        ("brushStrokes.inkOutlines", json!({"strokeLength": 50})),
        ("brushStrokes.spatter", json!({"sprayRadius": 25})),
        ("brushStrokes.sprayedStrokes", json!({"strokeLength": 20, "sprayRadius": 25})),
        ("brushStrokes.sumiE", json!({"strokeWidth": 15})),
        ("pixelate.colorHalftone", json!({"maxRadius": 4})),
        ("pixelate.crystallize", json!({"cellSize": 3})),
        ("pixelate.crystallize", json!({"cellSize": 300})),
        ("pixelate.mezzotint", json!({"type": "grainyDots"})),
        ("pixelate.pointillize", json!({"cellSize": 3})),
        ("pixelate.pointillize", json!({"cellSize": 300})),
        ("texture.craquelure", json!({"crackSpacing": 2})),
        ("texture.craquelure", json!({"crackSpacing": 100})),
        ("texture.grain", json!({"grainType": "clumped"})),
        ("texture.mosaicTiles", json!({"tileSize": 2})),
        ("texture.patchwork", json!({"squareSize": 0})),
        ("texture.stainedGlass", json!({"cellSize": 2})),
        ("texture.texturizer", json!({"texture": "sandstone", "scaling": 50})),
        ("texture.texturizer", json!({"texture": "burlap"})),
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

/// Video › De-Interlace: lines 1, 3, 5… (or 2, 4, 6…) are made again from the others, by copying
/// the line above or by averaging the lines above and below; the lines are the document's raster
/// rows, whatever the raster's own resolution.
#[test]
fn deinterlace_remakes_one_field_from_the_other() {
    let (w, h) = (2, 6);
    // Line n (from 1) is grey 40 n.
    let src = image(w, h, |_, y| grey(40 * (y as u8 + 1)));
    let run = |p: serde_json::Value, s: &PixelSpace| {
        let mut d = src.clone();
        fx("video.deinterlace", p).apply(&mut d, w, h, s);
        (0..h).map(|y| at(&d, w, 1, y)[0]).collect::<Vec<_>>()
    };
    let s = space(w, h);
    // Odd lines (1, 3, 5) copy the line above; line 1 has none, so it takes line 2.
    assert_eq!(run(json!({}), &s), [80, 80, 80, 160, 160, 240]);
    assert_eq!(run(json!({"eliminate": "even"}), &s), [40, 40, 120, 120, 200, 200]);
    // Interpolated: the average of the lines around (one side at the edges).
    assert_eq!(run(json!({"eliminate": "even", "create": "interpolation"}), &s), [40, 80, 120, 160, 200, 200]);
    // Lines two pixels tall: lines 1 and 3 (pixels 0–1 and 4–5) copy their neighbour, row by row.
    let coarse = PixelSpace { line: 2.0, ..space(w, h) };
    assert_eq!(run(json!({}), &coarse), [120, 160, 120, 160, 120, 160]);
    // A damaged space changes nothing.
    assert_eq!(run(json!({}), &PixelSpace { line: f64::NAN, ..space(w, h) }), [40, 80, 120, 160, 200, 240]);
}

/// Video › NTSC Colors: saturated yellow and cyan lose saturation (their composite signal is too
/// strong) but keep their brightness; greys, dark colours and transparency stay as they are.
#[test]
fn ntsc_colors_tame_only_colours_too_strong_for_the_signal() {
    let (w, h) = (5, 1);
    let px = [[255, 255, 0, 255], [0, 255, 255, 255], [128, 128, 128, 255], [100, 20, 20, 255], [0, 0, 0, 0]];
    let mut d: Vec<u8> = px.concat();
    fx("video.ntscColors", json!({})).apply(&mut d, w, h, &space(w, h));
    let luma = |p: [u8; 4]| 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
    for (x, &a) in px.iter().enumerate() {
        let b = at(&d, w, x, 0);
        if x >= 2 {
            assert_eq!(b, a, "{x}: unchanged");
            continue;
        }
        let spread = |p: [u8; 4]| p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap();
        assert!(spread(b) < spread(a), "{x}: less saturated {b:?}");
        assert!((luma(a) - luma(b)).abs() < 3.0, "{x}: as bright {a:?} {b:?}");
    }
}

#[test]
fn texture_params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("texture.craquelure", json!({})), PixelFx::Craquelure { spacing: 15.0, depth: 6.0, brightness: 9.0 });
    assert_eq!(
        fx("texture.craquelure", json!({"crackSpacing": 1e9, "crackDepth": -1, "crackBrightness": "x"})),
        PixelFx::Craquelure { spacing: 100.0, depth: 0.0, brightness: 9.0 }
    );
    assert_eq!(fx("texture.grain", json!({})), PixelFx::Grain { intensity: 40.0, contrast: 50.0, kind: Grain::Regular });
    assert_eq!(
        fx("texture.grain", json!({"intensity": 1e300, "contrast": -5, "grainType": "SPECKLE"})),
        PixelFx::Grain { intensity: 100.0, contrast: 0.0, kind: Grain::Speckle }
    );
    assert_eq!(fx("texture.mosaicTiles", json!({})), PixelFx::MosaicTiles { tile: 12.0, grout: 3.0, lighten: 9.0 });
    assert_eq!(
        fx("texture.mosaicTiles", json!({"tileSize": 0, "groutWidth": 99, "lightenGrout": 11})),
        PixelFx::MosaicTiles { tile: 2.0, grout: 15.0, lighten: 10.0 }
    );
    assert_eq!(fx("texture.patchwork", json!({})), PixelFx::Patchwork { square: 4.0, relief: 8.0 });
    assert_eq!(fx("texture.patchwork", json!({"squareSize": -1, "relief": 26})), PixelFx::Patchwork { square: 0.0, relief: 25.0 });
    assert_eq!(fx("texture.stainedGlass", json!({})), PixelFx::StainedGlass { cell: 10.0, border: 4.0, light: 3.0 });
    assert_eq!(
        fx("texture.stainedGlass", json!({"cellSize": 1, "borderThickness": 21, "lightIntensity": "1e999"})),
        PixelFx::StainedGlass { cell: 2.0, border: 20.0, light: 3.0 }
    );
    let canvas = PixelFx::Texturizer { texture: Texture::Canvas, scaling: 1.0, relief: 4.0, light: Light::Top, invert: false };
    assert_eq!(fx("texture.texturizer", json!({})), canvas);
    assert_eq!(
        fx("texture.texturizer", json!({"texture": "Brick", "scaling": 10, "relief": 60, "lightDirection": "bottomRight", "invert": true})),
        PixelFx::Texturizer { texture: Texture::Brick, scaling: 0.5, relief: 50.0, light: Light::BottomRight, invert: true }
    );
    assert_eq!(
        fx("texture.texturizer", json!({"texture": 3, "lightDirection": "up", "scaling": 1e9})),
        PixelFx::Texturizer { texture: Texture::Canvas, scaling: 2.0, relief: 4.0, light: Light::Top, invert: false }
    );
    assert_eq!(fx("texture.grain", json!({"grainType": ["x"]})), fx("texture.grain", json!({})));
    // Every menu value is documented in the catalogue and parses to a choice of its own.
    for (id, key, table) in [
        ("texture.grain", "grainType", GRAIN_TYPES.as_slice()),
        ("texture.texturizer", "texture", TEXTURES.as_slice()),
        ("texture.texturizer", "lightDirection", LIGHT_DIRECTIONS.as_slice()),
    ] {
        let doc = crate::effect_info(id).unwrap().params;
        let parsed: Vec<PixelFx> = table.iter().map(|(_, v)| fx(id, json!({ key: v }))).collect();
        for (i, (_, v)) in table.iter().enumerate() {
            assert!(doc.contains(&format!("\"{v}\"")), "{id} {v}");
            assert!(parsed.iter().skip(i + 1).all(|p| *p != parsed[i]), "{id} {v}");
        }
    }
    // Panes and squares reach beyond the object; the surfaces stay on it.
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    assert_eq!(fx("texture.stainedGlass", json!({"cellSize": 20})).outset(b), 30.0);
    assert_eq!(fx("texture.patchwork", json!({"squareSize": 0})).outset(b), 1.0);
    for id in ["texture.craquelure", "texture.grain", "texture.mosaicTiles", "texture.texturizer"] {
        assert_eq!(fx(id, json!({})).outset(b), 0.0, "{id}");
    }
    assert_eq!(fx("texture.grain", json!({})).reach(), Some(0.0));
    assert_eq!(fx("texture.craquelure", json!({"crackSpacing": 12})).reach(), Some(6.0));
}

/// The pixels of `d` (`w` wide) in the square `lo..hi` on both axes.
fn square(d: &[u8], w: usize, lo: usize, hi: usize) -> Vec<[u8; 4]> {
    (lo..hi).flat_map(|y| (lo..hi).map(move |x| (x, y))).map(|(x, y)| at(d, w, x, y)).collect()
}

/// A `w`-square raster of one colour.
fn flat(w: usize, c: [u8; 4]) -> Vec<u8> {
    image(w, w, |_, _| c)
}

#[test]
fn craquelure_cracks_plates_and_contours() {
    let w = 96;
    let tan = [180, 150, 110, 255];
    let mut d = flat(w, tan);
    fx("texture.craquelure", json!({"crackBrightness": 10})).apply(&mut d, w, w, &space(w, w));
    let px = square(&d, w, 16, 80);
    let dark = px.iter().filter(|p| p[0] < 120).count() as f64 / px.len() as f64;
    assert!((0.03..0.4).contains(&dark), "a network of dark cracks: {dark}");
    assert!(px.iter().all(|p| p[3] == 255 && p[0] >= p[1] && p[1] >= p[2]), "the colour stays, darkened or lit");
    assert!(px.iter().any(|p| p[0] > 182), "plate edges catch the light");
    // Without depth there are no cracks and no relief; brightness lights the surface.
    let mut flat_lit = flat(w, tan);
    fx("texture.craquelure", json!({"crackDepth": 0, "crackBrightness": 10})).apply(&mut flat_lit, w, w, &space(w, w));
    assert_eq!(flat_lit, flat(w, tan));
    let mut dim = flat(w, tan);
    fx("texture.craquelure", json!({"crackDepth": 0, "crackBrightness": 0})).apply(&mut dim, w, w, &space(w, w));
    assert_eq!(at(&dim, w, 40, 40), [90, 75, 55, 255]);
    // A dark half and a light half: a crack follows the contour between them, straight down (a
    // dark line between lighter pixels in nearly every row), where one tone has none.
    let craquelure = fx("texture.craquelure", json!({"crackSpacing": 100, "crackBrightness": 10}));
    let rows_cracked = |src: Vec<u8>| {
        let mut d = src;
        craquelure.apply(&mut d, w, w, &space(w, w));
        let v = |x: usize, y: usize| i32::from(at(&d, w, x, y)[0]);
        (20..76).filter(|y| (26..35).any(|x| v(x, *y) + 8 < v(x - 2, *y) && v(x, *y) + 8 < v(x + 2, *y))).count()
    };
    let (contour, plain) = (rows_cracked(image(w, w, |x, _| grey(if x < 30 { 60 } else { 200 }))), rows_cracked(flat(w, grey(200))));
    assert!(contour >= 48 && contour >= plain + 25, "{contour} rows cracked along the contour, {plain} without one");
}

#[test]
fn grain_types_each_add_their_own_noise() {
    let w = 48;
    let mid = grey(128);
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for (_, kind) in GRAIN_TYPES {
        let mut d = flat(w, mid);
        fx("texture.grain", json!({ "grainType": kind })).apply(&mut d, w, w, &space(w, w));
        assert_ne!(d, flat(w, mid), "{kind} adds grain");
        assert!(d.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "{kind}: coverage stays");
        assert!(!seen.contains(&d), "{kind}: a grain of its own");
        seen.push(d);
    }
    // Regular grain is 1-point grains of every colour around the image's own.
    let mut d = flat(w, mid);
    fx("texture.grain", json!({})).apply(&mut d, w, w, &space(w, w));
    let px = d.as_chunks::<4>().0;
    let mean = px.iter().map(|p| f64::from(p[0])).sum::<f64>() / px.len() as f64;
    assert!((mean - 128.0).abs() < 8.0, "{mean}");
    assert!(px.iter().any(|p| p[0] != p[1]), "coloured grains");
    // Horizontal grain runs in streaks along the rows.
    let mut d = flat(w, mid);
    fx("texture.grain", json!({"grainType": "horizontal", "intensity": 100})).apply(&mut d, w, w, &space(w, w));
    let along = (0..w - 1).map(|x| u32::from(at(&d, w, x, 10)[0].abs_diff(at(&d, w, x + 1, 10)[0]))).sum::<u32>();
    let across = (0..w - 1).map(|y| u32::from(at(&d, w, 10, y)[0].abs_diff(at(&d, w, 10, y + 1)[0]))).sum::<u32>();
    assert!(along * 3 < across, "{along} along, {across} across");
    // No intensity at neutral contrast leaves the image; contrast spreads its tones.
    let ramp = image(w, w, |x, _| grey((x * 5) as u8));
    let mut d = ramp.clone();
    fx("texture.grain", json!({"intensity": 0})).apply(&mut d, w, w, &space(w, w));
    assert_eq!(d, ramp);
    fx("texture.grain", json!({"intensity": 0, "contrast": 100})).apply(&mut d, w, w, &space(w, w));
    assert!(at(&d, w, 2, 0)[0] < at(&ramp, w, 2, 0)[0] && at(&d, w, 45, 0)[0] > at(&ramp, w, 45, 0)[0]);
    // Sprinkles are the background colour, white.
    let mut d = flat(w, [0, 0, 160, 255]);
    fx("texture.grain", json!({"grainType": "sprinkles", "intensity": 100})).apply(&mut d, w, w, &space(w, w));
    assert!(d.as_chunks::<4>().0.iter().any(|p| p[0] > 200 && p[2] > 240));
}

#[test]
fn mosaic_tiles_lay_tiles_in_light_grout() {
    let w = 96;
    let blue = [40, 60, 160, 255];
    let mut d = flat(w, blue);
    fx("texture.mosaicTiles", json!({})).apply(&mut d, w, w, &space(w, w));
    let px = square(&d, w, 8, 88);
    let grout = px.iter().filter(|p| p[0] > 170).count() as f64 / px.len() as f64;
    assert!((0.08..0.5).contains(&grout), "light grout between the tiles: {grout}");
    assert!(px.iter().filter(|p| **p == blue).count() > px.len() / 10, "tile faces keep the colour");
    // Dark grout without lightening.
    let mut d = flat(w, blue);
    fx("texture.mosaicTiles", json!({"lightenGrout": 0})).apply(&mut d, w, w, &space(w, w));
    let px = square(&d, w, 8, 88);
    let grout = px.iter().filter(|p| p[2] < 100).count() as f64 / px.len() as f64;
    assert!((0.08..0.5).contains(&grout) && px.iter().all(|p| p[0] < 170), "dark grout: {grout}");
    // Transparency around the object stays.
    let mut d = image(w, w, |x, _| if x < 48 { blue } else { [0; 4] });
    fx("texture.mosaicTiles", json!({})).apply(&mut d, w, w, &space(w, w));
    assert!((0..w).all(|y| (48..w).all(|x| at(&d, w, x, y) == [0; 4])));
}

#[test]
fn patchwork_fills_squares_with_one_colour_in_relief() {
    let w = 64;
    let src = image(w, w, |x, y| [(x * 4) as u8, (y * 4) as u8, 90, 255]);
    let mut d = src.clone();
    fx("texture.patchwork", json!({"squareSize": 8, "relief": 0})).apply(&mut d, w, w, &space(w, w));
    // Squares of 8 points (the object's centre is a corner), each one colour.
    for (i, j) in [(1, 1), (2, 5), (6, 3)] {
        let first = at(&d, w, 8 * i, 8 * j);
        assert!((0..8).all(|y| (0..8).all(|x| at(&d, w, 8 * i + x, 8 * j + y) == first)), "square {i} {j}");
    }
    assert_ne!(at(&d, w, 8, 8), at(&d, w, 16, 8));
    // In relief, the squares' sides catch the light from the top left and fall into shade.
    let mut lit = src.clone();
    fx("texture.patchwork", json!({"squareSize": 8, "relief": 25})).apply(&mut lit, w, w, &space(w, w));
    let (face, top, bottom) = (at(&lit, w, 28, 28)[1], at(&lit, w, 28, 24)[1], at(&lit, w, 28, 31)[1]);
    assert!(top > face && bottom < face, "{top} {face} {bottom}");
    // Squares around the object's edge are drawn whole.
    let mut d = image(w, w, |x, _| if x < 30 { [200, 0, 0, 255] } else { [0; 4] });
    fx("texture.patchwork", json!({"squareSize": 8, "relief": 0})).apply(&mut d, w, w, &space(w, w));
    assert!((0..w).all(|y| at(&d, w, 31, y)[3] == 255 && at(&d, w, 33, y)[3] == 0));
}

#[test]
fn stained_glass_leads_panes_and_lights_the_centre() {
    let w = 96;
    let src = image(w, w, |x, y| [(40 + x) as u8, (40 + y) as u8, 120, 255]);
    let mut d = src.clone();
    fx("texture.stainedGlass", json!({"lightIntensity": 0})).apply(&mut d, w, w, &space(w, w));
    let px = square(&d, w, 16, 80);
    let lead = px.iter().filter(|p| p[0] == 0 && p[1] == 0 && p[2] == 0).count() as f64 / px.len() as f64;
    assert!((0.1..0.6).contains(&lead), "black lead: {lead}");
    let same = (16..80).flat_map(|y| (16..79).map(move |x| (x, y))).filter(|(x, y)| at(&d, w, *x, *y) == at(&d, w, x + 1, *y)).count();
    assert!(same * 5 > 2 * 64 * 63, "single-coloured panes: {same}");
    assert!(px.iter().all(|p| p[3] == 255));
    // The light shines through the middle, not the corners.
    let mut lit = src.clone();
    fx("texture.stainedGlass", json!({"lightIntensity": 10})).apply(&mut lit, w, w, &space(w, w));
    let brighter = |x: usize, y: usize| i32::from(at(&lit, w, x, y)[2]) - i32::from(at(&d, w, x, y)[2]);
    let centre = (44..52).flat_map(|y| (44..52).map(move |x| (x, y))).map(|(x, y)| brighter(x, y)).max().unwrap();
    let corner = (0..6).flat_map(|y| (0..6).map(move |x| (x, y))).map(|(x, y)| brighter(x, y)).max().unwrap();
    assert!(centre > 40 && corner * 3 < centre, "{centre} at the centre, {corner} in the corner");
    // Panes beyond the object stay clear; lead outlines it.
    let mut d = image(w, w, |x, y| if (24..72).contains(&x) && (24..72).contains(&y) { [200, 40, 40, 255] } else { [0; 4] });
    fx("texture.stainedGlass", json!({})).apply(&mut d, w, w, &space(w, w));
    assert!((0..6).all(|y| (0..w).all(|x| at(&d, w, x, y) == [0; 4])));
    assert!(d.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
}

#[test]
fn texturizer_lights_each_surface_from_its_side() {
    let w = 64;
    let mid = grey(150);
    let run = |p: serde_json::Value| {
        let mut d = flat(w, mid);
        fx("texture.texturizer", p).apply(&mut d, w, w, &space(w, w));
        d
    };
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for (_, texture) in TEXTURES {
        let d = run(json!({ "texture": texture, "relief": 20 }));
        assert_ne!(d, flat(w, mid), "{texture}");
        assert!(!seen.contains(&d), "{texture}: a surface of its own");
        // Inverted, the lit slopes are the shaded ones, and the other way round.
        let inv = run(json!({ "texture": texture, "relief": 20, "invert": true }));
        let shift = |d: &[u8]| d.as_chunks::<4>().0.iter().map(|p| i32::from(p[0]) - 150).collect::<Vec<_>>();
        let (a, b) = (shift(&d), shift(&inv));
        let opposite = a.iter().zip(&b).filter(|(a, b)| a.abs() > 4 && b.abs() > 4 && a.signum() != b.signum()).count();
        let lit_or_shaded = a.iter().filter(|a| a.abs() > 4).count();
        assert!(lit_or_shaded > 100 && opposite * 10 > lit_or_shaded * 8, "{texture}: {opposite} of {lit_or_shaded}");
        // From the other side as well.
        assert_ne!(run(json!({ "texture": texture, "relief": 20, "lightDirection": "bottom" })), d, "{texture}");
        // No relief, no change.
        assert_eq!(run(json!({ "texture": texture, "relief": 0 })), flat(w, mid), "{texture}");
        seen.push(d);
    }
    // Scaling enlarges the surface: 200 % at 2 points a pixel draws as 100 % at 1.
    let mut big = flat(w, mid);
    let coarse = PixelSpace { to_doc: Affine::scale(2.0), px: 2.0, center: Point::new(64.0, 64.0), ..space(w, w) };
    fx("texture.texturizer", json!({"texture": "brick", "scaling": 200, "relief": 20})).apply(&mut big, w, w, &coarse);
    assert_eq!(big, run(json!({"texture": "brick", "relief": 20})));
}

/// The Texture filters' patterns lie in document space around the object's centre: the same at
/// twice the resolution, and moving with the object.
#[test]
fn texture_patterns_follow_the_object_at_any_resolution() {
    let (w, h) = (48, 48);
    let art = |x: f64, y: f64| -> [u8; 4] {
        if (8.0..40.0).contains(&x) && (8.0..40.0).contains(&y) { [(x * 5.0) as u8, (y * 5.0) as u8, 120, 255] } else { [0; 4] }
    };
    for (id, p) in [
        ("texture.craquelure", json!({"crackSpacing": 8})),
        ("texture.grain", json!({"grainType": "enlarged"})),
        ("texture.mosaicTiles", json!({"tileSize": 8, "groutWidth": 2})),
        ("texture.patchwork", json!({"squareSize": 6})),
        ("texture.stainedGlass", json!({"cellSize": 8, "borderThickness": 2})),
        ("texture.texturizer", json!({"texture": "sandstone", "relief": 10})),
    ] {
        let f = fx(id, p);
        let mut one = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut one, w, h, &space(w, h));
        let mut two = image(2 * w, 2 * h, |x, y| art((x as f64 + 0.5) / 2.0, (y as f64 + 0.5) / 2.0));
        let fine = PixelSpace { to_doc: Affine::scale(0.5), px: 0.5, ..space(w, h) };
        f.apply(&mut two, 2 * w, 2 * h, &fine);
        let down = image(w, h, |x, y| {
            let q = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| at(&two, 2 * w, 2 * x + dx, 2 * y + dy));
            std::array::from_fn(|c| (q.iter().map(|p| u32::from(p[c])).sum::<u32>() / 4) as u8)
        });
        let diff = one.iter().zip(&down).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / one.len() as f64;
        assert!(diff < 12.0, "{id}: {diff}");
        let moved = PixelSpace { to_doc: Affine::translate((7.0, 3.0)), center: Point::new(31.0, 27.0), ..space(w, h) };
        let mut shifted = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut shifted, w, h, &moved);
        assert_eq!(shifted, one, "{id}");
    }
}

#[test]
fn distort_params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("distort.diffuseGlow", json!({})), PixelFx::DiffuseGlow { graininess: 6.0, glow: 10.0, clear: 15.0 });
    assert_eq!(
        fx("distort.diffuseGlow", json!({"graininess": 1e9, "glowAmount": -3, "clearAmount": "x"})),
        PixelFx::DiffuseGlow { graininess: 10.0, glow: 0.0, clear: 15.0 }
    );
    assert_eq!(
        fx("distort.glass", json!({})),
        PixelFx::Glass { distortion: 5.0, smoothness: 3.0, texture: GlassTexture::Frosted, scaling: 1.0, invert: false }
    );
    assert_eq!(
        fx("distort.glass", json!({"distortion": 1e308, "smoothness": 0, "texture": "TINYLENS", "scaling": 1, "invert": true})),
        PixelFx::Glass { distortion: 20.0, smoothness: 1.0, texture: GlassTexture::TinyLens, scaling: 0.5, invert: true }
    );
    assert_eq!(fx("distort.glass", json!({"texture": "bubbles"})), fx("distort.glass", json!({})));
    assert_eq!(fx("distort.oceanRipple", json!({})), PixelFx::OceanRipple { size: 9.0, magnitude: 9.0 });
    assert_eq!(fx("distort.oceanRipple", json!({"rippleSize": 99, "rippleMagnitude": -1})), PixelFx::OceanRipple { size: 15.0, magnitude: 0.0 });
    // Shifted content reaches past the bounds by the longest shift, and comes from as far.
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let glass = fx("distort.glass", json!({"distortion": 20}));
    assert_eq!((glass.outset(b), glass.reach()), (10.0, Some(10.0)));
    let ripple = fx("distort.oceanRipple", json!({}));
    assert!((ripple.outset(b) - 0.03 * 9.0 * 11.0).abs() < 1e-9 && ripple.reach() == Some(ripple.outset(b)));
    assert_eq!(fx("distort.diffuseGlow", json!({})).outset(b), 0.0);
}

/// Vertical stripes 5 points wide (out of step with every glass surface), alternately light and
/// dark, over the whole raster.
fn stripes(w: usize, h: usize) -> Vec<u8> {
    image(w, h, |x, _| if (x / 5) % 2 == 0 { [230, 200, 60, 255] } else { [20, 40, 120, 255] })
}

#[test]
fn glass_bends_the_image_by_its_texture() {
    let (w, h) = (48, 40);
    let src = stripes(w, h);
    let run = |p: serde_json::Value| {
        let mut d = src.clone();
        fx("distort.glass", p).apply(&mut d, w, h, &space(w, h));
        d
    };
    // No distortion, no change.
    assert_eq!(run(json!({"distortion": 0})), src);
    let mut seen = vec![];
    for (_, texture) in GLASS_TEXTURES {
        let bent = run(json!({"texture": texture, "distortion": 12}));
        assert_ne!(bent, src, "{texture}");
        assert!(!seen.contains(&bent), "{texture} differs from the other textures");
        assert_ne!(run(json!({"texture": texture, "distortion": 12, "invert": true})), bent, "{texture} inverted");
        // A colour moves; none is invented: every pixel blends two neighbouring stripes at most
        // (those within a shift of the raster's edges also take in the transparency beyond it).
        for (x, y) in (6..w - 6).flat_map(|x| (6..h - 6).map(move |y| (x, y))) {
            let p = at(&bent, w, x, y);
            assert!(p[3] == 255 && p[0] >= 20 && p[0] <= 230, "{texture} at ({x}, {y}): {p:?}");
        }
        seen.push(bent);
    }
    // Smoother glass bends the stripes differently (more gently).
    assert_ne!(run(json!({"distortion": 12, "smoothness": 15})), run(json!({"distortion": 12, "smoothness": 1})));
}

/// The stored form of a `w` × `h` texture whose straight RGBA pixels `f` gives.
fn stored(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> String {
    LoadedTexture::from_rgba(w as u32, h as u32, &image(w, h, f)).and_then(|t| t.to_stored()).unwrap()
}

#[test]
fn a_loaded_texture_is_its_gray_over_middle_gray() {
    let t = stored(3, 1, |x, _| [[255, 255, 255, 255], [0, 0, 0, 0], [255, 0, 0, 255]][x]);
    let loaded = texture(&t).unwrap();
    assert_eq!(loaded.size(), (3, 1));
    assert_eq!(texture(&t), Some(loaded.clone()), "read again from the cache");
    // A white pixel is 1 high, a transparent one is middle gray and a red one is its luma. Heights
    // between pixel centers are interpolated, and the image repeats: x = -1 is the first pixel's
    // center, and x = 2 is the same pixel again.
    let h = |x: f64| loaded.height(Point::new(x, 0.0));
    assert_eq!(h(-1.0), 1.0);
    assert_eq!(h(0.0), 128.0 / 255.0);
    assert_eq!(h(1.0), 76.0 / 255.0);
    assert_eq!(h(2.0), 1.0);
    assert_eq!(h(-0.5), 0.5 * (1.0 + 128.0 / 255.0));
    assert!(LoadedTexture::from_rgba(0, 4, &[]).is_none() && LoadedTexture::from_rgba(2, 2, &[0; 12]).is_none());
    assert!(texture("not base64!").is_none() && texture("aGVsbG8=").is_none(), "base64 of a PNG only");
}

#[test]
fn glass_bends_the_image_by_a_loaded_texture() {
    let (w, h) = (48, 40);
    let src = stripes(w, h);
    let run = |p: serde_json::Value| {
        let mut d = src.clone();
        fx("distort.glass", p).apply(&mut d, w, h, &space(w, h));
        d
    };
    // Rings around the texture's center, 9 × 7 pixels, repeated.
    let rings = stored(9, 7, |x, y| grey((((x as f64 - 4.0).hypot(y as f64 - 3.0) * 60.0) as u32 % 256) as u8));
    let loaded = json!({"texture": "loaded", "textureImage": rings, "distortion": 12});
    let bent = run(loaded.clone());
    assert_ne!(bent, src);
    for (_, texture) in GLASS_TEXTURES {
        assert_ne!(bent, run(json!({"texture": texture, "distortion": 12})), "differs from {texture}");
    }
    assert_ne!(run(json!({"texture": "loaded", "textureImage": rings, "distortion": 12, "invert": true})), bent, "inverted");
    assert_ne!(run(json!({"texture": "loaded", "textureImage": rings, "distortion": 12, "scaling": 200})), bent, "scaled");
    for (x, y) in (6..w - 6).flat_map(|x| (6..h - 6).map(move |y| (x, y))) {
        let p = at(&bent, w, x, y);
        assert!(p[3] == 255 && p[0] >= 20 && p[0] <= 230, "at ({x}, {y}): {p:?}");
    }
    assert_eq!(run(json!({"texture": "loaded", "textureImage": rings, "distortion": 0})), src, "no distortion, no change");
    // A flat texture bends nothing, whatever its gray; so does a transparent one (middle gray).
    for flat in [grey(0), grey(200), [0, 0, 0, 0]] {
        assert_eq!(run(json!({"texture": "loaded", "textureImage": stored(5, 5, |_, _| flat), "distortion": 20})), src, "{flat:?}");
    }
    // The image is kept but not used with a built-in texture; without one, "loaded" is Frosted.
    assert_eq!(run(json!({"texture": "blocks", "textureImage": rings, "distortion": 12})), run(json!({"texture": "blocks", "distortion": 12})));
    assert_eq!(fx("distort.glass", json!({"texture": "loaded"})), fx("distort.glass", json!({})));
    assert_eq!(fx("distort.glass", json!({"texture": "LOADED", "textureImage": "not a texture"})), fx("distort.glass", json!({})));
    assert_eq!(
        fx("distort.glass", loaded),
        PixelFx::Glass { distortion: 12.0, smoothness: 3.0, texture: GlassTexture::Loaded(texture(&rings).unwrap()), scaling: 1.0, invert: false }
    );
}

#[test]
fn ocean_ripple_moves_content_no_farther_than_its_magnitude() {
    let (w, h) = (64, 32);
    // The left half white, the right half transparent.
    let src = image(w, h, |x, _| if x < 32 { grey(255) } else { [0; 4] });
    let mut d = src.clone();
    fx("distort.oceanRipple", json!({"rippleMagnitude": 0})).apply(&mut d, w, h, &space(w, h));
    assert_eq!(d, src, "no magnitude, no ripples");
    let f = fx("distort.oceanRipple", json!({"rippleSize": 6, "rippleMagnitude": 20}));
    let most = f.reach().unwrap();
    let mut d = src.clone();
    f.apply(&mut d, w, h, &space(w, h));
    assert_ne!(d, src);
    // Away from the white's edge, and from the raster's (beyond which is transparency), nothing
    // changes.
    let inside = |v: usize, n: usize| (v as f64 + 0.5).min(n as f64 - v as f64 - 0.5) > most + 1.0;
    for y in (0..h).filter(|y| inside(*y, h)) {
        for x in (0..w).filter(|x| inside(*x, w)) {
            let to_edge = (x as f64 + 0.5 - 32.0).abs();
            if to_edge > most + 1.0 {
                assert_eq!(at(&d, w, x, y), at(&src, w, x, y), "({x}, {y}) is {to_edge} from the edge, past {most}");
            }
        }
    }
    // The edge wanders: some rows reach farther right than others.
    let ends: Vec<usize> = (0..h).map(|y| (0..w).filter(|x| at(&d, w, *x, y)[3] > 127).count()).collect();
    assert!(ends.iter().min() != ends.iter().max(), "{ends:?}");
}

#[test]
fn diffuse_glow_whitens_the_highlights_and_sprinkles_grain() {
    let (w, h) = (48, 48);
    // Dark on the left, light on the right, a transparent margin round them.
    let src = image(w, h, |x, y| {
        if !(4..44).contains(&x) || !(4..44).contains(&y) {
            [0; 4]
        } else if x < 24 {
            [30, 30, 60, 255]
        } else {
            [200, 190, 170, 255]
        }
    });
    let run = |p: serde_json::Value| {
        let mut d = src.clone();
        fx("distort.diffuseGlow", p).apply(&mut d, w, h, &space(w, h));
        d
    };
    let clean = run(json!({"graininess": 0}));
    // Transparency stays; the shadows stay clear; the highlights glow.
    for (a, b) in clean.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0) {
        assert_eq!(a[3], b[3]);
    }
    assert_eq!(at(&clean, w, 10, 24), at(&src, w, 10, 24));
    let (lit, before) = (at(&clean, w, 36, 24), at(&src, w, 36, 24));
    assert!((0..3).all(|c| lit[c] > before[c]), "{lit:?} brighter than {before:?}");
    // More Clear Amount leaves more of the image clear of glow.
    assert!(at(&run(json!({"graininess": 0, "clearAmount": 20})), w, 36, 24)[0] < lit[0]);
    assert_eq!(run(json!({"graininess": 0, "glowAmount": 0})), src);
    // Grain: white specks, in the shadows too.
    let grainy = run(json!({"graininess": 10}));
    let specks = (4..24).flat_map(|x| (4..44).map(move |y| (x, y))).filter(|(x, y)| at(&grainy, w, *x, *y)[0] > 100).count();
    assert!(specks > 10, "{specks} white specks in the shadows");
}

#[test]
fn distort_patterns_follow_the_object_at_any_resolution() {
    let (w, h) = (48, 48);
    let art = |x: f64, y: f64| -> [u8; 4] {
        if (8.0..40.0).contains(&x) && (8.0..40.0).contains(&y) { [(x * 5.0) as u8, (y * 5.0) as u8, 120, 255] } else { [0; 4] }
    };
    for (id, p) in [
        ("distort.diffuseGlow", json!({"clearAmount": 5})),
        ("distort.glass", json!({"texture": "blocks", "distortion": 8})),
        ("distort.oceanRipple", json!({"rippleMagnitude": 12})),
    ] {
        let f = fx(id, p);
        let mut one = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut one, w, h, &space(w, h));
        let mut two = image(2 * w, 2 * h, |x, y| art((x as f64 + 0.5) / 2.0, (y as f64 + 0.5) / 2.0));
        let fine = PixelSpace { to_doc: Affine::scale(0.5), px: 0.5, ..space(w, h) };
        f.apply(&mut two, 2 * w, 2 * h, &fine);
        let down = image(w, h, |x, y| {
            let q = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| at(&two, 2 * w, 2 * x + dx, 2 * y + dy));
            std::array::from_fn(|c| (q.iter().map(|p| u32::from(p[c])).sum::<u32>() / 4) as u8)
        });
        let diff = one.iter().zip(&down).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / one.len() as f64;
        assert!(diff < 12.0, "{id}: {diff}");
        let moved = PixelSpace { to_doc: Affine::translate((7.0, 3.0)), center: Point::new(31.0, 27.0), ..space(w, h) };
        let mut shifted = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut shifted, w, h, &moved);
        assert_eq!(shifted, one, "{id}");
    }
}

/// The Brush Strokes effect ids.
fn brush_strokes() -> impl Iterator<Item = &'static str> {
    PIXEL_EFFECTS.into_iter().filter(|id| id.starts_with("brushStrokes."))
}

#[test]
fn brush_strokes_params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("brushStrokes.accentedEdges", json!({})), PixelFx::AccentedEdges { width: 2.0, brightness: 38.0, smoothness: 5.0 });
    assert_eq!(
        fx("brushStrokes.accentedEdges", json!({"edgeWidth": 99, "edgeBrightness": -1, "smoothness": "x"})),
        PixelFx::AccentedEdges { width: 14.0, brightness: 0.0, smoothness: 5.0 }
    );
    assert_eq!(fx("brushStrokes.angledStrokes", json!({})), PixelFx::AngledStrokes { balance: 50.0, length: 15.0, sharpness: 3.0 });
    assert_eq!(
        fx("brushStrokes.angledStrokes", json!({"directionBalance": 1e308, "strokeLength": 0, "sharpness": 11})),
        PixelFx::AngledStrokes { balance: 100.0, length: 3.0, sharpness: 10.0 }
    );
    assert_eq!(fx("brushStrokes.crosshatch", json!({})), PixelFx::Crosshatch { length: 9.0, sharpness: 6.0, strength: 1 });
    assert_eq!(
        fx("brushStrokes.crosshatch", json!({"strokeLength": 51, "sharpness": -1, "strength": 2.6})),
        PixelFx::Crosshatch { length: 50.0, sharpness: 0.0, strength: 3 }
    );
    assert_eq!(fx("brushStrokes.crosshatch", json!({"strength": -7})), PixelFx::Crosshatch { length: 9.0, sharpness: 6.0, strength: 1 });
    assert_eq!(fx("brushStrokes.darkStrokes", json!({})), PixelFx::DarkStrokes { balance: 5.0, black: 6.0, white: 2.0 });
    assert_eq!(
        fx("brushStrokes.darkStrokes", json!({"balance": 11, "blackIntensity": -1, "whiteIntensity": 1e9})),
        PixelFx::DarkStrokes { balance: 10.0, black: 0.0, white: 10.0 }
    );
    assert_eq!(fx("brushStrokes.inkOutlines", json!({})), PixelFx::InkOutlines { length: 4.0, dark: 20.0, light: 10.0 });
    assert_eq!(
        fx("brushStrokes.inkOutlines", json!({"strokeLength": 0, "darkIntensity": 51, "lightIntensity": -5})),
        PixelFx::InkOutlines { length: 1.0, dark: 50.0, light: 0.0 }
    );
    assert_eq!(fx("brushStrokes.spatter", json!({})), PixelFx::Spatter { radius: 10.0, smoothness: 5.0 });
    assert_eq!(fx("brushStrokes.spatter", json!({"sprayRadius": 26, "smoothness": 0})), PixelFx::Spatter { radius: 25.0, smoothness: 1.0 });
    assert_eq!(
        fx("brushStrokes.sprayedStrokes", json!({})),
        PixelFx::SprayedStrokes { length: 12.0, radius: 7.0, direction: StrokeDirection::RightDiagonal }
    );
    assert_eq!(
        fx("brushStrokes.sprayedStrokes", json!({"strokeLength": -1, "sprayRadius": 1e308, "strokeDirection": "VERTICAL"})),
        PixelFx::SprayedStrokes { length: 0.0, radius: 25.0, direction: StrokeDirection::Vertical }
    );
    assert_eq!(fx("brushStrokes.sprayedStrokes", json!({"strokeDirection": "diagonal"})), fx("brushStrokes.sprayedStrokes", json!({})));
    assert_eq!(fx("brushStrokes.sumiE", json!({})), PixelFx::SumiE { width: 10.0, pressure: 2.0, contrast: 16.0 });
    assert_eq!(
        fx("brushStrokes.sumiE", json!({"strokeWidth": 2, "strokePressure": 16, "contrast": -1})),
        PixelFx::SumiE { width: 3.0, pressure: 15.0, contrast: 0.0 }
    );
    // Outsets: the strokes and the shifts reach out of the object as far as into it; Accented
    // Edges and Sumi-e keep the object's shape.
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    let reach = brushstrokes::stroke_reach;
    assert!((reach(20.0) - (0.75 * 20.0 + 1.5 / 15.0 * 20.0 + 1.0)).abs() < 1e-9);
    assert_eq!(fx("brushStrokes.accentedEdges", json!({})).outset(b), 0.0);
    assert_eq!(fx("brushStrokes.accentedEdges", json!({})).reach(), Some(3.0 * (0.35 * 2.0 + 0.5) + 1.0));
    assert_eq!(fx("brushStrokes.angledStrokes", json!({})).outset(b), reach(15.0));
    assert_eq!(fx("brushStrokes.crosshatch", json!({})).outset(b), reach(9.0) + 0.9);
    assert_eq!(fx("brushStrokes.darkStrokes", json!({})).outset(b), reach(9.0));
    assert_eq!(fx("brushStrokes.inkOutlines", json!({"strokeLength": 50})).outset(b), reach(50.0));
    let spatter = fx("brushStrokes.spatter", json!({}));
    assert_eq!((spatter.outset(b), spatter.reach()), (4.0, Some(4.0)));
    assert_eq!(fx("brushStrokes.spatter", json!({"sprayRadius": 0})).outset(b), 0.0);
    assert_eq!(fx("brushStrokes.sprayedStrokes", json!({})).outset(b), reach(12.0) + 0.3 * 7.0);
    assert_eq!(fx("brushStrokes.sumiE", json!({})).outset(b), 0.0);
    for id in brush_strokes() {
        let f = fx(id, json!({}));
        assert!(f.reach().is_some_and(|r| r >= f.outset(b)), "{id}");
    }
}

/// A light square in a darker ground with a ramp across it, over the whole raster: edges,
/// shadows and highlights for the strokes to work on.
fn scene(w: usize, h: usize) -> Vec<u8> {
    image(w, h, |x, y| if (12..36).contains(&x) && (12..36).contains(&y) { [235, 220, 190, 255] } else { [(30 + x) as u8, 40, 90, 255] })
}

/// `src` (`w` × `h`) through effect `id` with parameters `p`.
fn run_fx(src: &[u8], w: usize, h: usize, id: &str, p: serde_json::Value) -> Vec<u8> {
    let mut d = src.to_vec();
    fx(id, p).apply(&mut d, w, h, &space(w, h));
    d
}

#[test]
fn brush_strokes_each_repaint_the_art_their_own_way() {
    let (w, h) = (48, 48);
    let src = scene(w, h);
    let mut seen: Vec<(&str, Vec<u8>)> = vec![];
    for id in brush_strokes() {
        let d = run_fx(&src, w, h, id, json!({}));
        assert_ne!(d, src, "{id} changes the art");
        for (other, o) in &seen {
            assert_ne!(&d, o, "{id} differs from {other}");
        }
        seen.push((id, d));
    }
    assert_eq!(seen.len(), 8);
    // Each stroke direction paints its own way.
    let mut ways: Vec<Vec<u8>> = vec![];
    for (_, way) in STROKE_DIRECTIONS {
        let d = run_fx(&src, w, h, "brushStrokes.sprayedStrokes", json!({"strokeDirection": way}));
        assert!(!ways.contains(&d), "{way} differs from the other directions");
        ways.push(d);
    }
    // Every option shows.
    for (id, a, b) in [
        ("brushStrokes.accentedEdges", json!({"edgeWidth": 1}), json!({"edgeWidth": 14})),
        ("brushStrokes.accentedEdges", json!({"smoothness": 1}), json!({"smoothness": 15})),
        ("brushStrokes.angledStrokes", json!({"directionBalance": 0}), json!({"directionBalance": 100})),
        ("brushStrokes.angledStrokes", json!({"strokeLength": 3}), json!({"strokeLength": 50})),
        ("brushStrokes.angledStrokes", json!({"sharpness": 0}), json!({"sharpness": 10})),
        ("brushStrokes.crosshatch", json!({"strokeLength": 3}), json!({"strokeLength": 50})),
        ("brushStrokes.crosshatch", json!({"sharpness": 0}), json!({"sharpness": 20})),
        ("brushStrokes.crosshatch", json!({"strength": 1}), json!({"strength": 3})),
        ("brushStrokes.darkStrokes", json!({"balance": 0}), json!({"balance": 10})),
        ("brushStrokes.darkStrokes", json!({"blackIntensity": 0}), json!({"blackIntensity": 10})),
        ("brushStrokes.darkStrokes", json!({"whiteIntensity": 0}), json!({"whiteIntensity": 10})),
        ("brushStrokes.inkOutlines", json!({"strokeLength": 1}), json!({"strokeLength": 50})),
        ("brushStrokes.inkOutlines", json!({"darkIntensity": 0}), json!({"darkIntensity": 50})),
        ("brushStrokes.inkOutlines", json!({"lightIntensity": 0}), json!({"lightIntensity": 50})),
        ("brushStrokes.spatter", json!({"sprayRadius": 5}), json!({"sprayRadius": 25})),
        ("brushStrokes.spatter", json!({"smoothness": 1}), json!({"smoothness": 15})),
        ("brushStrokes.sprayedStrokes", json!({"strokeLength": 0}), json!({"strokeLength": 20})),
        ("brushStrokes.sprayedStrokes", json!({"sprayRadius": 0}), json!({"sprayRadius": 25})),
        ("brushStrokes.sumiE", json!({"strokeWidth": 3}), json!({"strokeWidth": 15})),
        ("brushStrokes.sumiE", json!({"strokePressure": 0}), json!({"strokePressure": 15})),
        ("brushStrokes.sumiE", json!({"contrast": 0}), json!({"contrast": 40})),
    ] {
        assert_ne!(run_fx(&src, w, h, id, a.clone()), run_fx(&src, w, h, id, b.clone()), "{id}: {a} and {b}");
    }
}

/// The mean of the colour channels over columns `xs` and rows `ys` (0..255).
fn mean_tone(d: &[u8], w: usize, xs: std::ops::Range<usize>, ys: std::ops::Range<usize>) -> f64 {
    let px: Vec<[u8; 4]> = ys.flat_map(|y| xs.clone().map(move |x| (x, y))).map(|(x, y)| at(d, w, x, y)).collect();
    px.iter().map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>() / (3.0 * px.len().max(1) as f64)
}

/// Dark on the left, light on the right, opaque over the whole raster.
fn halves(w: usize, h: usize) -> Vec<u8> {
    image(w, h, |x, _| if x < w / 2 { [50, 60, 100, 255] } else { [210, 200, 180, 255] })
}

#[test]
fn accented_edges_draw_chalk_or_ink_on_the_edges_only() {
    let (w, h) = (48, 48);
    // The halves inside a transparent margin.
    let both = halves(w, h);
    let src = image(w, h, |x, y| if (4..44).contains(&x) && (4..44).contains(&y) { at(&both, w, x, y) } else { [0; 4] });
    let chalk = run_fx(&src, w, h, "brushStrokes.accentedEdges", json!({"edgeBrightness": 50}));
    let ink = run_fx(&src, w, h, "brushStrokes.accentedEdges", json!({"edgeBrightness": 0}));
    let sum = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    // Along the edge between the halves: white chalk on the dark side, black ink on the light.
    assert!(sum(at(&chalk, w, 23, 24)) > sum(at(&src, w, 23, 24)) + 150, "{:?}", at(&chalk, w, 23, 24));
    assert!(sum(at(&ink, w, 24, 24)) + 150 < sum(at(&src, w, 24, 24)), "{:?}", at(&ink, w, 24, 24));
    // Away from the edges nothing changes, and the object keeps its shape.
    for (x, y) in [(13, 24), (34, 24), (13, 13)] {
        assert_eq!(at(&chalk, w, x, y), at(&src, w, x, y), "({x}, {y})");
    }
    for (a, b) in chalk.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0) {
        assert_eq!(a[3], b[3]);
    }
}

#[test]
fn dark_and_ink_strokes_darken_the_shadows_and_sumi_e_inks_them_black() {
    let (w, h) = (64, 48);
    let src = halves(w, h);
    let (shadow, light) = (8..24, 40..56);
    let tone = |d: &[u8], xs: std::ops::Range<usize>| mean_tone(d, w, xs, 8..40);
    let dark = run_fx(&src, w, h, "brushStrokes.darkStrokes", json!({}));
    assert!(tone(&dark, shadow.clone()) < tone(&src, shadow.clone()) - 20.0, "dark strokes in the shadows");
    assert!(tone(&dark, light.clone()) > tone(&src, light.clone()) + 5.0, "white strokes in the light");
    let ink = run_fx(&src, w, h, "brushStrokes.inkOutlines", json!({}));
    assert!(tone(&ink, shadow.clone()) < tone(&src, shadow.clone()) - 10.0, "ink in the shadows");
    // The outline along the edge is darker than the light side around it.
    assert!(tone(&ink, 32..34) + 40.0 < tone(&ink, light.clone()), "an ink outline along the edge");
    let sumi = run_fx(&src, w, h, "brushStrokes.sumiE", json!({}));
    assert!(tone(&sumi, shadow.clone()) < 0.4 * tone(&src, shadow.clone()), "rich blacks: {}", tone(&sumi, shadow.clone()));
    assert!(tone(&sumi, 50..56) > 150.0, "the light stays light: {}", tone(&sumi, 50..56));
}

#[test]
fn sumi_e_keeps_the_shape_and_the_hues() {
    let (w, h) = (48, 48);
    // A strong yellow square and a dark blue one in a transparent margin.
    let src = image(w, h, |x, y| match (x, y) {
        (8..24, 8..40) => [230, 200, 40, 255],
        (24..40, 8..40) => [30, 40, 120, 255],
        _ => [0; 4],
    });
    let d = run_fx(&src, w, h, "brushStrokes.sumiE", json!({"contrast": 40, "strokePressure": 15}));
    // Transparency stays transparent and the coverage stays as it was: no fringe.
    for (a, b) in d.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0) {
        assert_eq!(a[3], b[3]);
        if b[3] == 0 {
            assert_eq!(*a, [0; 4]);
        }
    }
    // The yellow stays yellow, no more saturated than it was.
    let sat = |p: [u8; 4]| {
        let (hi, lo) = (p[..3].iter().max().copied().unwrap(), p[..3].iter().min().copied().unwrap());
        f64::from(hi - lo) / f64::from(hi.max(1))
    };
    let y = at(&d, w, 12, 24);
    assert!(y[0] > y[2] + 80 && y[1] > y[2] + 80, "{y:?}");
    assert!(sat(y) <= sat([230, 200, 40, 255]) + 0.02, "{y:?}");
    // The dark blue is inked darker, still blue.
    let b = at(&d, w, 34, 24);
    assert!(b[2] < 120 && b[2] >= b[0] && b[2] >= b[1], "{b:?}");
}

#[test]
fn crosshatch_hatches_flat_colour_and_keeps_its_tone() {
    let (w, h) = (48, 48);
    let src = image(w, h, |_, _| grey(128));
    let spread = |d: &[u8]| {
        let m = mean_tone(d, w, 8..40, 8..40);
        let var = (8..40).flat_map(|x| (8..40).map(move |y| (x, y))).map(|(x, y)| (f64::from(at(d, w, x, y)[0]) - m).powi(2)).sum::<f64>() / 1024.0;
        (m, var.sqrt())
    };
    let (one, three) =
        (run_fx(&src, w, h, "brushStrokes.crosshatch", json!({})), run_fx(&src, w, h, "brushStrokes.crosshatch", json!({"strength": 3})));
    let ((m1, s1), (_, s3)) = (spread(&one), spread(&three));
    assert!((m1 - 128.0).abs() < 25.0 && s1 > 3.0, "{m1} ± {s1}");
    assert!(s3 > s1, "more passes, more hatching: {s3} vs {s1}");
}

#[test]
fn stroke_directions_smear_along_themselves() {
    let (w, h) = (48, 48);
    // A white line two pixels thick across black.
    let src = image(w, h, |_, y| if (23..25).contains(&y) { grey(255) } else { grey(0) });
    let row = |d: &[u8], y: usize| mean_tone(d, w, 12..36, y..y + 1);
    let along = run_fx(&src, w, h, "brushStrokes.sprayedStrokes", json!({"sprayRadius": 0, "strokeDirection": "horizontal"}));
    let across = run_fx(&src, w, h, "brushStrokes.sprayedStrokes", json!({"sprayRadius": 0, "strokeDirection": "vertical"}));
    assert!(row(&along, 20) < 1.0 && row(&along, 24) > 250.0, "strokes along the line keep it");
    assert!(row(&across, 20) > 15.0 && row(&across, 24) < 200.0, "strokes across it spread it: {} {}", row(&across, 20), row(&across, 24));
    // Diagonal strokes spread it too, along the diagonal.
    let diagonal = run_fx(&src, w, h, "brushStrokes.sprayedStrokes", json!({"sprayRadius": 0}));
    assert!(row(&diagonal, 20) > 5.0);
}

#[test]
fn spatter_scatters_content_no_farther_than_its_radius() {
    let (w, h) = (64, 32);
    // The left half white, the right half transparent.
    let src = image(w, h, |x, _| if x < 32 { grey(255) } else { [0; 4] });
    assert_eq!(run_fx(&src, w, h, "brushStrokes.spatter", json!({"sprayRadius": 0})), src, "no radius, no spatter");
    let f = fx("brushStrokes.spatter", json!({"sprayRadius": 15}));
    let most = f.reach().unwrap();
    let d = run_fx(&src, w, h, "brushStrokes.spatter", json!({"sprayRadius": 15}));
    assert_ne!(d, src);
    let inside = |v: usize, n: usize| (v as f64 + 0.5).min(n as f64 - v as f64 - 0.5) > most + 1.0;
    for y in (0..h).filter(|y| inside(*y, h)) {
        for x in (0..w).filter(|x| inside(*x, w) && (*x as f64 + 0.5 - 32.0).abs() > most + 1.0) {
            assert_eq!(at(&d, w, x, y), at(&src, w, x, y), "({x}, {y})");
        }
    }
    // The edge is spattered: white specks past it and holes before it.
    let past = (33..36).flat_map(|x| (8..24).map(move |y| (x, y))).filter(|(x, y)| at(&d, w, *x, *y)[3] > 127).count();
    let holes = (28..31).flat_map(|x| (8..24).map(move |y| (x, y))).filter(|(x, y)| at(&d, w, *x, *y)[3] < 128).count();
    assert!(past > 0 && holes > 0, "{past} specks past the edge, {holes} holes before it");
}

#[test]
fn brush_strokes_look_no_farther_than_their_reach() {
    let (w, h) = (200, 24);
    let base = image(w, h, |x, y| [(x * 7 % 256) as u8, (y * 11 % 256) as u8, 120, 255]);
    let cases = brush_strokes().map(|id| (id, json!({}))).chain([
        ("brushStrokes.accentedEdges", json!({"edgeWidth": 14, "smoothness": 15})),
        ("brushStrokes.angledStrokes", json!({"strokeLength": 50})),
        ("brushStrokes.crosshatch", json!({"strokeLength": 50, "strength": 3})),
        ("brushStrokes.inkOutlines", json!({"strokeLength": 50})),
        ("brushStrokes.spatter", json!({"sprayRadius": 25, "smoothness": 15})),
        ("brushStrokes.sprayedStrokes", json!({"strokeLength": 20, "sprayRadius": 25, "strokeDirection": "horizontal"})),
        ("brushStrokes.sumiE", json!({"strokeWidth": 15})),
    ]);
    for (id, p) in cases {
        let reach = fx(id, p.clone()).reach().unwrap();
        // Everything from `cut` on differs; the pixels left of 20 can't tell.
        let cut = 20 + reach.ceil() as usize + 1;
        assert!(cut < w, "{id}");
        let other = image(w, h, |x, y| if x >= cut { [255 - (x * 5 % 256) as u8, 30, (y * 9 % 256) as u8, 200] } else { at(&base, w, x, y) });
        let (a, b) = (run_fx(&base, w, h, id, p.clone()), run_fx(&other, w, h, id, p.clone()));
        for (x, y) in (0..20).flat_map(|x| (0..h).map(move |y| (x, y))) {
            let (pa, pb) = (at(&a, w, x, y), at(&b, w, x, y));
            assert!(pa.iter().zip(pb).all(|(u, v)| u.abs_diff(v) <= 2), "{id} {p}: ({x}, {y}) {pa:?} vs {pb:?}, reach {reach}");
        }
    }
}

#[test]
fn brush_strokes_follow_the_object_at_any_resolution() {
    let (w, h) = (48, 48);
    let art = |x: f64, y: f64| -> [u8; 4] {
        if (8.0..40.0).contains(&x) && (8.0..40.0).contains(&y) { [(x * 5.0) as u8, (y * 5.0) as u8, 120, 255] } else { [0; 4] }
    };
    for id in brush_strokes() {
        let f = fx(id, json!({}));
        let mut one = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut one, w, h, &space(w, h));
        let mut two = image(2 * w, 2 * h, |x, y| art((x as f64 + 0.5) / 2.0, (y as f64 + 0.5) / 2.0));
        let fine = PixelSpace { to_doc: Affine::scale(0.5), px: 0.5, ..space(w, h) };
        f.apply(&mut two, 2 * w, 2 * h, &fine);
        let down = image(w, h, |x, y| {
            let q = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| at(&two, 2 * w, 2 * x + dx, 2 * y + dy));
            std::array::from_fn(|c| (q.iter().map(|p| u32::from(p[c])).sum::<u32>() / 4) as u8)
        });
        let diff = one.iter().zip(&down).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / one.len() as f64;
        assert!(diff < 12.0, "{id}: {diff}");
        let moved = PixelSpace { to_doc: Affine::translate((7.0, 3.0)), center: Point::new(31.0, 27.0), ..space(w, h) };
        let mut shifted = image(w, h, |x, y| art(x as f64 + 0.5, y as f64 + 0.5));
        f.apply(&mut shifted, w, h, &moved);
        assert_eq!(shifted, one, "{id}");
    }
}

/// Halftone Pattern of `p` over `w` × `h` pixels of `src`, laid out by `s`.
fn halftone(p: serde_json::Value, src: &[u8], w: usize, h: usize, s: &PixelSpace) -> Vec<u8> {
    let mut out = src.to_vec();
    fx("sketch.halftonePattern", p).apply(&mut out, w, h, s);
    out
}

/// The smallest period of `values` (their length when they don't repeat).
fn period(values: &[u8]) -> usize {
    (1..values.len()).find(|p| values.iter().zip(&values[*p..]).all(|(a, b)| a == b)).unwrap_or(values.len())
}

#[test]
fn halftone_pattern_params_take_defaults_and_stay_in_range() {
    assert_eq!(fx("sketch.halftonePattern", json!({})), PixelFx::HalftonePattern { size: 1.0, contrast: 5.0, kind: HalftonePattern::Dot });
    assert_eq!(
        fx("sketch.halftonePattern", json!({"size": 1e9, "contrast": -3, "patternType": "LINE"})),
        PixelFx::HalftonePattern { size: 12.0, contrast: 0.0, kind: HalftonePattern::Line }
    );
    assert_eq!(
        fx("sketch.halftonePattern", json!({"size": 0, "contrast": 1e9, "patternType": "circle"})),
        PixelFx::HalftonePattern { size: 1.0, contrast: 50.0, kind: HalftonePattern::Circle }
    );
    for junk in [json!({"patternType": 2}), json!({"patternType": "nonsense"})] {
        assert_eq!(fx("sketch.halftonePattern", junk), fx("sketch.halftonePattern", json!({})));
    }
    let doc = crate::effect_info("sketch.halftonePattern").unwrap().params;
    let kinds: Vec<HalftonePattern> = HALFTONE_TYPES.iter().map(|(_, v)| HalftonePattern::parse(v)).collect();
    assert_eq!(kinds, [HalftonePattern::Circle, HalftonePattern::Dot, HalftonePattern::Line]);
    assert!(HALFTONE_TYPES.iter().all(|(_, v)| doc.contains(&format!("\"{v}\""))), "{doc}");
    let f = fx("sketch.halftonePattern", json!({"size": 12}));
    assert_eq!(f.outset(Rect::new(0.0, 0.0, 100.0, 50.0)), 0.0);
    assert_eq!(f.reach(), Some(0.0));
}

/// Line's rows repeat every Size points: Size pixels at 72 ppi, twice as many at 144 ppi.
#[test]
fn halftone_line_period_is_the_size_at_any_resolution() {
    let p = json!({"size": 4, "contrast": 50, "patternType": "line"});
    let (w, h) = (48, 48);
    let out = halftone(p.clone(), &image(w, h, |_, _| grey(128)), w, h, &space(w, h));
    let column: Vec<u8> = (0..h).map(|y| at(&out, w, 0, y)[0]).collect();
    assert!(column.contains(&0) && column.contains(&255), "{column:?}");
    assert_eq!(period(&column), 4, "{column:?}");
    let fine = PixelSpace { to_doc: Affine::scale(0.5), px: 0.5, ..space(w, h) };
    let out = halftone(p, &image(2 * w, 2 * h, |_, _| grey(128)), 2 * w, 2 * h, &fine);
    let column: Vec<u8> = (0..2 * h).map(|y| at(&out, 2 * w, 0, y)[0]).collect();
    assert_eq!(period(&column), 8, "{column:?}");
}

/// Circle draws rings round the object's centre, Line rows that don't change along them, and
/// Dot neither.
#[test]
fn halftone_pattern_type_changes_the_pattern() {
    let (w, h) = (64, 64);
    let src = image(w, h, |_, _| grey(100));
    // The centre on a pixel's centre: pixel (32 + dx, 32 + dy) lies at (dx, dy) from it.
    let s = PixelSpace { center: Point::new(32.5, 32.5), ..space(w, h) };
    let draw = |kind: &str| halftone(json!({"size": 8, "contrast": 50, "patternType": kind}), &src, w, h, &s);
    let (circle, dot, line) = (draw("circle"), draw("dot"), draw("line"));
    assert!(circle != dot && dot != line && line != circle);
    // Pixels at the same distance from the centre in different directions.
    let same_radius: [&[(i64, i64)]; 6] = [
        &[(5, 0), (3, 4), (-4, 3), (0, -5), (-3, -4)],
        &[(10, 0), (6, 8), (-8, -6), (0, 10)],
        &[(13, 0), (5, 12), (-12, 5), (0, -13)],
        &[(15, 0), (9, 12), (-12, 9)],
        &[(17, 0), (8, 15), (-15, -8)],
        &[(25, 0), (7, 24), (-15, 20), (-20, -15), (24, -7)],
    ];
    let value = |d: &[u8], (dx, dy): (i64, i64)| at(d, w, (32 + dx) as usize, (32 + dy) as usize)[0];
    let radial = |d: &[u8]| same_radius.iter().all(|ring| ring.iter().all(|q| value(d, *q) == value(d, ring[0])));
    assert!(radial(&circle), "Circle is the same all round");
    assert!(!radial(&dot) && !radial(&line));
    for y in 0..h {
        assert!((0..w).all(|x| at(&line, w, x, y) == at(&line, w, 0, y)), "row {y} of Line is uniform");
    }
}

#[test]
fn halftone_contrast_sharpens_the_screen() {
    let (w, h) = (64, 32);
    let src = image(w, h, |x, _| grey((x * 4) as u8));
    let mid = |d: &[u8]| share(d, |p| p[0] != 0 && p[0] != 255);
    let soft = halftone(json!({"size": 6, "contrast": 0}), &src, w, h, &space(w, h));
    let hard = halftone(json!({"size": 6, "contrast": 50}), &src, w, h, &space(w, h));
    assert!(mid(&soft) > 0.3, "{}", mid(&soft));
    assert_eq!(mid(&hard), 0.0);
}

/// Halftone Pattern redraws in black ink on white paper, whatever the object's colours, and
/// keeps its coverage.
#[test]
fn halftone_draws_in_black_and_white_only() {
    let (w, h) = (48, 48);
    let src = image(w, h, |x, y| {
        let c = [[230u8, 20, 20], [20, 200, 40], [30, 60, 220], [240, 220, 30]][(x / 12) % 4];
        let a: u16 = if y < 24 { 255 } else { 128 };
        [c[0], c[1], c[2]].map(|v| (u16::from(v) * a / 255) as u8).into_iter().chain([a as u8]).collect::<Vec<_>>().try_into().unwrap()
    });
    for kind in ["circle", "dot", "line"] {
        let out = halftone(json!({"size": 6, "contrast": 50, "patternType": kind}), &src, w, h, &space(w, h));
        for (p, q) in out.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0) {
            assert_eq!(p[3], q[3], "{kind}: coverage kept");
            assert!(p[0] == p[1] && p[1] == p[2] && (p[0] == 0 || p[0] == p[3]), "{kind}: {p:?} from {q:?}");
        }
        assert!(share(&out, |p| p[0] == 0) > 0.1 && share(&out, |p| p[0] == p[3]) > 0.1, "{kind}: ink and paper");
        let soft = halftone(json!({"size": 6, "contrast": 0, "patternType": kind}), &src, w, h, &space(w, h));
        assert!(soft.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]), "{kind}: greys only");
        for (tone, expect) in [(0, 0), (255, 255)] {
            let out = halftone(json!({"size": 6, "contrast": 0, "patternType": kind}), &image(w, h, |_, _| grey(tone)), w, h, &space(w, h));
            assert!(out.as_chunks::<4>().0.iter().all(|p| p[0] == expect), "{kind}: {tone} stays {expect}");
        }
    }
}
