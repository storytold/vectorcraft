//! Effect › Sketch (Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Halftone Pattern, Note Paper, Photocopy,
//! Plaster, Reticulation, Stamp, Torn Edges and Water Paper; Graphic Pen lives in
//! [`super::brushstrokes`]). Original filters built from public behaviour documentation:
//! <https://helpx.adobe.com/illustrator/using/summary-effects.html> (Sketch effects redraw in
//! black and white; Water Paper flows colours on damp fibrous paper; Conté Crayon draws dark areas
//! in black and light areas in white) and the public Photoshop Elements Sketch filters guide
//! (<https://helpx.adobe.com/photoshop-elements/using/sketch-filters.html>: Conté Crayon's
//! foreground and background levels and texture options). Visual approximations, not proprietary
//! reproductions.

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

use vectorcraft_geom::{Point, Vec2};

use super::distort::displace;
use super::edges::sobel;
use super::pixelate::{Place, cell, rand, value_noise};
use super::texture::{Light, Texture, Tones, luma, premultiply, smooth_noise, straight, texturizer};
use super::{PixelSpace, blur_plane, pick};

const SALT_CHALK: u64 = 216;
const SALT_CHROME: u64 = 0xC40_4D45;
const SALT_CONTE: u64 = 0xC047E;
const SALT_TORN: u64 = 0x746f726e5f6564;
const SALT_WATER: u64 = 208;
const EDGE_SOFT: f64 = 0.8;
const TONE_SOFT: f64 = 1.0;
/// Light from the top left (document axes, y down), as on polished relief surfaces.
const CHROME_LIGHT: Vec2 = Vec2::new(-FRAC_1_SQRT_2, -FRAC_1_SQRT_2);

/// How far (document units) Chalk & Charcoal samples softened tones and edges.
pub(super) const CHALK_CHARCOAL_REACH: f64 = 3.0 * TONE_SOFT + 1.0 + 3.0 * EDGE_SOFT;

fn ramp(t: f64) -> f64 {
    t.clamp(0.0, 1.0)
}

fn tone_edges(px: &[[u8; 4]], w: usize, h: usize, place: &Place, sigma: f64) -> Vec<f32> {
    let mut tone: Vec<f32> = px.iter().map(|p| luma(straight(*p)) * f32::from(p[3]) / 255.0).collect();
    let mut cover: Vec<f32> = px.iter().map(|p| f32::from(p[3]) / 255.0).collect();
    let s = sigma / place.px;
    blur_plane(&mut tone, w, h, s);
    blur_plane(&mut cover, w, h, s);
    let norm = (s.max(0.8) * TAU.sqrt() / 8.0) as f32;
    let norm = if norm.is_finite() { norm } else { 0.0 };
    sobel(&tone, w, h).iter().zip(sobel(&cover, w, h)).map(|(t, c)| t.max(c) * norm).collect()
}

/// Halftone Pattern's screens, in its Pattern Type menu's order: (label, parameter value).
pub const HALFTONE_TYPES: [(&str, &str); 3] = [("Circle", "circle"), ("Dot", "dot"), ("Line", "line")];

/// A Halftone Pattern screen ([`HALFTONE_TYPES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HalftonePattern {
    /// Rings round the object's centre.
    Circle,
    /// Dots on a square grid.
    Dot,
    /// Horizontal lines.
    Line,
}

impl HalftonePattern {
    const ALL: [HalftonePattern; 3] = [HalftonePattern::Circle, HalftonePattern::Dot, HalftonePattern::Line];

    /// The screen of Pattern Type `value` (a [`HALFTONE_TYPES`] value, any case); Dot otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&HALFTONE_TYPES, &Self::ALL, value, HalftonePattern::Dot)
    }
}

/// Halftone Pattern: the object redrawn in black ink on white paper through a `kind` screen whose
/// cells are `size` points apart, anchored at the object's centre. The ink covers as much of a
/// cell as the colour is dark; `contrast` (0..50) hardens the screen's soft edges, to pure black
/// and white at 50. Coverage is kept.
pub(super) fn halftone_pattern(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, size: f64, contrast: f64, kind: HalftonePattern) {
    if !(size.is_finite() && size > 0.0 && contrast.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    // The width of the screen's soft edge, in tones: 0 is a hard threshold.
    let soft = (0.5 * (1.0 - contrast / 50.0)).clamp(0.0, 0.5);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let band = |v: f64| (2.0 * ((v / size).rem_euclid(1.0) - 0.5)).abs();
        // Where in its cell the pixel lies, 0 where the ink starts to 1 where it ends.
        let screen = match kind {
            HalftonePattern::Line => band(q.y),
            HalftonePattern::Circle => band(q.x.hypot(q.y)),
            HalftonePattern::Dot => 0.5 * (band(q.x).powi(2) + band(q.y).powi(2)),
        };
        let dark = f64::from(1.0 - luma(straight(*p))).clamp(0.0, 1.0);
        let ink = if dark >= 1.0 {
            1.0
        } else if soft > 0.0 {
            // The threshold kept inside [soft / 2, 1 - soft / 2], so white stays white and black black.
            let threshold = 0.5 * soft + screen * (1.0 - soft);
            ((dark - threshold) / soft + 0.5).clamp(0.0, 1.0)
        } else if dark > screen {
            1.0
        } else {
            0.0
        };
        *p = premultiply([(1.0 - ink) as f32; 3], f32::from(p[3]) / 255.0);
    });
}

/// Note Paper: the object embossed on fibrous paper. `balance` (0..50) shifts tones toward
/// highlights or shadows around the middle; `graininess` (0..10) adds paper fibre; `relief`
/// (0..50, points) is how strongly the tones stand out in the light. Coverage is kept.
pub(super) fn note_paper(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, balance: f64, graininess: f64, relief: f64) {
    if !(balance.is_finite() && graininess.is_finite() && relief.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let height: Vec<f32> = px
        .iter()
        .map(|p| {
            if p[3] == 0 {
                0.0
            } else {
                let mut l = f64::from(luma(straight(*p)));
                l += (balance - 25.0) / 125.0;
                l.clamp(0.0, 1.0) as f32
            }
        })
        .collect();
    let sample = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return 0.0;
        }
        height.get(y as usize * w + x as usize).copied().unwrap_or(0.0)
    };
    let emboss = relief / 50.0;
    let grain = graininess / 10.0;
    let light = (-0.6_f64, -0.6, 0.8);
    let ln = (light.0 * light.0 + light.1 * light.1 + light.2 * light.2).sqrt().max(1e-9);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let xi = x as i32;
        let yi = y as i32;
        let h0 = height.get(y * w + x).copied().unwrap_or(0.0);
        let dx = (sample(xi + 1, yi) - sample(xi - 1, yi)) as f64;
        let dy = (sample(xi, yi + 1) - sample(xi, yi - 1)) as f64;
        let dz = emboss * 0.12;
        let nx = -dx;
        let ny = -dy;
        let nz = dz;
        let nn = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-9);
        let shade = ((nx * light.0 + ny * light.1 + nz * light.2) / (nn * ln) * 0.5 + 0.5).clamp(0.0, 1.0);
        let tone = (0.35 + 0.65 * f64::from(h0)) * shade;
        let q = place.at(x, y);
        let fibre = if grain > 0.0 { value_noise(0x4e4f_5450, q.x * 0.35, q.y * 0.35) * grain * 0.18 } else { 0.0 };
        let v = (tone + fibre).clamp(0.0, 1.0) as f32;
        *p = premultiply([v; 3], f32::from(p[3]) / 255.0);
    });
}

/// Chalk & Charcoal: light areas get chalky highlights and dark areas charcoal strokes and grain;
/// `chalk` and `charcoal` (0..20) set how strongly, `pressure` (0..5) the stroke weight. Black
/// chalk and charcoal on white paper, coverage kept.
pub(super) fn chalk_charcoal(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, chalk: f64, charcoal: f64, pressure: f64) {
    if !(chalk.is_finite() && charcoal.is_finite() && pressure.is_finite()) {
        return;
    }
    let scale = (pressure.clamp(0.0, 5.0) / 5.0).clamp(0.0, 1.0);
    let chalk_amt = (0.05 * chalk.clamp(0.0, 20.0) * scale).clamp(0.0, 1.0);
    let charcoal_amt = (0.05 * charcoal.clamp(0.0, 20.0) * scale).clamp(0.0, 1.0);
    if chalk_amt <= 0.0 && charcoal_amt <= 0.0 {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let tones = Tones::new(px, w, h, TONE_SOFT / place.px);
    let edges = tone_edges(px, w, h, &place, EDGE_SOFT);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let tone = tones.get(x, y);
        let dark = (1.0 - tone).clamp(0.0, 1.0);
        let light = tone.clamp(0.0, 1.0);
        let edge = f64::from(edges.get(y * w + x).copied().unwrap_or(0.0));
        let charcoal_stroke = value_noise(SALT_CHALK + 1, q.x * 1.1, q.y * 0.12);
        let charcoal_grain = value_noise(SALT_CHALK, q.x * 0.35, q.y * 0.35);
        let charcoal_mask = ramp((dark - 0.3) / 0.22)
            * charcoal_amt
            * (0.35 + 0.65 * edge)
            * (0.45 + 0.55 * ramp(charcoal_stroke * 2.2 - 0.55))
            * (0.6 + 0.4 * charcoal_grain);
        let chalk_grain = value_noise(SALT_CHALK + 2, q.x * 0.45 + 7.0, q.y * 0.45);
        let chalk_mask = ramp((light - 0.5) / 0.22) * chalk_amt * (0.55 + 0.45 * chalk_grain);
        let down = (0.9 * charcoal_mask).clamp(0.0, 1.0) as f32;
        let up = (0.8 * chalk_mask).clamp(0.0, 1.0) as f32;
        let mut grey = luma(straight(*p));
        grey *= 1.0 - down;
        grey += (1.0 - grey) * up;
        *p = premultiply([grey, grey, grey], f32::from(p[3]) / 255.0);
    });
}

/// How far (document units) Chrome reaches for `smoothness` (0..10): 3 σ of the height-field blur
/// plus the Sobel neighbours.
pub(super) fn chrome_reach(smoothness: f64) -> f64 {
    let smoothness = smoothness.clamp(0.0, 10.0);
    3.0 * (0.4 + 0.55 * smoothness) + 1.0
}

/// Chrome: the object as a polished chrome surface lit from the top left. `detail` (0..10) adds
/// finer relief; `smoothness` (0..10) softens the height field before the highlights and shadows
/// are drawn. Coverage is kept.
pub(super) fn chrome(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, detail: f64, smoothness: f64) {
    if w == 0 || h == 0 {
        return;
    }
    let detail = detail.clamp(0.0, 10.0);
    let smoothness = smoothness.clamp(0.0, 10.0);
    let Some(place) = Place::new(space, w, h) else { return };
    let sigma = (0.4 + smoothness * 0.55) / place.px.max(1e-9);
    let cell = (14.0 - detail * 1.2).max(1.5);
    let bump = (detail / 10.0) * 0.4;
    let strength = 2.5 + detail * 0.35;
    let mut height: Vec<f32> = Vec::with_capacity(w * h);
    let mut alpha: Vec<f32> = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let p = px[y * w + x];
            let a = f32::from(p[3]) / 255.0;
            alpha.push(a);
            if a <= 0.0 {
                height.push(0.0);
                continue;
            }
            let q = place.at(x, y);
            let base = f64::from(luma(straight(p)));
            let n = if bump > 0.0 { bump * smooth_noise(SALT_CHROME, q, cell) } else { 0.0 };
            height.push((base + n).clamp(0.0, 1.0) as f32);
        }
    }
    blur_plane(&mut height, w, h, sigma);
    blur_plane(&mut alpha, w, h, sigma);
    let tone: Vec<f64> = height.iter().zip(&alpha).map(|(h, a)| if *a > 1e-4 { f64::from(*h / *a).clamp(0.0, 1.0) } else { 0.0 }).collect();
    let px_unit = place.px.max(1e-9);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let at = |dx: isize, dy: isize| {
            let (xx, yy) = ((x as isize + dx).clamp(0, w as isize - 1) as usize, (y as isize + dy).clamp(0, h as isize - 1) as usize);
            tone[yy * w + xx]
        };
        let h0 = tone[y * w + x];
        let gx = 0.5 * (at(1, 0) - at(-1, 0)) / px_unit;
        let gy = 0.5 * (at(0, 1) - at(0, -1)) / px_unit;
        let face = -(gx * CHROME_LIGHT.x + gy * CHROME_LIGHT.y);
        let spec = (face * strength + (h0 - 0.5) * 1.5).tanh();
        let grey = (0.5 + 0.5 * spec).clamp(0.0, 1.0);
        *p = premultiply([grey as f32; 3], f32::from(p[3]) / 255.0);
    });
}

/// Colour `c` lit by `s`: darkened towards black below 0, lightened towards white above.
fn lit(c: [f32; 3], s: f64) -> [f32; 3] {
    let s = if s.is_finite() { s.clamp(-1.0, 1.0) as f32 } else { 0.0 };
    c.map(|v| if s < 0.0 { v * (1.0 + s) } else { v + (1.0 - v) * s })
}

/// How a surface whose height rises along `slope` faces a light from `toward`.
fn facing(slope: Vec2, toward: Vec2) -> f64 {
    -slope.dot(toward)
}

/// The σ (document units) of Plaster's height blur at `smoothness` (1..15).
fn plaster_sigma_pt(smoothness: f64) -> f64 {
    let smoothness = smoothness.clamp(1.0, 15.0);
    0.15 + 0.85 * (smoothness - 1.0) / 14.0
}

/// How far (document units) Plaster's softened relief reaches at `smoothness` (1..15).
pub(super) fn plaster_reach(smoothness: f64) -> f64 {
    if !smoothness.is_finite() {
        return 3.0 * plaster_sigma_pt(8.0) + 0.5;
    }
    3.0 * plaster_sigma_pt(smoothness) + 0.5
}

/// Plaster: the object molded in relief plaster and lit from `light`. Dark areas are raised and
/// light areas recessed; `imageBalance` (1..50) shifts more of the image lower at higher values;
/// `smoothness` (1..15) softens the relief. Coverage is kept.
pub(super) fn plaster(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, image_balance: f64, smoothness: f64, light: Light) {
    if !(image_balance.is_finite() && smoothness.is_finite() && space.px.is_finite() && space.px > 0.0)
        || w == 0
        || h == 0
        || w.checked_mul(h) != Some(px.len())
    {
        return;
    }
    let image_balance = image_balance.clamp(1.0, 50.0);
    let smoothness = smoothness.clamp(1.0, 15.0);
    let bias = (image_balance - 1.0) / 49.0;
    let mut height: Vec<f32> = px
        .iter()
        .map(|p| {
            let a = f32::from(p[3]) / 255.0;
            let dark = (1.0 - luma(straight(*p))) * a;
            (dark - bias as f32 * 0.65).clamp(0.0, 1.0)
        })
        .collect();
    blur_plane(&mut height, w, h, plaster_sigma_pt(smoothness) / space.px);
    let Some(place) = Place::new(space, w, h) else { return };
    let step = (0.35f64).max(0.5 * place.px);
    let strength = 0.55;
    let toward = light.toward();
    let at = |x: usize, y: usize, dx: i32, dy: i32| {
        let (xx, yy) = ((x as i32 + dx).clamp(0, w as i32 - 1) as usize, (y as i32 + dy).clamp(0, h as i32 - 1) as usize);
        height[yy * w + xx]
    };
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let h = height[y * w + x];
        let (left, right, up, down) = (at(x, y, -1, 0), at(x, y, 1, 0), at(x, y, 0, -1), at(x, y, 0, 1));
        let slope = Vec2::new(f64::from(right - left), f64::from(down - up)) / (2.0 * step);
        let ridge = f64::from(h - 0.25 * (left + right + up + down)) / step;
        let shade = strength * (facing(slope, toward) + ridge);
        *p = premultiply(lit(straight(*p), shade), f32::from(p[3]) / 255.0);
    });
}

/// Reticulation: the object redrawn in black ink on white paper through a high-contrast film-grain
/// screen. `density` (0..50) sets how fine the grain is; `foreground` and `background` (0..50 each)
/// pull shadows toward black and highlights toward white before the grain decides ink. Coverage is
/// kept.
pub(super) fn reticulation(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, density: f64, foreground: f64, background: f64) {
    if !(density.is_finite() && foreground.is_finite() && background.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    // Finer grain at higher density: cells from about 11 pt down to 1 pt.
    let cell_size = 1.0 + (50.0 - density.clamp(0.0, 50.0)) * 0.2;
    let fg = foreground.clamp(0.0, 50.0) / 50.0;
    let bg = background.clamp(0.0, 50.0) / 50.0;
    let spread = 0.12 + 0.38 * (density.clamp(0.0, 50.0) / 50.0);
    const SALT: u64 = 0x5245_7469;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let dark = f64::from(1.0 - luma(straight(*p))).clamp(0.0, 1.0);
        let tone = (dark + fg * (1.0 - dark) * 0.65 - bg * dark * 0.65).clamp(0.0, 1.0);
        let coarse = value_noise(SALT, q.x / cell_size, q.y / cell_size);
        let fine = match (cell(q.x), cell(q.y)) {
            (Some(i), Some(j)) => rand(SALT + 1, i, j, 2),
            _ => 0.5,
        };
        let grain = 0.55 * coarse + 0.45 * fine;
        let ink = if tone + (grain - 0.5) * spread > 0.5 { 1.0 } else { 0.0 };
        *p = premultiply([(1.0 - ink) as f32; 3], f32::from(p[3]) / 255.0);
    });
}

/// How far (document units) Conté Crayon samples the softened tones that choose its crayons.
pub(super) const CONTE_REACH: f64 = 3.0 * TONE_SOFT + 1.0;

/// How much of a crayon is laid down where `amount` (0..1: how dark, for the dark crayon, or how
/// light, for the light one) meets its `level` (0..1) and `grain` (0..1, the paper's tooth and the
/// stroke): at level 0 only the deepest tones take the crayon, at level 1 it reaches past the
/// middle tone; the grain breaks its edge.
fn crayon(amount: f64, level: f64, grain: f64) -> f64 {
    ramp((amount - (0.9 - 0.45 * level) + 0.4 * (grain - 0.5)) / 0.12 + 0.5)
}

/// Conté Crayon: the dark areas drawn in a dense black crayon and the light areas in a pure white
/// one, on mid-grey paper whose tooth (`texture` scaled by `scaling` (0.5..2), its heights turned
/// over when `invert`) and diagonal strokes break the crayons. `foreground` and `background`
/// (0..20) set how far the black and the white crayon reach into the midtones; the paper then
/// stands in relief as strong as `relief` (0..50), lit from `light`, as Texturizer draws it.
/// Coverage is kept.
#[allow(clippy::too_many_arguments)] // the effect's seven options
pub(super) fn conte_crayon(
    px: &mut [[u8; 4]],
    w: usize,
    h: usize,
    space: &PixelSpace,
    foreground: f64,
    background: f64,
    texture: &Texture,
    scaling: f64,
    relief: f64,
    light: Light,
    invert: bool,
) {
    if !([foreground, background, scaling, relief].iter().all(|v| v.is_finite()) && scaling > 0.0) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let fg = foreground.clamp(0.0, 20.0) / 20.0;
    let bg = background.clamp(0.0, 20.0) / 20.0;
    let tones = Tones::new(px, w, h, TONE_SOFT / place.px);
    // The tooth: how far the surface stands above its neighbours a quarter point (or half a pixel)
    // away, so that crayon catches on the peaks and leaves the hollows bare.
    let step = (0.25f64).max(0.5 * place.px / scaling);
    let flip = if invert { -1.0 } else { 1.0 };
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let t = Point::new(q.x / scaling, q.y / scaling);
        let tooth = if t.x.is_finite() && t.y.is_finite() {
            let at = |dx: f64, dy: f64| texture.height(Point::new(t.x + dx, t.y + dy));
            let ridge = (at(0.0, 0.0) - 0.25 * (at(-step, 0.0) + at(step, 0.0) + at(0.0, -step) + at(0.0, step))) / step;
            let tooth = 0.5 + 0.5 * (2.0 * flip * ridge).tanh();
            if tooth.is_finite() { tooth } else { 0.5 }
        } else {
            0.5
        };
        // Strokes along the two diagonals, about a point across and several long.
        let (u, v) = ((q.x + q.y) * FRAC_1_SQRT_2, (q.x - q.y) * FRAC_1_SQRT_2);
        let dark_stroke = value_noise(SALT_CONTE, u * 1.2, v * 0.15);
        let light_stroke = value_noise(SALT_CONTE + 1, v * 1.2, u * 0.15);
        let tone = tones.get(x, y);
        let black = crayon(1.0 - tone, fg, 0.55 * tooth + 0.45 * dark_stroke);
        let white = crayon(tone, bg, 0.55 * tooth + 0.45 * light_stroke);
        let paper = 0.4 + 0.2 * tone;
        let grey = ((paper + (1.0 - paper) * white) * (1.0 - black)) as f32;
        *p = premultiply([grey; 3], f32::from(p[3]) / 255.0);
    });
    if relief > 0.0 {
        texturizer(px, w, h, space, texture, scaling, relief, light, invert);
    }
}

/// Torn Edges: the object in high-contrast black and white with ragged edges. `image_balance`
/// (0..50) shifts whether midtones fall to ink or paper; `smoothness` (0..15) softens tones and
/// the fraying; `contrast` (0..20) hardens the split. Coverage is kept.
pub(super) fn torn_edges(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, image_balance: f64, smoothness: f64, contrast: f64) {
    if !(image_balance.is_finite() && smoothness.is_finite() && contrast.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    if w == 0 || h == 0 || w.checked_mul(h) != Some(px.len()) {
        return;
    }
    let mut tone: Vec<f32> = px.iter().map(|p| luma(straight(*p)) * f32::from(p[3]) / 255.0).collect();
    let cover: Vec<f32> = px.iter().map(|p| f32::from(p[3]) / 255.0).collect();
    let sigma = (0.12 * smoothness.clamp(0.0, 15.0)) / place.px.max(1e-9);
    if sigma > 0.0 {
        blur_plane(&mut tone, w, h, sigma);
    }
    let mut edges: Vec<f32> = sobel(&tone, w, h).into_iter().zip(sobel(&cover, w, h)).map(|(t, c)| t.max(c)).collect();
    let edge_soft = (0.08 * smoothness.clamp(0.0, 15.0)) / place.px.max(1e-9);
    if edge_soft > 0.0 {
        blur_plane(&mut edges, w, h, edge_soft);
    }
    let peak = edges.iter().copied().fold(0.0f32, f32::max);
    let balance = image_balance.clamp(0.0, 50.0);
    // Lower balance favours ink (darker result); higher balance favours paper.
    let balance_tone = 0.25 + balance / 50.0 * 0.75;
    let steep = 4.0 + contrast.clamp(0.0, 20.0) * 0.45;
    let hard = contrast.clamp(0.0, 20.0) >= 18.0;
    let fray = 0.25 + 0.05 * smoothness.clamp(0.0, 15.0);
    place.each(px, |x, y, p| {
        let a = f32::from(p[3]) / 255.0;
        if a <= 0.0 {
            *p = [0; 4];
            return;
        }
        let i = y * w + x;
        let t = tone.get(i).copied().unwrap_or(0.0);
        let e = if peak > 1e-6 { edges.get(i).copied().unwrap_or(0.0) / peak } else { 0.0 };
        let noise = smooth_noise(SALT_TORN, place.at(x, y), 2.5 + 0.3 * smoothness) as f32;
        let local = (1.0 - balance_tone) as f32 + e * fray as f32 * noise;
        let mut ink = 1.0 / (1.0 + (steep as f32 * (t - local)).exp());
        if hard {
            ink = if ink > 0.5 { 1.0 } else { 0.0 };
        }
        let paper = 1.0 - ink.clamp(0.0, 1.0);
        *p = premultiply([paper, paper, paper], a);
    });
}

/// How far Water Paper's blotchy displacement can shift content (document units).
pub(super) fn water_paper_shift(fiber_length: f64) -> f64 {
    0.12 * fiber_length.clamp(3.0, 50.0)
}

/// Water Paper: colours flow on damp fibrous paper. `fiber_length` (3..50) sets the scale of the
/// fibers and daubs; `brightness` and `contrast` (0..100) adjust the washed result. Ranges and
/// defaults are documented in [`crate::effect_catalog`] (unverified; helpx lists the controls only).
pub(super) fn water_paper(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, fiber_length: f64, brightness: f64, contrast: f64) {
    if !(fiber_length.is_finite() && brightness.is_finite() && contrast.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let fiber = fiber_length.clamp(3.0, 50.0);
    let bright = (brightness / 100.0).clamp(0.0, 1.0);
    let cont = (contrast / 100.0).clamp(0.0, 1.0);
    let shift = water_paper_shift(fiber);
    let wave = fiber * 0.4;
    displace(px, w, h, &place, shift, |q| {
        let a = 0.65 * smooth_noise(SALT_WATER, q, wave) + 0.35 * smooth_noise(SALT_WATER + 3, q, wave * 1.7);
        let b = 0.65 * smooth_noise(SALT_WATER + 1, q, wave * 1.1) + 0.35 * smooth_noise(SALT_WATER + 4, q, wave * 0.85);
        Vec2::new(a, b) * shift
    });
    let flow = (0.06 * fiber).max(0.35) / place.px.max(1e-9);
    let mut planes: [Vec<f32>; 3] = std::array::from_fn(|ch| px.iter().map(|p| straight(*p)[ch] * f32::from(p[3]) / 255.0).collect());
    for plane in &mut planes {
        blur_plane(plane, w, h, flow);
    }
    for (i, p) in px.iter_mut().enumerate() {
        if p[3] == 0 {
            continue;
        }
        let a = f32::from(p[3]) / 255.0;
        if a <= 0.0 {
            continue;
        }
        let rgb = {
            let mut rgb = [0.0f32; 3];
            let mut ok = true;
            for ch in 0..3 {
                let Some(v) = planes[ch].get(i) else {
                    ok = false;
                    break;
                };
                rgb[ch] = (*v / a).clamp(0.0, 1.0);
            }
            if !ok {
                continue;
            }
            rgb
        };
        *p = premultiply(rgb, a);
    }
    let lift = (bright - 0.5) * 0.35;
    let gain = 0.35 + 0.65 * cont;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let along = smooth_noise(SALT_WATER + 5, q, fiber * 0.25);
        let across = smooth_noise(SALT_WATER + 6, Point::new(q.y, q.x), fiber * 0.22);
        let fiber_tex = (0.5 * (along + across) + 0.5).clamp(0.0, 1.0);
        let paper = (0.82 + 0.18 * fiber_tex) as f32;
        let mut rgb = straight(*p).map(|v| ((v + lift as f32 - 0.5) * gain as f32 + 0.5).clamp(0.0, 1.0));
        rgb = rgb.map(|v| (v * paper).clamp(0.0, 1.0));
        *p = premultiply(rgb, f32::from(p[3]) / 255.0);
    });
}
