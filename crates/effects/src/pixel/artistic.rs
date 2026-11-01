//! Effect › Artistic (Colored Pencil, Cutout, Dry Brush, Film Grain, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster
//! Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting and Watercolor). Original filters built from
//! public behaviour documentation:
//! <https://helpx.adobe.com/illustrator/using/summary-effects.html> and
//! <https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html>.
//! These are visual approximations, not reproductions of a proprietary algorithm.

use std::f64::consts::TAU;

use super::brushstrokes::{LEFT, RIGHT, Smear, TONE_SOFTNESS, mix, streak, stroke_reach, unit, unpremultiply};
use super::edges::sobel;
use super::pixelate::{Around, Place, Soft, cell, point_in, rand, value_noise};
use super::texture::{self, Light, Texture, Tones, luma, premultiply, smooth_noise, straight, texturizer};
use super::{PixelSpace, blur_plane, pick};
use vectorcraft_geom::{Point, Vec2};

const SALT_DRY: u64 = 264;
const MARGIN: f64 = 1.0;
const SALT_PLASTIC: u64 = 512;

const SALT_PASTEL: u64 = 200;

const SALT_FILM: u64 = 256;
const SALT_DAUB: u64 = 272;

/// Paint Daubs' brush types, in the Brush Type menu's order: (label, parameter value).
/// Illustrator's list: <https://helpx.adobe.com/illustrator/using/filter-effects.html#paint_daubs>.
pub const PAINT_DAUB_BRUSH_TYPES: [(&str, &str); 6] = [
    ("Simple", "simple"),
    ("Light Rough", "lightRough"),
    ("Dark Rough", "darkRough"),
    ("Wide Sharp", "wideSharp"),
    ("Wide Blurry", "wideBlurry"),
    ("Sparkle", "sparkle"),
];

/// A Paint Daubs brush ([`PAINT_DAUB_BRUSH_TYPES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintDaubBrush {
    Simple,
    LightRough,
    DarkRough,
    WideSharp,
    WideBlurry,
    Sparkle,
}

impl PaintDaubBrush {
    const ALL: [PaintDaubBrush; 6] = [
        PaintDaubBrush::Simple,
        PaintDaubBrush::LightRough,
        PaintDaubBrush::DarkRough,
        PaintDaubBrush::WideSharp,
        PaintDaubBrush::WideBlurry,
        PaintDaubBrush::Sparkle,
    ];

    /// The brush of Brush Type `value` (a [`PAINT_DAUB_BRUSH_TYPES`] value, any case); Simple otherwise.
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("wideRough") {
            return PaintDaubBrush::WideSharp;
        }
        if value.eq_ignore_ascii_case("hue") {
            return PaintDaubBrush::WideBlurry;
        }
        pick(&PAINT_DAUB_BRUSH_TYPES, &Self::ALL, value, PaintDaubBrush::Simple)
    }
}

/// How far (document units) a dab reaches past its centre at `size` (1..50).
pub(super) fn paint_daub_radius(size: f64) -> f64 {
    size.clamp(1.0, 50.0) * 0.48
}
const SALT_PENCIL: u64 = 304;
const SALT_FRESCO: u64 = 320;

/// Keep the last cell's four corner values: magnified adjacent pixels often share a cell.
/// The cache changes only cost, never the field, and needs constant storage per raster.
#[derive(Default)]
struct CellNoise {
    last: Option<(i64, i64, [f64; 4])>,
}

impl CellNoise {
    fn at(&mut self, salt: u64, q: Point) -> f64 {
        let (Some(i), Some(j)) = (cell(q.x), cell(q.y)) else { return 0.0 };
        let corners = match self.last {
            Some((x, y, values)) if x == i && y == j => values,
            _ => {
                let at = |di: i64, dj: i64| 2.0 * rand(salt, i.wrapping_add(di), j.wrapping_add(dj), 0) - 1.0;
                let values = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                self.last = Some((i, j, values));
                values
            }
        };
        let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
        let (fx, fy) = (smooth(q.x - i as f64), smooth(q.y - j as f64));
        let [a, b, c, d] = corners;
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        // Divide by the root sum of squared interpolation weights so corners and cell
        // interiors have the same variance, rather than imprinting a contrast grid.
        let norm = ((fx * fx + (1.0 - fx).powi(2)) * (fy * fy + (1.0 - fy).powi(2))).sqrt();
        (top + (bottom - top) * fy) / norm
    }
}

#[derive(Default)]
struct FilmNoise {
    low: CellNoise,
    high: CellNoise,
}

impl FilmNoise {
    fn at(&mut self, q: Point) -> f32 {
        // Fixed independent rotations (0.6 and -0.35 radians) and a second-octave offset
        // keep the two lattices from lining up with pixel rows or with each other.
        let low = Point::new(0.8253356149096783 * q.x - 0.5646424733950354 * q.y, 0.5646424733950354 * q.x + 0.8253356149096783 * q.y);
        let high = Point::new(
            2.0 * (0.9393727128473789 * q.x + 0.34289780745545134 * q.y) + 0.37,
            2.0 * (-0.34289780745545134 * q.x + 0.9393727128473789 * q.y) - 0.37,
        );
        // Normalising the blend can exceed the corner range, so bound the final field.
        (0.8 * (0.65 * self.low.at(SALT_FILM, low) + 0.35 * self.high.at(SALT_FILM + 1, high))).clamp(-1.0, 1.0) as f32
    }
}

/// Bounded footprint sampling during minification, with attenuation for very large pixels.
/// Up to four samples approximate area filtering, not exact integration of every grain.
fn grain_at(place: &Place, field: &mut FilmNoise, x: usize, y: usize) -> f32 {
    if place.px <= 0.5 {
        return field.at(place.at(x, y));
    }
    let mut sample = |dx, dy| field.at(place.to_rel * Point::new(x as f64 + dx, y as f64 + dy));
    let diagonal = 0.5 * (sample(0.25, 0.25) + sample(0.75, 0.75));
    // Blend sampling modes rather than changing the texture's strength suddenly at a zoom.
    if place.px < 0.75 {
        let t = ((place.px - 0.5) / 0.25) as f32;
        return field.at(place.at(x, y)) * (1.0 - t) + diagonal * t;
    }
    if place.px <= 1.0 {
        return diagonal;
    }
    let other = 0.5 * (sample(0.75, 0.25) + sample(0.25, 0.75));
    let t = (0.5 * ((place.px - 1.0) / 0.5).min(1.0)) as f32;
    // The four samples already average about a two-point footprint. Only wider pixels need
    // extra attenuation; /px alone suppressed coarse grain twice (checked against dense sampling).
    (diagonal * (1.0 - t) + other * t) / (place.px / 2.0).max(1.0) as f32
}

/// Film Grain: monochrome grain in shadows and midtones, with brighter highlights as
/// `highlight` (0..20) widens their area. `intensity` (0..10) increases the highlight lift and
/// quiets highlight grain, including when the Highlight Area is zero;
/// `grain` (0..20) sets the noise strength. The smooth grain field lies relative to the object's
/// bounds' centre, following translations and redraws; changing the bounds shifts the pattern.
/// Its scale is in document units; minification uses bounded approximate footprint filtering.
/// Coverage is preserved, including fully transparent pixels. Constant work and no allocations.
pub(super) fn film_grain(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, grain: f64, highlight: f64, intensity: f64) {
    if ![grain, highlight, intensity].iter().all(|v| v.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let grain = (grain / 20.0).clamp(0.0, 1.0) as f32;
    let highlight = (highlight / 20.0).clamp(0.0, 1.0) as f32;
    let intensity = (intensity / 10.0).clamp(0.0, 1.0) as f32;
    if grain == 0.0 && (intensity == 0.0 || highlight == 0.0) {
        return;
    }
    let mut field = FilmNoise::default();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let c = straight(*p);
        let brightness = luma(c);
        // Wider highlight areas include more midtones, without lifting black or growing a halo.
        let threshold = 0.85 - 0.6 * highlight;
        let t = ((brightness - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
        let light = t * t * (3.0 - 2.0 * t) * highlight * intensity * 0.65;
        // Keep the grain strength even across shadows and midtones, then quiet it smoothly
        // above mid-grey to a quarter-strength floor at full intensity, before highlight
        // headroom. This keeps a softer grain on cream. Black takes light speckles after
        // clipping; white stays clean.
        // This tonal curve is an interpretation of the public description, not a measured
        // proprietary transfer function. Shared offsets stay neutral until a channel clips.
        let upper = ((brightness - 0.5) * 2.0).clamp(0.0, 1.0);
        let upper = upper * upper * (3.0 - 2.0 * upper);
        let amplitude = (0.4 * (1.0 - upper * (0.5 + 0.5 * intensity))).max(0.1).min(1.0 - brightness);
        let strength = amplitude * grain * (1.0 - light);
        // With no highlight lift the starting premultiplied channels are whole bytes. A
        // bounded offset below half a byte cannot change rounding, so avoid hashing it.
        // Leave a small margin for straight/premultiplied floating-point roundoff.
        if light == 0.0 && strength * f32::from(p[3]) < 0.45 {
            return;
        }
        let noise = if strength > 0.0 { grain_at(&place, &mut field, x, y) * strength } else { 0.0 };
        let c = c.map(|v| v + (1.0 - v) * light + noise);
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    });
}

/// The coverage-weighted Gaussian softening of the planes `planes` (premultiplied by `alpha`) by
/// `sigma` pixels: each divided by the softened coverage, so transparency doesn't darken edges.
fn soften(planes: &mut [Vec<f32>], alpha: &[f32], w: usize, h: usize, sigma: f64) {
    let mut a = alpha.to_vec();
    blur_plane(&mut a, w, h, sigma);
    for plane in planes {
        for (v, c) in plane.iter_mut().zip(alpha) {
            *v *= c;
        }
        blur_plane(plane, w, h, sigma);
        for (v, c) in plane.iter_mut().zip(&a) {
            *v = if *c > 1e-4 { (*v / c).clamp(0.0, 1.0) } else { 0.0 };
        }
    }
}

/// The pixels' straight colour channels and their coverage (0..1), as planes.
fn planes(px: &[[u8; 4]]) -> ([Vec<f32>; 3], Vec<f32>) {
    let colours = px.iter().map(|p| straight(*p)).collect::<Vec<_>>();
    (std::array::from_fn(|k| colours.iter().map(|c| c[k]).collect()), px.iter().map(|p| f32::from(p[3]) / 255.0).collect())
}

/// How far (document units) Colored Pencil reaches at pencil width `width`: soften at
/// `0.12 * width` and the Sobel neighbourhood around it.
pub(super) fn colored_pencil_reach(width: f64) -> f64 {
    3.0 * 0.12 * width.clamp(1.0, 24.0) + 1.0
}

/// Colored Pencil: the object redrawn with coloured pencil strokes along local edges, keeping its
/// hues. `width` (1..24) sets the stroke spacing in document units; `pressure` (0..15) darkens
/// along the strokes; `paper` (0..50) lifts lighter areas toward the paper. Coverage is kept.
pub(super) fn colored_pencil(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, width: f64, pressure: f64, paper: f64) {
    if !([width, pressure, paper].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let width = width.clamp(1.0, 24.0);
    let pressure = (pressure / 15.0).clamp(0.0, 1.0) as f32;
    let paper_lift = (paper / 50.0).clamp(0.0, 1.0) as f32;
    if pressure == 0.0 && paper_lift == 0.0 {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let (mut colour, alpha) = planes(px);
    soften(&mut colour, &alpha, w, h, (0.12 * width) / space.px);
    let tones: Vec<f32> = (0..px.len()).map(|k| luma([colour[0][k], colour[1][k], colour[2][k]])).collect();
    let mut edges = vec![0.0f32; px.len()];
    for (e, s) in edges.iter_mut().zip(sobel(&tones, w, h)) {
        *e = (s / 4.0).min(1.0);
    }
    let spacing = (0.35 * width).max(1.0);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let k = y * w + x;
        let c = [colour[0][k], colour[1][k], colour[2][k]];
        let gx = tone_at(&tones, w, h, x as isize + 1, y as isize) - tone_at(&tones, w, h, x as isize - 1, y as isize);
        let gy = tone_at(&tones, w, h, x as isize, y as isize + 1) - tone_at(&tones, w, h, x as isize, y as isize - 1);
        let angle = gy.atan2(gx) + std::f32::consts::FRAC_PI_2;
        let (dx, dy) = (angle.cos() as f64, angle.sin() as f64);
        let q = place.at(x, y);
        let along = q.x * dx + q.y * dy;
        let grain = value_noise(SALT_PENCIL, along / spacing, (q.x * 0.37 + q.y * 0.11) / spacing);
        let stroke = (0.5 + 0.5 * (along / spacing * TAU + grain * 3.0).sin()) as f32;
        let ink = pressure * (0.3 + 0.55 * stroke) * (0.55 + 0.45 * edges[k]);
        let lift = paper_lift * (1.0 - luma(c)) * 0.35;
        let c = c.map(|v| (v * (1.0 - ink * 0.45) + lift * (1.0 - v)).clamp(0.0, 1.0));
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    });
}

/// Cutout: the object as flat pieces of coloured paper. Its colours, softened as `simplicity`
/// (0..10) asks and less as `fidelity` (1..3) keeps detail, fall into `levels` (2..8) steps per
/// colour channel; each step's piece is painted in the average colour of what falls into it, so
/// colours keep their hue. Slivers (the blends along softened edges) take the nearest large
/// piece's colour, so the pieces meet with crisp edges. Coverage is kept.
pub(super) fn cutout(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, levels: f64, simplicity: f64, fidelity: f64) {
    if !([levels, simplicity, fidelity].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let levels = levels.round().clamp(2.0, 8.0) as usize;
    // Simpler edges come from softer colours; more fidelity softens them less.
    let sigma = 0.6 * simplicity.clamp(0.0, 10.0) * (4.0 - fidelity.clamp(1.0, 3.0)) / 3.0 / space.px;
    let (mut colour, alpha) = planes(px);
    soften(&mut colour, &alpha, w, h, sigma);
    let at = |k: usize| [0, 1, 2].map(|i| colour[i].get(k).copied().unwrap_or(0.0));
    let piece = |c: [f32; 3]| c.iter().fold(0, |n, v| n * levels + ((v * levels as f32) as usize).min(levels - 1));
    // Each piece's colour: the coverage-weighted mean of the softened colours in it.
    let mut sums = vec![[0.0f64; 4]; levels * levels * levels];
    for (k, a) in alpha.iter().enumerate() {
        let c = at(k);
        let Some(sum) = sums.get_mut(piece(c)) else { continue };
        for (s, v) in sum.iter_mut().zip(c) {
            *s += f64::from(v * a);
        }
        sum[3] += f64::from(*a);
    }
    let mean = |sum: &[f64; 4]| [0, 1, 2].map(|i| (sum[i] / sum[3]) as f32);
    let total: f64 = sums.iter().map(|s| s[3]).sum();
    let heaviest = sums.iter().map(|s| s[3]).fold(0.0, f64::max);
    // Pieces of at least 1% of the object (and always the largest one) are kept.
    let large: Vec<[f32; 3]> = sums.iter().filter(|s| s[3] > 0.0 && (s[3] >= 0.01 * total || s[3] >= heaviest)).map(mean).collect();
    let paint: Vec<Option<[f32; 3]>> = sums
        .iter()
        .map(|s| {
            let c = (s[3] > 0.0).then(|| mean(s))?;
            let far = |o: &[f32; 3]| o.iter().zip(c).map(|(a, b)| (a - b) * (a - b)).sum::<f32>();
            large.iter().min_by(|a, b| far(a).total_cmp(&far(b))).copied()
        })
        .collect();
    for (k, p) in px.iter_mut().enumerate() {
        if p[3] == 0 {
            continue;
        }
        if let Some(Some(c)) = paint.get(piece(at(k))) {
            *p = premultiply(*c, f32::from(p[3]) / 255.0);
        }
    }
}

/// Poster Edges: the object's colours posterized to `posterization` + 2 (2..8) levels per
/// channel, with black lines drawn on the edges of any of its colour channels and of its outline. `thickness` (0..10) widens the lines and
/// `intensity` (0..10) darkens them and lets fainter edges take a line. Coverage is kept.
pub(super) fn poster_edges(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, thickness: f64, intensity: f64, posterization: f64) {
    if !([thickness, intensity, posterization].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let steps = (posterization.round().clamp(0.0, 6.0) + 1.0) as f32;
    // The edges of each premultiplied channel (so the object's outline too), a step of 1 reading 1.
    let mut edges = vec![0.0f32; px.len()];
    for k in 0..4 {
        let plane: Vec<f32> = px.iter().map(|p| f32::from(p[k]) / 255.0).collect();
        for (e, s) in edges.iter_mut().zip(sobel(&plane, w, h)) {
            *e = e.max(s / 4.0);
        }
    }
    // Wider lines spread the edges; scale them back up so a spread line stays as dark.
    let sigma = (0.3 + 0.25 * thickness.clamp(0.0, 10.0)) / space.px;
    blur_plane(&mut edges, w, h, sigma);
    let gain = ((0.3 + 0.25 * intensity.clamp(0.0, 10.0)) * (1.0 + 1.5 * sigma.max(0.0))) as f32;
    for (p, e) in px.iter_mut().zip(edges) {
        if p[3] == 0 {
            continue;
        }
        let t = ((e * gain - 0.25) / 0.5).clamp(0.0, 1.0);
        let ink = t * t * (3.0 - 2.0 * t);
        let c = straight(*p).map(|v| (v * steps).round() / steps * (1.0 - ink));
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    }
}

/// Paint Daubs: thick round dabs on a grid `size` (1..50) points apart, each dab painted in the
/// local colour around its centre, with edges as sharp as `sharpness` (0..40) and a profile from
/// `kind`. Coverage is kept.
pub(super) fn paint_daubs(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, size: f64, sharpness: f64, kind: PaintDaubBrush) {
    if !([size, sharpness].iter().all(|v| v.is_finite()) && size > 0.0) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let spacing = size.clamp(1.0, 50.0);
    let brush_r = paint_daub_radius(size);
    let edge = 1.0 + 2.5 * (sharpness.clamp(0.0, 40.0) / 40.0);
    let soft = Soft::new(px, w, (spacing * 0.15) / place.px);
    let dab = |i: i64, j: i64| {
        let at = point_in(SALT_DAUB, i, j);
        let center = Point::new(at.x * spacing, at.y * spacing);
        (center, soft.at(place.index(center)))
    };
    let cover = |q: Point, center: Point, colour: [f32; 4]| -> f32 {
        if colour[3] <= 0.0 {
            return 0.0;
        }
        let (dx, dy) = (q.x - center.x, q.y - center.y);
        let (dx, dy) = match kind {
            PaintDaubBrush::WideSharp | PaintDaubBrush::WideBlurry => (dx * 0.65, dy * 1.35),
            _ => (dx, dy),
        };
        let t = (dx.hypot(dy) / brush_r).clamp(0.0, 1.0);
        let edge_k = match kind {
            PaintDaubBrush::WideSharp => edge * 1.35,
            PaintDaubBrush::WideBlurry => edge * 0.55,
            _ => edge,
        };
        let mut c = (1.0 - t).powf(edge_k) as f32;
        let tone = luma(unpremultiply(colour).0);
        match kind {
            PaintDaubBrush::LightRough => c *= 0.7 + 0.35 * rough_bristle(q, 0),
            PaintDaubBrush::DarkRough => c *= 0.55 + 0.45 * (1.0 - rough_bristle(q, 1)),
            PaintDaubBrush::Sparkle if rough_bristle(center, 2) > 0.92 && tone > 0.55 => c = c.max(0.85),
            PaintDaubBrush::Simple | PaintDaubBrush::WideSharp | PaintDaubBrush::WideBlurry | PaintDaubBrush::Sparkle => {}
        }
        c
    };
    let mut around = Around::new();
    place.each(px, |x, y, out| {
        if out[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let (u, v) = (q.x / spacing, q.y / spacing);
        let (Some(i), Some(j)) = (cell(u), cell(v)) else { return };
        let mut sum = [0.0f32; 3];
        let mut weight = 0.0f32;
        for k in 0..9 {
            let Some((center, colour)) = around.get((i, j), k, dab) else { continue };
            let w = cover(q, center, colour);
            if w <= 0.0 {
                continue;
            }
            let c = unpremultiply(colour).0;
            for (s, v) in sum.iter_mut().zip(c) {
                *s += v * w;
            }
            weight += w;
        }
        if weight <= 1e-6 {
            return;
        }
        let c = sum.map(|v| (v / weight).clamp(0.0, 1.0));
        *out = premultiply(c, f32::from(out[3]) / 255.0);
    });
}

fn rough_bristle(p: Point, k: u64) -> f32 {
    let (Some(i), Some(j)) = (cell(p.x * 3.7), cell(p.y * 3.7)) else { return 0.5 };
    rand(SALT_DAUB + 3, i, j, k) as f32
}

/// Fresco: coarse brush dabs on a plaster wall. `brush_size` (1..10) sets dab spacing,
/// `brush_detail` (0..10) keeps sharper edges when high, and `texture` (1..3) deepens the
/// relief. Coverage is kept.
pub(super) fn fresco(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, brush_size: f64, brush_detail: f64, texture: f64) {
    if !([brush_size, brush_detail, texture].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let brush_size = brush_size.clamp(1.0, 10.0);
    let brush_detail = brush_detail.clamp(0.0, 10.0);
    let texture = texture.round().clamp(1.0, 3.0);
    let side = 2.0 + 1.8 * brush_size;
    let sigma = (0.15 * (10.0 - brush_detail) * brush_size / place.px).max(0.0);
    let soft = Soft::new(px, w, sigma);
    let tex_strength = 0.08 + 0.07 * (texture - 1.0);
    let tex_scale = 3.0 / texture;
    let streak_mix = 0.25 * (1.0 - brush_detail / 10.0);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let (Some(i), Some(j)) = (cell(q.x / side), cell(q.y / side)) else { return };
        let jx = (rand(SALT_FRESCO, i, j, 0) - 0.5) * 0.35 * side;
        let jy = (rand(SALT_FRESCO, i, j, 1) - 0.5) * 0.35 * side;
        let sample = Point::new(q.x + jx, q.y + jy);
        let mut sampled = soft.at(place.index(sample));
        if sampled[3] <= 0.0 {
            sampled = soft.at(place.index(q));
        }
        if sampled[3] <= 0.0 {
            return;
        }
        let [r, g, b, alpha] = sampled;
        let c = [r, g, b].map(|v| v / alpha);
        let angle = TAU * rand(SALT_FRESCO, i, j, 2);
        let (fx, fy) = (q.x - i as f64 * side, q.y - j as f64 * side);
        let along = fx * angle.cos() + fy * angle.sin();
        let streak = (1.0 - streak_mix * (0.5 + 0.5 * (along / side).sin().powi(2))) as f32;
        let bump = (1.0 + tex_strength * smooth_noise(SALT_FRESCO + 3, q, tex_scale * side)) as f32;
        let lit = c.map(|v| (v * streak * bump).clamp(0.0, 1.0));
        *p = premultiply(lit, f32::from(p[3]) / 255.0);
    });
}

/// How far (document units) a Fresco dab can reach from a pixel.
pub(super) fn fresco_reach(brush_size: f64, brush_detail: f64) -> f64 {
    let brush_size = brush_size.clamp(1.0, 10.0);
    let brush_detail = brush_detail.clamp(0.0, 10.0);
    let side = 2.0 + 1.8 * brush_size;
    side + 0.15 * (10.0 - brush_detail) * brush_size
}

/// How far (document units) Plastic Wrap's softened relief is sampled at `smoothness` (1..15).
pub(super) fn plastic_reach(smoothness: f64) -> f64 {
    3.0 * (0.15 + 0.35 * smoothness.clamp(1.0, 15.0))
}

/// Plastic Wrap: the object under shiny plastic. Its tones form a relief; `detail` (0..15) adds
/// finer wrinkles in document space; `smoothness` (1..15) softens the relief; specular highlights
/// as strong as `highlight` (0..20) are screened over it. Coverage is kept.
pub(super) fn plastic_wrap(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, highlight: f64, detail: f64, smoothness: f64) {
    if ![highlight, detail, smoothness].iter().all(|v| v.is_finite()) || !space.px.is_finite() || space.px <= 0.0 {
        return;
    }
    let strength = (highlight / 20.0).clamp(0.0, 1.0) as f32;
    if strength == 0.0 {
        return;
    }
    let detail_amt = (detail / 15.0).clamp(0.0, 1.0) as f32;
    let sigma = (0.15 + 0.35 * smoothness.clamp(1.0, 15.0)) / space.px;
    let Some(place) = Place::new(space, w, h) else { return };
    let len = px.len();
    let mut height = vec![0.0f32; len];
    let alpha: Vec<f32> = px.iter().map(|p| f32::from(p[3]) / 255.0).collect();
    let cell_scale = 8.0 + 24.0 * (1.0 - detail_amt as f64);
    let mut noise = CellNoise::default();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let i = y * w + x;
        let l = luma(straight(*p));
        let q = place.at(x, y);
        let nq = Point::new(q.x / cell_scale, q.y / cell_scale);
        height[i] = l + detail_amt * 0.35 * noise.at(SALT_PLASTIC, nq) as f32;
    });
    for (h, a) in height.iter_mut().zip(&alpha) {
        *h *= *a;
    }
    blur_plane(&mut height, w, h, sigma);
    let mut cov = alpha.clone();
    blur_plane(&mut cov, w, h, sigma);
    for (h, c) in height.iter_mut().zip(&cov) {
        *h = if *c > 1e-4 { *h / *c } else { 0.0 };
    }
    const LIGHT: [f32; 3] = [0.35, -0.25, 0.9];
    let inv = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt().max(1e-6);
    let light = LIGHT.map(|v| v / inv);
    let scale = (8.0 * space.px).max(1e-6) as f32;
    let sample = |x: isize, y: isize| -> f32 {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize {
            return 0.0;
        }
        height[y as usize * w + x as usize]
    };
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let gx = sample(x as isize + 1, y as isize) - sample(x as isize - 1, y as isize);
        let gy = sample(x as isize, y as isize + 1) - sample(x as isize, y as isize - 1);
        let nx = -gx / scale;
        let ny = -gy / scale;
        let nz = 1.0;
        let nlen = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
        let ndotl = (nx / nlen * light[0] + ny / nlen * light[1] + nz / nlen * light[2]).max(0.0);
        let spec = ndotl.powi(12) * strength;
        if spec <= 1e-5 {
            return;
        }
        let c = straight(*p).map(|v| 1.0 - (1.0 - v) * (1.0 - spec));
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    });
}

/// Stroke length (document units) for Dry Brush at Brush Size `size` (1..10).
fn dry_brush_length(size: f64) -> f64 {
    2.0 + 2.8 * (size.clamp(1.0, 10.0) - 1.0)
}

/// How much Dry Brush softens before smearing at Brush Detail `detail` (0..10; 10 keeps the most detail).
fn dry_brush_sigma(detail: f64) -> f64 {
    0.22 * (10.0 - detail.clamp(0.0, 10.0))
}

/// How far (document units) Dry Brush reads from a pixel.
pub(super) fn dry_brush_reach(size: f64, detail: f64) -> f64 {
    stroke_reach(dry_brush_length(size)) + 3.0 * dry_brush_sigma(detail) + MARGIN
}

/// How far (document units) Neon Glow's glow spreads at `size` (-24..24).
pub(super) fn glow_spread(size: f64) -> f64 {
    0.25 * size.abs().min(24.0)
}

/// Underpainting: broad brush strokes (`brush_size`, 0..40) over a relief `texture` surface
/// (`scaling`, `relief`, `light`, `invert`, as Texturizer uses), blended in as strongly as
/// `texture_coverage` (0..40) says. Coverage is kept.
#[allow(clippy::too_many_arguments)]
pub(super) fn underpainting(
    px: &mut [[u8; 4]],
    w: usize,
    h: usize,
    space: &PixelSpace,
    brush_size: f64,
    texture_coverage: f64,
    texture: &Texture,
    scaling: f64,
    relief: f64,
    light: Light,
    invert: bool,
) {
    if !(brush_size.is_finite() && texture_coverage.is_finite() && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let brush_size = brush_size.clamp(0.0, 40.0);
    let coverage = (texture_coverage / 40.0).clamp(0.0, 1.0) as f32;
    let (mut colour, alpha) = planes(px);
    let sigma = (0.1 * brush_size.max(0.25)) / space.px;
    if sigma > 0.0 {
        soften(&mut colour, &alpha, w, h, sigma);
    }
    for (k, p) in px.iter_mut().enumerate() {
        if p[3] == 0 {
            continue;
        }
        let c = [colour[0][k], colour[1][k], colour[2][k]];
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    }
    if coverage <= 0.0 {
        return;
    }
    let softened = px.to_vec();
    texture::texturizer(px, w, h, space, texture, scaling, relief, light, invert);
    if coverage >= 1.0 {
        return;
    }
    let mix = 1.0 - coverage;
    for (k, p) in px.iter_mut().enumerate() {
        if p[3] == 0 {
            continue;
        }
        let a = f32::from(p[3]) / 255.0;
        let tex = straight(*p);
        let soft = straight(softened[k]);
        let c = [
            (soft[0] * mix + tex[0] * coverage).clamp(0.0, 1.0),
            (soft[1] * mix + tex[1] * coverage).clamp(0.0, 1.0),
            (soft[2] * mix + tex[2] * coverage).clamp(0.0, 1.0),
        ];
        *p = premultiply(c, a);
    }
}

/// How far (document units) Rough Pastels reaches with strokes `len` long.
pub(super) fn rough_pastels_reach(len: f64) -> f64 {
    super::brushstrokes::stroke_reach(len).max(super::brushstrokes::TONE_REACH)
}

/// Rough Pastels: soft diagonal pastel strokes `length` (0..40) long with bristle `detail` (1..20),
/// then the shared Texturizer surface (`texture`, `scaling`, `relief`, `light`, `invert`).
/// Coverage is kept.
#[allow(clippy::too_many_arguments)]
pub(super) fn rough_pastels(
    px: &mut [[u8; 4]],
    w: usize,
    h: usize,
    space: &PixelSpace,
    length: f64,
    detail: f64,
    texture: &Texture,
    scaling: f64,
    relief: f64,
    light: Light,
    invert: bool,
) {
    if !([length, detail, scaling, relief].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let length = length.clamp(0.0, 40.0);
    let detail = detail.clamp(1.0, 20.0);
    if length > 0.0 {
        let Some(place) = Place::new(space, w, h) else { return };
        let smear = Smear::new(px, w, h, &place, length.max(1.0));
        let tones = Tones::new(px, w, h, TONE_SOFTNESS / place.px);
        let row = (0.18 * smear.len).max(1.0);
        let stroke_mix = (0.35 + 0.0225 * detail).clamp(0.35, 0.8) as f32;
        let bristle = (0.015 + 0.0035 * detail) as f32;
        let src = px.to_vec();
        place.each(px, |x, y, p| {
            if p[3] == 0 {
                return;
            }
            let q = place.at(x, y);
            let pm = mix(smear.stroke(SALT_PASTEL, q, RIGHT, row), smear.stroke(SALT_PASTEL + 1, q, LEFT, row), 0.5);
            let orig = unit(src.get(y * w + x).copied().unwrap_or([0; 4]));
            let pm = mix(orig, pm, stroke_mix);
            let (mut c, a) = unpremultiply(pm);
            if a <= 0.0 {
                *p = [0; 4];
                return;
            }
            let dir = if tones.get(x, y) > 0.52 { RIGHT } else { LEFT };
            let n = bristle * streak(SALT_PASTEL + 2, q, dir, smear.len.max(1.0), row) as f32;
            c = c.map(|v| (v + n).clamp(0.0, 1.0));
            *p = premultiply(c, orig[3]);
        });
    }
    if relief > 0.0 {
        texturizer(px, w, h, space, texture, scaling, relief, light, invert);
    }
}

/// Neon Glow: the object in dimmed greys, lit by `colour` glowing from its light areas (`size`
/// above 0) or from its dark areas (below 0), spread as far as |`size`| (0..24) says and as
/// strong as `brightness` (0..50). Coverage is kept.
pub(super) fn neon_glow(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, size: f64, brightness: f64, colour: [f32; 3]) {
    if !([size, brightness].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let strength = (brightness / 50.0).clamp(0.0, 1.0) as f32;
    let tones: Vec<f32> = px.iter().map(|p| luma(straight(*p))).collect();
    let alpha: Vec<f32> = px.iter().map(|p| f32::from(p[3]) / 255.0).collect();
    let mut glow = [tones.iter().map(|l| if size < 0.0 { 1.0 - l } else { *l }).collect::<Vec<_>>()];
    soften(&mut glow, &alpha, w, h, glow_spread(size) / space.px);
    let [glow] = glow;
    for ((p, tone), g) in px.iter_mut().zip(tones).zip(glow) {
        if p[3] == 0 {
            continue;
        }
        // The greys dim as the glow brightens, and the glow is screened over them.
        let base = tone * (1.0 - 0.6 * strength);
        let lit = (3.0 * g * strength).clamp(0.0, 1.0);
        let c = colour.map(|v| 1.0 - (1.0 - base) * (1.0 - v.clamp(0.0, 1.0) * lit));
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    }
}

/// The tone at `(x, y)` in `tone`, `w` × `h`, or 0 outside.
fn tone_at(tone: &[f32], w: usize, h: usize, x: isize, y: isize) -> f32 {
    if x < 0 || y < 0 {
        return 0.0;
    }
    let (x, y) = (x as usize, y as usize);
    if x >= w || y >= h {
        return 0.0;
    }
    tone[y * w + x]
}

/// A unit vector along the image gradient's tangent at `(x, y)`, or horizontal when flat.
fn dry_brush_dir(tone: &[f32], w: usize, h: usize, x: usize, y: usize) -> Vec2 {
    let gx = 0.5 * (tone_at(tone, w, h, x as isize + 1, y as isize) - tone_at(tone, w, h, x as isize - 1, y as isize));
    let gy = 0.5 * (tone_at(tone, w, h, x as isize, y as isize + 1) - tone_at(tone, w, h, x as isize, y as isize - 1));
    let v = Vec2::new(-f64::from(gy), f64::from(gx));
    let len = v.hypot();
    if len > 1e-6 { v / len } else { Vec2::new(1.0, 0.0) }
}

/// Premultiplied RGBA (0..1) at document `q`, bilinearly from `src`.
fn sample_pm(src: &[[f32; 4]], w: usize, h: usize, place: &Place, q: Point) -> [f32; 4] {
    let p = place.to_px * q;
    if !(p.x.is_finite() && p.y.is_finite()) {
        return [0.0; 4];
    }
    let (x, y) = (p.x - 0.5, p.y - 0.5);
    let (i, j) = (x.floor() as isize, y.floor() as isize);
    let (fx, fy) = (x - i as f64, y - j as f64);
    let mut acc = [0.0f32; 4];
    for (row, wy) in [(j, 1.0 - fy), (j + 1, fy)] {
        let Some(row) = usize::try_from(row).ok().filter(|r| *r < h) else { continue };
        for (col, wx) in [(i, 1.0 - fx), (i + 1, fx)] {
            let Some(col) = usize::try_from(col).ok().filter(|c| *c < w) else { continue };
            let k = (wx * wy) as f32;
            if let Some(p) = src.get(row * w + col) {
                for (a, v) in acc.iter_mut().zip(p) {
                    *a += *v * k;
                }
            }
        }
    }
    acc
}

/// Dry Brush: the object repainted with short strokes along its form, edges darkened. `size`
/// (1..10) sets the stroke length, `detail` (0..10) how much is softened first (10 keeps the most
/// detail), and `texture` (1..3) how strongly edges and bristle grain contrast. Coverage is kept.
pub(super) fn dry_brush(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, size: f64, detail: f64, texture: f64) {
    if !([size, detail, texture].iter().all(|v| v.is_finite()) && space.px.is_finite() && space.px > 0.0) {
        return;
    }
    let len = dry_brush_length(size);
    let sigma = dry_brush_sigma(detail);
    let texture = texture.clamp(1.0, 3.0);
    let edge_gain = (0.25 + 0.2 * (texture - 1.0)) as f32;
    let bristle = (0.04 + 0.05 * (texture - 1.0)) as f32;
    let (mut colour, alpha) = planes(px);
    if sigma > 0.0 {
        soften(&mut colour, &alpha, w, h, sigma);
    }
    let at = |k: usize| [0, 1, 2].map(|i| colour[i].get(k).copied().unwrap_or(0.0));
    let tone: Vec<f32> = (0..w * h)
        .map(|k| {
            let c = at(k);
            let a = alpha.get(k).copied().unwrap_or(0.0);
            (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) * a
        })
        .collect();
    let edges = sobel(&tone, w, h);
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|k| {
            let c = at(k);
            let a = alpha.get(k).copied().unwrap_or(0.0);
            [c[0] * a, c[1] * a, c[2] * a, a]
        })
        .collect();
    let Some(place) = Place::new(space, w, h) else { return };
    const SAMPLES: usize = 5;
    let mut out = px.to_vec();
    for (k, p) in out.iter_mut().enumerate() {
        if p[3] == 0 {
            continue;
        }
        let (x, y) = (k % w, k / w);
        let q = place.at(x, y);
        let dir = dry_brush_dir(&tone, w, h, x, y);
        let mut acc = [0.0f32; 4];
        for i in 0..SAMPLES {
            let t = (i as f64 / (SAMPLES - 1) as f64 - 0.5) * len;
            let s = sample_pm(&src, w, h, &place, q + dir * t);
            for (a, v) in acc.iter_mut().zip(s) {
                *a += v;
            }
        }
        for a in &mut acc {
            *a /= SAMPLES as f32;
        }
        let a = acc[3].clamp(0.0, 1.0);
        let c = if a > 1e-6 { [0, 1, 2].map(|i| (acc[i] / a).clamp(0.0, 1.0)) } else { straight(*p) };
        let edge = edges[k].min(1.0);
        let dark = (edge_gain * edge).clamp(0.0, 0.75);
        let grain = bristle * (2.0 * value_noise(SALT_DRY, q.x / (5.0 * space.px), q.y / (5.0 * space.px)) - 1.0) as f32;
        let c = c.map(|v| (v * (1.0 - dark) + grain).clamp(0.0, 1.0));
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    }
    px.copy_from_slice(&out);
}

const SALT_KNIFE: u64 = 208;

/// How far Palette Knife reaches with strokes `stroke_size` long and Softness `softness` (0..10).
pub(super) fn palette_knife_reach(stroke_size: f64, softness: f64) -> f64 {
    stroke_reach(stroke_size) + 3.0 * 0.35 * softness.clamp(0.0, 10.0) + 1.0
}

/// Artistic › Palette Knife: the object repainted with short, angled knife strokes that smear its
/// colours; `stroke_size` (1..50) sets their length, `stroke_detail` (1..3) keeps more of the
/// original where it's higher, and `softness` (0..10) softens the strokes. Coverage is kept.
pub(super) fn palette_knife(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, stroke_size: f64, stroke_detail: f64, softness: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(stroke_size.is_finite() && stroke_detail.is_finite() && softness.is_finite()) {
        return;
    }
    let len = stroke_size.clamp(1.0, 50.0);
    let keep = ((stroke_detail.clamp(1.0, 3.0) - 1.0) / 2.0) as f32;
    let smear = Smear::new(px, w, h, &place, len);
    let row = (0.25 * smear.len).max(1.0);
    let cell = len * 0.35;
    let src = px.to_vec();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let angle = TAU * value_noise(SALT_KNIFE, q.x / cell, q.y / cell);
        let dir = Vec2::new(angle.cos(), angle.sin());
        let pm = smear.stroke(SALT_KNIFE + 1, q, dir, row);
        let orig = unit(src.get(y * w + x).copied().unwrap_or([0; 4]));
        let coverage = orig[3];
        let (c, _) = unpremultiply(mix(orig, pm, 1.0 - keep));
        *p = premultiply(c, coverage);
    });
    let sigma = 0.35 * softness.clamp(0.0, 10.0) / place.px;
    if sigma > 0.0 {
        let (mut colour, alpha) = planes(px);
        let cover = alpha.clone();
        soften(&mut colour, &alpha, w, h, sigma);
        for (k, p) in px.iter_mut().enumerate() {
            if cover[k] == 0.0 {
                continue;
            }
            let c = [colour[0][k], colour[1][k], colour[2][k]];
            *p = premultiply(c, cover[k]);
        }
    }
}
