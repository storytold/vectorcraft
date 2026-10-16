//! Effect › Artistic › Smudge Stick, Sponge and Watercolor. Original filters built from public
//! behaviour documentation: <https://helpx.adobe.com/illustrator/using/summary-effects.html> and
//! <https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html> (Smudge Stick smudges the
//! dark areas with short diagonal strokes and brightens the light ones, Sponge paints highly
//! textured areas of contrasting colour, Watercolor simplifies detail with a medium brush loaded
//! with water and saturates the colour where tones change at edges). Visual approximations, not
//! reproductions of a proprietary algorithm; the ranges and defaults are chosen for VectorCraft.
//!
//! All three keep the object's coverage, so they paint nothing outside it. Their patterns lie in
//! document space around the object's centre, as the Brush Strokes' and Texture filters' do.

use super::blur;
use super::brushstrokes::{MARGIN, RIGHT, Smear, TONE_SOFTNESS, edges, mix, ramp, streak, stroke_reach, unpremultiply};
use super::pixelate::Place;
use super::texture::{Tones, luma, premultiply, smooth_noise, straight};
use super::{PixelSpace, Px16, gaussian, to16};

const SALT_SMUDGE: u64 = 320;
const SALT_SPONGE: u64 = 328;
const SALT_WATER: u64 = 336;

/// The length (points) of Smudge Stick's short strokes for Stroke Length `stroke` (0..10); the
/// light areas are smudged with strokes twice as long.
fn smudge_length(stroke: f64) -> f64 {
    1.0 + 1.5 * stroke.clamp(0.0, 10.0)
}

/// How far (document units) Smudge Stick reaches for Stroke Length `stroke`: its strokes and the
/// softened tones that choose them.
pub(super) fn smudge_reach(stroke: f64) -> f64 {
    stroke_reach(2.0 * smudge_length(stroke)).max(3.0 * TONE_SOFTNESS + MARGIN)
}

/// The σ (points) Sponge softens the object by, for Brush Size `brush` (0..10) and Smoothness
/// `smoothness` (1..15).
fn sponge_softness(brush: f64, smoothness: f64) -> f64 {
    (0.15 + 0.04 * smoothness.clamp(1.0, 15.0)) * sponge_pore(brush)
}

/// The size (points) of Sponge's pores for Brush Size `brush` (0..10).
fn sponge_pore(brush: f64) -> f64 {
    2.0 + 1.5 * brush.clamp(0.0, 10.0)
}

/// How far (document units) Sponge reaches: the softened colours around a pixel.
pub(super) fn sponge_reach(brush: f64, smoothness: f64) -> f64 {
    3.0 * sponge_softness(brush, smoothness) + MARGIN
}

/// The radius (points) of the brush that simplifies detail for Brush Detail `detail` (1..14): the
/// less detail, the wider the brush.
fn water_brush(detail: f64) -> f64 {
    0.5 * (15.0 - detail.clamp(1.0, 14.0))
}

/// The σ (points) of the softening that finds Watercolor's edges.
const WATER_EDGE: f64 = 0.6;

/// How far (document units) Watercolor reaches for Brush Detail `detail`.
pub(super) fn watercolor_reach(detail: f64) -> f64 {
    water_brush(detail) + 3.0 * WATER_EDGE + MARGIN
}

/// Smudge Stick: the dark areas smudged along short diagonal strokes `stroke` (0..10) long, the
/// light areas smudged with longer strokes and lightened towards white as strongly as `intensity`
/// (0..10) says, so they lose their detail; `highlight` (0..20) widens the area counted as light.
pub(super) fn smudge_stick(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, stroke: f64, highlight: f64, intensity: f64) {
    if ![stroke, highlight, intensity].iter().all(|v| v.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let len = smudge_length(stroke);
    let (short, long) = (Smear::new(px, w, h, &place, len), Smear::new(px, w, h, &place, 2.0 * len));
    let tones = Tones::new(px, w, h, TONE_SOFTNESS / place.px);
    // Tones above the threshold count as light; a wider highlight area lowers it.
    let threshold = 0.9 - 0.5 * (highlight / 20.0).clamp(0.0, 1.0);
    let lift = (0.7 * intensity / 10.0).clamp(0.0, 0.7) as f32;
    let row = (0.25 * len).max(1.0);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let own = straight(*p);
        let paint = |smear: &Smear, salt: u64| {
            let (c, a) = unpremultiply(smear.stroke(salt, q, RIGHT, row));
            if a > 0.0 { c } else { own }
        };
        // Bristle marks along the dark strokes.
        let bristle = (1.0 + 0.08 * streak(SALT_SMUDGE + 2, q, RIGHT, len, 0.5)) as f32;
        let dark = paint(&short, SALT_SMUDGE).map(|v| v * bristle);
        let light = paint(&long, SALT_SMUDGE + 1);
        let t = ramp((tones.get(x, y) - threshold) / 0.2 + 0.5) as f32;
        let bright = light.map(|v| v + (1.0 - v) * lift);
        *p = premultiply(mix(dark, bright, t), f32::from(p[3]) / 255.0);
    });
}

/// Sponge: the object as if dabbed with a sponge, soft blobs of lighter and darker colour about
/// `brush` (0..10) large; `definition` (0..25) sets how strongly they contrast and `smoothness`
/// (1..15) how soft their edges are.
pub(super) fn sponge(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, brush: f64, definition: f64, smoothness: f64) {
    if ![brush, definition, smoothness].iter().all(|v| v.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let mut soft: Vec<Px16> = to16(px);
    gaussian(&mut soft, w, sponge_softness(brush, smoothness) / place.px);
    let pore = sponge_pore(brush);
    let edge = 0.12 + 0.03 * smoothness.clamp(1.0, 15.0);
    let amount = (definition / 25.0).clamp(0.0, 1.0);
    let gain = 1.0 + 1.2 * amount;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let alpha = f32::from(p[3]) / 255.0;
        let own = straight(*p);
        let (c, a) = unpremultiply(soft.get(y * w + x).map_or([0.0; 4], |s| s.map(|v| f32::from(v) / 65535.0)));
        let c = if a > 0.0 { c } else { own };
        let q = place.at(x, y);
        let n = 0.65 * smooth_noise(SALT_SPONGE, q, pore) + 0.35 * smooth_noise(SALT_SPONGE + 1, q, 0.45 * pore);
        // -1 inside a dark pore, 1 on a light bridge between pores.
        let blob = 2.0 * ramp(0.5 + n / (2.0 * edge)) - 1.0;
        let l = f64::from(luma(c));
        let target = (0.5 + (l + 0.55 * amount * blob - 0.5) * gain).clamp(0.0, 1.0);
        // Darker towards black, lighter towards white: the hue stays.
        let out = if target <= l {
            let k = if l > 1e-3 { (target / l) as f32 } else { 0.0 };
            c.map(|v| v * k)
        } else {
            let k = ((target - l) / (1.0 - l).max(1e-3)).clamp(0.0, 1.0) as f32;
            c.map(|v| v + (1.0 - v) * k)
        };
        *p = premultiply(out, alpha);
    });
}

/// Watercolor: the object's detail simplified by a brush whose size `detail` (1..14) sets (the
/// larger, the more detail stays), the colour saturated and its pigment pooled where tones change
/// at edges, the shadows deepened as strongly as `shadow` (0..10) says, and `texture` (1..3)
/// grains of paper showing through.
pub(super) fn watercolor(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, detail: f64, shadow: f64, texture: u32) {
    if ![detail, shadow].iter().all(|v| v.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let coverage: Vec<u8> = px.iter().map(|p| p[3]).collect();
    // Simplify: average within areas of similar colour, keeping the bolder edges; a coarser brush
    // (less detail) also merges colours further apart.
    blur::smart(px, w, h, water_brush(detail) / place.px, 20.0 + 6.0 * (14.0 - detail.clamp(1.0, 14.0)), 7);
    let found = edges(px, w, h, &place, WATER_EDGE);
    let shadow = shadow.clamp(0.0, 10.0);
    let paper = 0.02 * f64::from(texture.clamp(1, 3));
    let grain = 1.0 + 0.5 * f64::from(texture.clamp(1, 3));
    place.each(px, |x, y, p| {
        let alpha = coverage.get(y * w + x).copied().unwrap_or(0);
        if alpha == 0 {
            *p = [0; 4];
            return;
        }
        let c = straight(*p);
        let e = (2.0 * f64::from(found.get(y * w + x).copied().unwrap_or(0.0))).clamp(0.0, 1.0) as f32;
        let l = luma(c);
        // Saturated colour and darker pigment where the tones change.
        let c = c.map(|v| (l + (v - l) * (1.0 + 0.9 * e)) * (1.0 - 0.12 * e));
        let deep = ((1.0 - l / 0.5).clamp(0.0, 1.0) * 0.07 * shadow as f32).min(0.8);
        let q = place.at(x, y);
        let n = 0.6 * smooth_noise(SALT_WATER, q, grain) + 0.4 * smooth_noise(SALT_WATER + 1, q, 0.5 * grain);
        let n = (paper * n) as f32;
        *p = premultiply(c.map(|v| v * (1.0 - deep) + n), f32::from(alpha) / 255.0);
    });
}
