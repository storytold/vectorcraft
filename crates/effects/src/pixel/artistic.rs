//! Effect › Artistic (Cutout, Film Grain, Neon Glow, Poster Edges). Original filters built from
//! public behaviour documentation:
//! <https://helpx.adobe.com/illustrator/using/summary-effects.html> and
//! <https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html>.
//! These are visual approximations, not reproductions of a proprietary algorithm.

use super::edges::sobel;
use super::pixelate::{Place, cell, rand};
use super::texture::{luma, premultiply, straight};
use super::{PixelSpace, blur_plane};
use vectorcraft_geom::Point;

const SALT_FILM: u64 = 256;

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

/// How far (document units) Neon Glow's glow spreads at `size` (-24..24).
pub(super) fn glow_spread(size: f64) -> f64 {
    0.25 * size.abs().min(24.0)
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
