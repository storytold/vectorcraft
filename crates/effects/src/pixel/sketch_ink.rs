//! Effect › Sketch › Bas Relief, Charcoal, Photocopy and Stamp. Original filters built from public
//! behaviour documentation: <https://helpx.adobe.com/illustrator/using/summary-effects.html> and
//! <https://helpx.adobe.com/photoshop-elements/using/sketch-filters.html> (the Sketch effects
//! redraw the object in the foreground and background colours, black on white here; Bas Relief
//! carves low relief lit from a direction, Charcoal draws bold edges and sketches the midtones in
//! diagonal strokes, Photocopy copies only the edges of large dark areas and takes the midtones to
//! black or white, Stamp simplifies the image to solid black and white as a rubber stamp would).
//! Visual approximations, not reproductions of a proprietary algorithm; the ranges and defaults
//! are chosen for VectorCraft.
//!
//! All four keep the object's coverage, so they paint nothing outside it. Their sizes are in
//! document points, whatever the raster's resolution.

use std::f64::consts::FRAC_1_SQRT_2;

use super::pixelate::{Place, cell, value_noise};
use super::texture::{Light, Tones, premultiply};
use super::{PixelSpace, blur_plane};

const SALT_STROKES: u64 = 440;

/// A pixel's size (points) at 72 ppi: how far the neighbours of a gradient add to a filter's reach.
const MARGIN: f64 = 1.0;

/// Redraw `p` as ink `ink` (0..1) of black on white paper, keeping its coverage.
fn paint(p: &mut [u8; 4], ink: f64) {
    let v = if ink.is_finite() { (1.0 - ink).clamp(0.0, 1.0) as f32 } else { 1.0 };
    *p = premultiply([v; 3], f32::from(p[3]) / 255.0);
}

/// A smooth step from 0 at `t` ≤ 0 to 1 at `t` ≥ 1 (0 when `t` is not a number).
fn ramp(t: f64) -> f64 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    t * t * (3.0 - 2.0 * t)
}

/// The tone at (`x`, `y`) and its gradient per pixel, from central differences.
fn gradient(tones: &Tones, x: usize, y: usize) -> (f64, f64, f64) {
    let gx = 0.5 * (tones.get(x + 1, y) - tones.get(x.saturating_sub(1), y));
    let gy = 0.5 * (tones.get(x, y + 1) - tones.get(x, y.saturating_sub(1)));
    (tones.get(x, y), gx, gy)
}

/// The σ (points) Bas Relief softens the object by, for Detail `detail` (1..15) and Smoothness
/// `smoothness` (1..15).
fn relief_softness(detail: f64, smoothness: f64) -> f64 {
    0.2 + 0.15 * smoothness.clamp(1.0, 15.0) + 0.1 * (15.0 - detail.clamp(1.0, 15.0))
}

/// How far (document units) Bas Relief reaches: the softened tones and their neighbours.
pub(super) fn relief_reach(detail: f64, smoothness: f64) -> f64 {
    3.0 * relief_softness(detail, smoothness) + MARGIN
}

/// Bas Relief: the object carved in low relief and lit from `light`. Surfaces facing the light turn
/// white, those turned away black, flat areas grey with a hint of the object's own tones; `detail`
/// (1..15) keeps finer features and `smoothness` (1..15) rounds the carving.
pub(super) fn bas_relief(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, detail: f64, smoothness: f64, light: Light) {
    if !(detail.is_finite() && smoothness.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let sigma = relief_softness(detail, smoothness);
    let tones = Tones::new(px, w, h, sigma / place.px);
    let toward = light.toward();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let (tone, slope) = tones.slope(x, y, &place);
        // A step from black to white, softened by σ, rises 0.4 per σ at its steepest.
        let lit = -slope.dot(toward) * sigma;
        paint(p, 1.0 - (0.5 + 0.2 * (tone - 0.5) + 2.5 * lit));
    });
}

/// The σ (points) Charcoal softens the object by to find its edges, for Detail `detail` (0..5).
fn charcoal_softness(detail: f64) -> f64 {
    0.25 + 0.25 * (5.0 - detail.clamp(0.0, 5.0)) + 0.25
}

/// The σ (points) that thickens Charcoal's edges, for Thickness `thickness` (1..7).
fn charcoal_thickening(thickness: f64) -> f64 {
    0.35 * thickness.clamp(1.0, 7.0)
}

/// The distance (points) between the strokes sketching Charcoal's midtones.
fn charcoal_spacing(thickness: f64) -> f64 {
    1.5 + 0.5 * thickness.clamp(1.0, 7.0)
}

/// How far (document units) Charcoal reaches: the edges, their thickening and the strokes' tones.
pub(super) fn charcoal_reach(thickness: f64, detail: f64) -> f64 {
    (3.0 * (charcoal_softness(detail) + charcoal_thickening(thickness)) + 2.0 * MARGIN).max(2.0 * charcoal_spacing(thickness) + MARGIN)
}

/// Charcoal: bold, `thickness` (1..7) wide edges, as fine as `detail` (0..5) says, and the
/// midtones sketched in diagonal strokes; `balance` (0..100) sets how far into the lights the
/// charcoal reaches (50 is even).
pub(super) fn charcoal(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, thickness: f64, detail: f64, balance: f64) {
    if !(thickness.is_finite() && detail.is_finite() && balance.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let sigma = charcoal_softness(detail);
    let spacing = charcoal_spacing(thickness);
    let bias = (balance.clamp(0.0, 100.0) - 50.0) / 100.0 * 0.8;
    // The edges: how much the softened tones change across σ, 1 for a step from black to white.
    let edge_tones = Tones::new(px, w, h, sigma / place.px);
    let mut edge: Vec<f32> = (0..w * h)
        .map(|k| {
            let (_, gx, gy) = gradient(&edge_tones, k % w, k / w);
            let contrast = gx.hypot(gy) / place.px * sigma / 0.4;
            let weakest = 0.35 - 0.2 * bias / 0.4;
            ramp((contrast - weakest) / 0.3) as f32
        })
        .collect();
    blur_plane(&mut edge, w, h, charcoal_thickening(thickness) / place.px);
    // The strokes' tones are softer than the edges', so the strokes don't follow every speck.
    let tones = Tones::new(px, w, h, 0.5 * spacing / place.px);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let bold = edge.get(y * w + x).map_or(0.0, |e| ((f64::from(*e) - 0.3) / 0.3).clamp(0.0, 1.0));
        let dark = 1.0 - tones.get(x, y);
        let share = ((dark + bias - 0.2) / 0.6).clamp(0.0, 1.0);
        let q = place.at(x, y);
        // Across the strokes, which run rising to the right, and along them.
        let (across, along) = ((q.x + q.y) * FRAC_1_SQRT_2 / spacing, (q.x - q.y) * FRAC_1_SQRT_2 / spacing);
        let stroke = match (share > 0.0, cell(across)) {
            (true, Some(line)) => {
                let f = across - line as f64;
                // Each stroke breaks and swells along its length.
                let swell = (0.6 + 0.8 * value_noise(SALT_STROKES, along / 6.0, line as f64)).clamp(0.0, 1.2);
                let reach = (share * swell).min(1.0);
                let soft = 2.0 * place.px / spacing;
                (((reach - (2.0 * f - 1.0).abs()) / soft + 0.5).clamp(0.0, 1.0)) * f64::from(reach > 0.0)
            }
            _ => 0.0,
        };
        paint(p, bold.max(stroke));
    });
}

/// The σ (points) of Photocopy's narrow softening for Detail `detail` (1..24): the less detail,
/// the wider.
fn copy_softness(detail: f64) -> f64 {
    0.15 * (25.0 - detail.clamp(1.0, 24.0))
}

/// How much wider Photocopy's second softening is than its first.
const COPY_WIDER: f64 = 3.0;

/// How far (document units) Photocopy reaches: the wide softening.
pub(super) fn copy_reach(detail: f64) -> f64 {
    3.0 * COPY_WIDER * copy_softness(detail) + MARGIN
}

/// Photocopy: black where a pixel is darker than its surroundings, white elsewhere. Large dark
/// areas copy only around their edges and midtones go to black or white. `detail` (1..24) keeps
/// finer features and `darkness` (1..50) copies more of the dark.
pub(super) fn photocopy(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, detail: f64, darkness: f64) {
    if !(detail.is_finite() && darkness.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let sigma = copy_softness(detail) / place.px;
    let narrow = Tones::new(px, w, h, sigma);
    let wide = Tones::new(px, w, h, COPY_WIDER * sigma);
    // How much darker than its surroundings a pixel must be to print.
    let threshold = 0.02 + 0.10 * (1.0 - (darkness.clamp(1.0, 50.0) - 1.0) / 49.0);
    let soft = 0.02;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        paint(p, ((wide.get(x, y) - narrow.get(x, y) - threshold) / soft + 0.5).clamp(0.0, 1.0));
    });
}

/// The σ (points) Stamp softens the object by for Smoothness `smoothness` (1..50).
fn stamp_softness(smoothness: f64) -> f64 {
    0.1 * smoothness.clamp(1.0, 50.0)
}

/// How far (document units) Stamp reaches: the softened tones and their neighbours.
pub(super) fn stamp_reach(smoothness: f64) -> f64 {
    3.0 * stamp_softness(smoothness) + MARGIN
}

/// Stamp: the object as solid black and white, like a rubber stamp. `balance` (0..50) sets how much
/// of the object prints black and `smoothness` (1..50) rounds the shapes.
pub(super) fn stamp(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, balance: f64, smoothness: f64) {
    if !(balance.is_finite() && smoothness.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let tones = Tones::new(px, w, h, stamp_softness(smoothness) / place.px);
    let threshold = 0.2 + 0.6 * balance.clamp(0.0, 50.0) / 50.0;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let (tone, gx, gy) = gradient(&tones, x, y);
        // A one pixel wide edge, whatever the softening.
        paint(p, ((threshold - tone) / gx.hypot(gy).max(0.01) + 0.5).clamp(0.0, 1.0));
    });
}
