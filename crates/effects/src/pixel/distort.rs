//! Effect › Distort (Diffuse Glow, Glass, Ocean Ripple).
//!
//! Glass and Ocean Ripple move each pixel's content by a shift that a surface decides (the slope
//! of Glass's texture, smooth noise for the ripples), sampling the object bilinearly. The shift is
//! capped, so the object reaches no farther than [`glass_shift`] or [`ripple_shift`] past its
//! bounds. Diffuse Glow brightens the object's highlights towards white and sprinkles it with
//! white grain. Every surface lies in document space around the object's centre, as the Texture
//! filters' do.

use vectorcraft_geom::{Point, Vec2};

use super::pixelate::{Place, value_noise};
use super::texture::{Texture, cell_noise, luma, premultiply, smooth_noise, straight};
use super::{PixelSpace, blur_plane, pick};

const SALT_GLOW: u64 = 120;
const SALT_FROST: u64 = 128;
const SALT_RIPPLE: u64 = 136;

/// Glass's surfaces, in its Texture menu's order: (label, parameter value).
pub const GLASS_TEXTURES: [(&str, &str); 4] = [("Blocks", "blocks"), ("Canvas", "canvas"), ("Frosted", "frosted"), ("Tiny Lens", "tinyLens")];

/// A Glass surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassTexture {
    Blocks,
    Canvas,
    Frosted,
    TinyLens,
}

impl GlassTexture {
    const ALL: [GlassTexture; 4] = [GlassTexture::Blocks, GlassTexture::Canvas, GlassTexture::Frosted, GlassTexture::TinyLens];

    /// The surface of `value` (a [`GLASS_TEXTURES`] value, any case); Frosted otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&GLASS_TEXTURES, &Self::ALL, value, GlassTexture::Frosted)
    }

    /// The surface's height at `p` (points at 100 %), in points: slopes of about 1 at most.
    fn height(self, p: Point) -> f64 {
        match self {
            GlassTexture::Blocks => blocks(p),
            GlassTexture::Canvas => Texture::Canvas.height(p),
            GlassTexture::Frosted => frosted(p),
            GlassTexture::TinyLens => tiny_lens(p),
        }
    }
}

/// Square glass blocks 10 points across, flat on top with 2.5-point bevels.
fn blocks(p: Point) -> f64 {
    const SIZE: f64 = 10.0;
    const BEVEL: f64 = 2.5;
    let to_edge = |v: f64| {
        let t = v.rem_euclid(SIZE);
        t.min(SIZE - t)
    };
    to_edge(p.x).min(to_edge(p.y)).min(BEVEL)
}

/// Frosted glass: two octaves of fine smooth noise (1.2 and 0.5 points).
fn frosted(p: Point) -> f64 {
    0.8 * value_noise(SALT_FROST, p.x / 1.2, p.y / 1.2) + 0.2 * value_noise(SALT_FROST + 1, p.x / 0.5, p.y / 0.5)
}

/// Tiny lenses 4 points across in rows and columns, each a shallow dome.
fn tiny_lens(p: Point) -> f64 {
    const SIZE: f64 = 4.0;
    let r = 0.5 * SIZE;
    let off = |v: f64| v.rem_euclid(SIZE) - r;
    let d2 = off(p.x).powi(2) + off(p.y).powi(2);
    0.5 * r * (1.0 - d2 / (r * r)).max(0.0)
}

/// The longest shift (points) Glass gives at `distortion` (0..20).
pub(super) fn glass_shift(distortion: f64) -> f64 {
    0.5 * distortion.clamp(0.0, 20.0)
}

/// The ripples' size (points) for Ripple Size `size` (1..15).
fn ripple_wave(size: f64) -> f64 {
    2.0 + size.clamp(1.0, 15.0)
}

/// The longest shift (points) Ocean Ripple gives for Ripple Size `size` (1..15) and Ripple
/// Magnitude `magnitude` (0..20).
pub(super) fn ripple_shift(size: f64, magnitude: f64) -> f64 {
    0.03 * magnitude.clamp(0.0, 20.0) * ripple_wave(size)
}

/// How far (points) Diffuse Glow spreads the highlights (the σ of their blur).
pub(super) const GLOW_SPREAD: f64 = 2.0;

/// Glass: the object seen through a `texture` surface scaled by `scaling` (0.5..2), bent by
/// `distortion` (0..20) and smoothed by `smoothness` (1..15); `invert` turns the surface's heights
/// over.
#[allow(clippy::too_many_arguments)] // the effect's five options
pub(super) fn glass(
    px: &mut [[u8; 4]],
    w: usize,
    h: usize,
    space: &PixelSpace,
    distortion: f64,
    smoothness: f64,
    texture: GlassTexture,
    scaling: f64,
    invert: bool,
) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(scaling.is_finite() && scaling > 0.0 && smoothness.is_finite()) {
        return;
    }
    // Slopes over a quarter point of the surface per step of Smoothness, or half a pixel where
    // pixels are larger: a smoother glass bends the image more gently.
    let step = (0.25 * smoothness.clamp(1.0, 15.0)).max(0.5 * place.px / scaling);
    let gain = 0.25 * distortion.clamp(0.0, 20.0) * if invert { -1.0 } else { 1.0 };
    displace(px, w, h, &place, glass_shift(distortion), |q| {
        let t = Point::new(q.x / scaling, q.y / scaling);
        let at = |dx: f64, dy: f64| texture.height(Point::new(t.x + dx, t.y + dy));
        Vec2::new(at(step, 0.0) - at(-step, 0.0), at(0.0, step) - at(0.0, -step)) * (gain / (2.0 * step))
    });
}

/// Ocean Ripple: the object as if under rippling water, ripples about `size` (1..15) points plus
/// two across, shifting it by up to [`ripple_shift`].
pub(super) fn ocean_ripple(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, size: f64, magnitude: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let (wave, most) = (ripple_wave(size), ripple_shift(size, magnitude));
    displace(px, w, h, &place, most, |q| {
        let n = |salt: u64| 0.7 * smooth_noise(salt, q, wave) + 0.3 * smooth_noise(salt + 2, q, 0.5 * wave);
        Vec2::new(n(SALT_RIPPLE), n(SALT_RIPPLE + 1)) * most
    });
}

/// Diffuse Glow: the object as seen through a soft diffusion filter. Its highlights, softened over
/// [`GLOW_SPREAD`], glow white as strongly as `glow` (0..20) says, from the brightness that
/// `clear` (0..20) leaves clear of glow; white grain as dense as `graininess` (0..10) says is
/// sprinkled over it, thicker in the glow.
pub(super) fn diffuse_glow(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, graininess: f64, glow: f64, clear: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let mut light: Vec<f32> = px.iter().map(|p| luma(straight(*p)) * f32::from(p[3]) / 255.0).collect();
    blur_plane(&mut light, w, h, GLOW_SPREAD / place.px);
    let amount = (glow / 20.0).clamp(0.0, 1.0) as f32;
    let from = (0.9 * clear / 20.0).clamp(0.0, 0.9) as f32;
    let grain = (graininess / 10.0).clamp(0.0, 1.0);
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let g = ((light.get(y * w + x).copied().unwrap_or(0.0) - from) / (1.0 - from)).clamp(0.0, 1.0);
        let chance = 0.5 * (cell_noise(SALT_GLOW, place.at(x, y), 1.0, 0) + 1.0);
        let dot = if chance < grain * (0.08 + 0.32 * f64::from(g)) { 0.55 } else { 0.0 };
        let white = (amount * g + dot).min(1.0);
        *p = premultiply(straight(*p).map(|v| v + (1.0 - v) * white), f32::from(p[3]) / 255.0);
    });
}

/// Move the object's content by `shift(q)` (document units; `q` a pixel's centre relative to the
/// object's centre, its length capped at `most`): each pixel takes the colour `shift` away from it,
/// interpolated between pixels, transparent beyond the raster.
pub(super) fn displace(px: &mut [[u8; 4]], w: usize, h: usize, place: &Place, most: f64, shift: impl Fn(Point) -> Vec2) {
    if !(most.is_finite() && most > 0.0) {
        return;
    }
    let src = px.to_vec();
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let d = shift(q);
        if !(d.x.is_finite() && d.y.is_finite()) {
            return;
        }
        let len = d.hypot();
        let d = if len > most { d * (most / len) } else { d };
        let s = place.to_px * (q + d);
        *p = sample(&src, w, h, s.x - 0.5, s.y - 0.5);
    });
}

/// The premultiplied colour at (`x`, `y`) of `w` × `h` pixels (pixel centres at whole
/// coordinates), interpolated between the four around it, transparent outside.
fn sample(src: &[[u8; 4]], w: usize, h: usize, x: f64, y: f64) -> [u8; 4] {
    if !(x.is_finite() && y.is_finite()) {
        return [0; 4];
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |i: f64, j: f64| -> [f64; 4] {
        if i < 0.0 || j < 0.0 || i >= w as f64 || j >= h as f64 {
            return [0.0; 4];
        }
        src.get(j as usize * w + i as usize).map_or([0.0; 4], |p| p.map(f64::from))
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8
    })
}
