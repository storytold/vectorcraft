//! Effect › Brush Strokes (Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines,
//! Spatter, Sprayed Strokes, Sumi-e).
//!
//! The stroke filters repaint the object with brush strokes: each pixel takes the object's colour
//! averaged along a segment through it ([`Smear`], a bounded number of samples whatever the
//! length). Strokes lie in rows across their direction, each row starting at random and each
//! stroke shifted along itself at random, so they begin and end like separate strokes; a streaked
//! noise along them gives them their bristle texture. Accented Edges and Ink Outlines find edges
//! with Glowing Edges' Sobel operator over the softened object; Spatter and Sprayed Strokes shift
//! the content by noise as Ocean Ripple does. Every pattern lies in document space around the
//! object's centre, as the Texture filters' do.

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

use vectorcraft_geom::{Point, Vec2};

use super::distort::displace;
use super::edges::sobel;
use super::pixelate::{Place, cell, rand, value_noise};
use super::texture::{Tones, luma, premultiply, smooth_noise, straight};
use super::{PixelSpace, blur_plane, gaussian, pick, to16};

const SALT_ANGLED: u64 = 144;
const SALT_CROSS: u64 = 152;
const SALT_DARK: u64 = 160;
const SALT_INK: u64 = 168;
const SALT_SPATTER: u64 = 176;
const SALT_SPRAYED: u64 = 184;
const SALT_SUMI: u64 = 192;

/// Sprayed Strokes' stroke directions, in its Stroke Direction menu's order: (label, parameter
/// value).
pub const STROKE_DIRECTIONS: [(&str, &str); 4] =
    [("Right Diagonal", "rightDiagonal"), ("Horizontal", "horizontal"), ("Left Diagonal", "leftDiagonal"), ("Vertical", "vertical")];

/// The way Sprayed Strokes are painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeDirection {
    RightDiagonal,
    Horizontal,
    LeftDiagonal,
    Vertical,
}

impl StrokeDirection {
    const ALL: [StrokeDirection; 4] =
        [StrokeDirection::RightDiagonal, StrokeDirection::Horizontal, StrokeDirection::LeftDiagonal, StrokeDirection::Vertical];

    /// The direction of `value` (a [`STROKE_DIRECTIONS`] value, any case); Right Diagonal
    /// otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&STROKE_DIRECTIONS, &Self::ALL, value, StrokeDirection::RightDiagonal)
    }

    /// The unit vector along the strokes (document axes, y down).
    fn along(self) -> Vec2 {
        match self {
            StrokeDirection::RightDiagonal => RIGHT,
            StrokeDirection::Horizontal => Vec2::new(1.0, 0.0),
            StrokeDirection::LeftDiagonal => LEFT,
            StrokeDirection::Vertical => Vec2::new(0.0, 1.0),
        }
    }
}

/// Along a right diagonal stroke, rising to the right (y down).
const RIGHT: Vec2 = Vec2::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
/// Along a left diagonal stroke, falling to the right.
const LEFT: Vec2 = Vec2::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2);

/// The most samples a stroke averages, whatever its length.
const MAX_SAMPLES: usize = 16;

/// The share of its length by which a stroke is shifted along itself at random, at most.
const JITTER: f64 = 0.25;

/// How soft (points, the σ) the tones are that decide which strokes a pixel gets.
const TONE_SOFTNESS: f64 = 1.0;

/// Dark Strokes' short dark strokes and long light ones (points).
pub(super) const DARK_LENGTH: f64 = 3.0;
pub(super) const LIGHT_LENGTH: f64 = 9.0;

/// How soft (points, the σ) Ink Outlines' outlines are.
const INK_SOFTNESS: f64 = 0.6;

/// A pixel at 72 ppi (points): how far the neighbours of the Sobel operator and of sampling
/// between pixels add to a filter's reach.
const MARGIN: f64 = 1.0;

/// How far (document units) strokes `len` long reach: half their length, their random shift, the
/// softening between their samples and the pixels a sample is taken between.
pub(super) fn stroke_reach(len: f64) -> f64 {
    let len = if len.is_finite() { len.max(0.0) } else { 0.0 };
    (0.5 + JITTER + 1.5 / (MAX_SAMPLES - 1) as f64) * len + MARGIN
}

/// How far (document units) the tones reach that the stroke filters choose their strokes by.
pub(super) const TONE_REACH: f64 = 3.0 * TONE_SOFTNESS + MARGIN;

/// How far Sumi-e reaches with strokes `width` across: the strokes, and the wet ink bleeding from
/// 3 σ of 0.2 × the width.
pub(super) fn sumi_reach(width: f64) -> f64 {
    stroke_reach(width).max(0.6 * width + MARGIN)
}

/// The σ (points) Accented Edges softens the object by before finding its edges, for Edge Width
/// `width` (1..14) and Smoothness `smoothness` (1..15).
pub(super) fn accent_softness(width: f64, smoothness: f64) -> f64 {
    0.35 * width.clamp(1.0, 14.0) + 0.1 * smoothness.clamp(1.0, 15.0)
}

/// How far Accented Edges reaches for Edge Width `width` and Smoothness `smoothness`.
pub(super) fn accent_reach(width: f64, smoothness: f64) -> f64 {
    3.0 * accent_softness(width, smoothness) + MARGIN
}

/// How far Ink Outlines reaches with strokes `len` long.
pub(super) fn ink_reach(len: f64) -> f64 {
    stroke_reach(len).max(3.0 * INK_SOFTNESS + MARGIN)
}

/// How far (points) Crosshatch roughens the edges, with strokes `len` long.
pub(super) fn rough(len: f64) -> f64 {
    0.1 * if len.is_finite() { len.max(0.0) } else { 0.0 }
}

/// The longest shift (points) Spatter gives at Spray Radius `radius` (0..25).
pub(super) fn spatter_shift(radius: f64) -> f64 {
    0.4 * radius.clamp(0.0, 25.0)
}

/// The longest shift (points) Sprayed Strokes' spray gives at Spray Radius `radius` (0..25).
pub(super) fn spray_shift(radius: f64) -> f64 {
    0.3 * radius.clamp(0.0, 25.0)
}

/// The object's colours averaged along strokes `len` long: [`MAX_SAMPLES`] at most, the object
/// softened first when they lie more than a pixel apart.
struct Smear<'a> {
    /// Premultiplied colours (0..1).
    src: Vec<[f32; 4]>,
    w: usize,
    h: usize,
    place: &'a Place,
    len: f64,
    n: usize,
}

impl<'a> Smear<'a> {
    fn new(px: &[[u8; 4]], w: usize, h: usize, place: &'a Place, len: f64) -> Self {
        let len = if len.is_finite() { len.max(0.0) } else { 0.0 };
        let span = len / place.px;
        let n = if span.is_finite() { (span.ceil() as usize).saturating_add(1).clamp(1, MAX_SAMPLES) } else { MAX_SAMPLES };
        let gap = if n > 1 { span / (n - 1) as f64 } else { 0.0 };
        let mut wide = to16(px);
        if gap > 1.0 {
            gaussian(&mut wide, w, 0.5 * gap);
        }
        let src = wide.iter().map(|p| p.map(|v| f32::from(v) / 65535.0)).collect();
        Self { src, w, h, place, len, n }
    }

    /// The premultiplied colour (0..1) averaged along the stroke through `centre` (relative to the
    /// object's centre) along `dir`.
    fn at(&self, centre: Point, dir: Vec2) -> [f32; 4] {
        let half = dir * (0.5 * self.len);
        let (a, b) = (self.place.to_px * (centre - half), self.place.to_px * (centre + half));
        if !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
            return [0.0; 4];
        }
        // Pixel centres at whole coordinates.
        let (mut x, mut y, step) = match self.n {
            0 | 1 => (0.5 * (a.x + b.x) - 0.5, 0.5 * (a.y + b.y) - 0.5, (0.0, 0.0)),
            n => (a.x - 0.5, a.y - 0.5, ((b.x - a.x) / (n - 1) as f64, (b.y - a.y) / (n - 1) as f64)),
        };
        let mut acc = [0.0f32; 4];
        for _ in 0..self.n {
            self.add(&mut acc, x, y);
            x += step.0;
            y += step.1;
        }
        acc.map(|v| v / self.n.max(1) as f32)
    }

    /// Add the colour at (`x`, `y`) (pixel centres at whole coordinates), interpolated between the
    /// four pixels around it, to `acc`; transparent outside.
    fn add(&self, acc: &mut [f32; 4], x: f64, y: f64) {
        let (x0, y0) = (x.floor(), y.floor());
        if !(x0 >= -1.0 && y0 >= -1.0 && x0 < self.w as f64 && y0 < self.h as f64) {
            return;
        }
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (i, j) = (x0 as isize, y0 as isize);
        // Inside the raster: the four pixels as two pairs.
        if i >= 0 && j >= 0 && (i as usize) + 1 < self.w && (j as usize) + 1 < self.h {
            let at = j as usize * self.w + i as usize;
            if let (Some([p00, p10]), Some([p01, p11])) = (self.src.get(at..at + 2), self.src.get(at + self.w..at + self.w + 2)) {
                let (k00, k10, k01, k11) = ((1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy);
                for (c, a) in acc.iter_mut().enumerate() {
                    *a += p00[c] * k00 + p10[c] * k10 + p01[c] * k01 + p11[c] * k11;
                }
            }
            return;
        }
        for (row, wy) in [(j, 1.0 - fy), (j + 1, fy)] {
            let Some(row) = usize::try_from(row).ok().filter(|r| *r < self.h) else { continue };
            for (col, wx) in [(i, 1.0 - fx), (i + 1, fx)] {
                let Some(col) = usize::try_from(col).ok().filter(|c| *c < self.w) else { continue };
                if let Some(p) = self.src.get(row * self.w + col) {
                    let k = wx * wy;
                    for (a, v) in acc.iter_mut().zip(p) {
                        *a += v * k;
                    }
                }
            }
        }
    }

    /// The stroke of pattern `salt` through `q` along `dir`, its rows `width` apart.
    fn stroke(&self, salt: u64, q: Point, dir: Vec2, width: f64) -> [f32; 4] {
        self.at(q + dir * stroke_shift(salt, q, dir, self.len, width), dir)
    }
}

/// `q`'s coordinates along `dir` (unit) and across it.
fn frame(q: Point, dir: Vec2) -> (f64, f64) {
    (q.x * dir.x + q.y * dir.y, q.y * dir.x - q.x * dir.y)
}

/// How far along `dir` the stroke through `q` is shifted: strokes `len` long lie in rows `width`
/// apart across `dir`, each row starting at random, each stroke shifted by up to [`JITTER`] ×
/// `len`.
fn stroke_shift(salt: u64, q: Point, dir: Vec2, len: f64, width: f64) -> f64 {
    if !(len > 0.0 && width > 0.0) {
        return 0.0;
    }
    let (u, v) = frame(q, dir);
    let Some(row) = cell(v / width) else { return 0.0 };
    let Some(col) = cell(u / len + rand(salt, row, 0, 0)) else { return 0.0 };
    (2.0 * rand(salt, col, row, 1) - 1.0) * JITTER * len
}

/// Smooth noise in [-1, 1) streaked along `dir`: blobs `along` long and `across` wide.
fn streak(salt: u64, q: Point, dir: Vec2, along: f64, across: f64) -> f64 {
    let (u, v) = frame(q, dir);
    2.0 * value_noise(salt, u / along.max(1e-3), v / across.max(1e-3)) - 1.0
}

/// The straight colour and coverage of premultiplied `pm` (0..1).
fn unpremultiply(pm: [f32; 4]) -> ([f32; 3], f32) {
    let a = pm[3].clamp(0.0, 1.0);
    if a > 1e-6 { ([pm[0], pm[1], pm[2]].map(|v| (v / a).clamp(0.0, 1.0)), a) } else { ([0.0; 3], 0.0) }
}

/// `a` + (`b` − `a`) × `t`, channel by channel.
fn mix<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// A smooth step from 0 at `t` ≤ 0 to 1 at `t` ≥ 1.
fn ramp(t: f64) -> f64 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    t * t * (3.0 - 2.0 * t)
}

/// Pixel `p` (premultiplied 0..255) as premultiplied 0..1.
fn unit(p: [u8; 4]) -> [f32; 4] {
    p.map(|v| f32::from(v) / 255.0)
}

/// The object's edges after softening it by `sigma` (document units): about 1 across a step from
/// black to white or from clear to opaque, the same at any resolution. Empty when the sizes don't
/// match.
fn edges(px: &[[u8; 4]], w: usize, h: usize, place: &Place, sigma: f64) -> Vec<f32> {
    let mut tone: Vec<f32> = px.iter().map(|p| luma(straight(*p)) * f32::from(p[3]) / 255.0).collect();
    let mut cover: Vec<f32> = px.iter().map(|p| f32::from(p[3]) / 255.0).collect();
    let s = sigma / place.px;
    blur_plane(&mut tone, w, h, s);
    blur_plane(&mut cover, w, h, s);
    // A step softened by σ pixels rises by 1 / (σ √(2π)) per pixel at most, and Sobel gives 8 ×
    // the slope; a pixel is the least it softens by.
    let norm = (s.max(0.8) * TAU.sqrt() / 8.0) as f32;
    let norm = if norm.is_finite() { norm } else { 0.0 };
    sobel(&tone, w, h).iter().zip(sobel(&cover, w, h)).map(|(t, c)| t.max(c) * norm).collect()
}

/// Accented Edges: the object's edges accented in white chalk when `brightness` (0..50) is high,
/// in black ink when it's low, about `width` (1..14) wide; `smoothness` (1..15) softens the object
/// first and leaves out its fainter edges.
pub(super) fn accented_edges(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, width: f64, brightness: f64, smoothness: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let found = edges(px, w, h, &place, accent_softness(width, smoothness));
    let faint = (0.02 * smoothness.clamp(1.0, 15.0)) as f32;
    let level = (brightness / 50.0).clamp(0.0, 1.0) as f32;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let e = found.get(y * w + x).copied().unwrap_or(0.0);
        let e = (2.5 * (e - faint) / (1.0 - faint)).clamp(0.0, 1.0);
        let c = straight(*p);
        // Chalk or ink of the brightness level, a little of the colour showing through.
        let accent = c.map(|v| 0.9 * level + 0.1 * v);
        *p = premultiply(mix(c, accent, e), f32::from(p[3]) / 255.0);
    });
}

/// Angled Strokes: the object repainted in diagonal strokes `len` long, light areas in right
/// diagonal strokes and dark areas in left diagonal ones; `balance` (0..100) moves the split
/// (0 all left diagonal, 100 all right diagonal) and `sharpness` (0..10) brings the detail and
/// the bristles out.
pub(super) fn angled_strokes(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, balance: f64, len: f64, sharpness: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let smear = Smear::new(px, w, h, &place, len);
    let tones = Tones::new(px, w, h, TONE_SOFTNESS / place.px);
    let split = 1.07 - 1.14 * (balance / 100.0).clamp(0.0, 1.0);
    let sharp = sharpness.clamp(0.0, 10.0);
    let (detail, bristle) = ((0.05 * sharp) as f32, 0.03 + 0.012 * sharp);
    let row = (0.2 * smear.len).max(1.0);
    let src = px.to_vec();
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        // The split between the directions wanders along the strokes, so they interlock.
        let right = tones.get(x, y) > split + 0.06 * streak(SALT_ANGLED + 3, q, RIGHT, row, row);
        let (dir, salt) = if right { (RIGHT, SALT_ANGLED) } else { (LEFT, SALT_ANGLED + 1) };
        let pm = smear.stroke(salt, q, dir, row);
        let pm = mix(pm, unit(src.get(y * w + x).copied().unwrap_or([0; 4])), detail);
        let (c, a) = unpremultiply(pm);
        let n = (bristle * streak(SALT_ANGLED + 2, q, dir, 0.5 * smear.len.max(1.0), 0.6)) as f32;
        *p = premultiply(c.map(|v| v + n), a);
    });
}

/// Crosshatch: the object's detail kept under pencil hatching along both diagonals, strokes `len`
/// long, roughening its edges; `sharpness` (0..20) sets how crisp and dark the hatching is,
/// `passes` (1..3) how many layers of it there are.
pub(super) fn crosshatch(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, len: f64, sharpness: f64, passes: u32) {
    let Some(place) = Place::new(space, w, h) else { return };
    let smear = Smear::new(px, w, h, &place, len);
    let tones = Tones::new(px, w, h, TONE_SOFTNESS / place.px);
    let sharp = sharpness.clamp(0.0, 20.0);
    let (amount, crisp) = (0.05 + 0.01 * sharp, 1.0 + 0.3 * sharp);
    let row = (0.2 * smear.len).max(1.0);
    let roughen = rough(smear.len);
    let src = px.to_vec();
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let line = |k: u32| {
            let dir = if k.is_multiple_of(2) { RIGHT } else { LEFT };
            streak(SALT_CROSS + u64::from(k), q, dir, smear.len.max(1.0), 0.7 + 0.3 * f64::from(k))
        };
        let first = line(0);
        // The edges roughened along the first hatching.
        let o = q + RIGHT * (roughen * first);
        let strokes = mix(smear.stroke(SALT_CROSS + 4, o, RIGHT, row), smear.stroke(SALT_CROSS + 5, o, LEFT, row), 0.5);
        let pm = mix(unit(src.get(y * w + x).copied().unwrap_or([0; 4])), strokes, 0.5);
        let (mut c, a) = unpremultiply(pm);
        if a <= 0.0 {
            *p = [0; 4];
            return;
        }
        let shade = 1.3 - tones.get(x, y);
        for k in 0..passes.clamp(1, 3) {
            let v = (if k == 0 { first } else { line(k) } * crisp).clamp(-1.0, 1.0);
            // Dark pencil lines and the light paper between them.
            let s = if v > 0.0 { -(amount * v * shade) } else { -0.6 * amount * v };
            let s = s.clamp(-1.0, 1.0) as f32;
            c = c.map(|v| if s < 0.0 { v * (1.0 + s) } else { v + (1.0 - v) * s });
        }
        *p = premultiply(c, a);
    });
}

/// Dark Strokes: dark areas painted closer to black with short strokes, light areas with long
/// strokes closer to white; `balance` (0..10) moves the tone between them up, `black` (0..10) and
/// `white` (0..10) set how dark and how light they get.
pub(super) fn dark_strokes(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, balance: f64, black: f64, white: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let short = Smear::new(px, w, h, &place, DARK_LENGTH);
    let long = Smear::new(px, w, h, &place, LIGHT_LENGTH);
    let tones = Tones::new(px, w, h, TONE_SOFTNESS / place.px);
    let split = 0.15 + 0.07 * balance.clamp(0.0, 10.0);
    let (black, white) = (0.085 * black.clamp(0.0, 10.0), 0.085 * white.clamp(0.0, 10.0));
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let dark = 1.0 - ramp((tones.get(x, y) - split) / 0.12 + 0.5);
        let painted = |light: bool| {
            let (smear, dir, salt) = if light { (&long, RIGHT, SALT_DARK) } else { (&short, LEFT, SALT_DARK + 1) };
            let (c, a) = unpremultiply(smear.stroke(salt, q, dir, 1.0));
            let n = 0.7 + 0.3 * streak(salt + 2, q, dir, 0.5 * smear.len, 0.5);
            let c = if light {
                let s = (white * n).clamp(0.0, 1.0) as f32;
                c.map(|v| v + (1.0 - v) * s)
            } else {
                let s = (black * n).clamp(0.0, 1.0) as f32;
                c.map(|v| v * (1.0 - s))
            };
            [c[0] * a, c[1] * a, c[2] * a, a]
        };
        let pm = if dark >= 1.0 {
            painted(false)
        } else if dark <= 0.0 {
            painted(true)
        } else {
            mix(painted(true), painted(false), dark as f32)
        };
        let (c, a) = unpremultiply(pm);
        *p = premultiply(c, a);
    });
}

/// Ink Outlines: the object redrawn in pen and ink, fine outlines over its edges and ink strokes
/// `len` long in its shadows; `dark` (0..50) darkens the shadows, `light` (0..50) lightens the
/// highlights.
pub(super) fn ink_outlines(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, len: f64, dark: f64, light: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let smear = Smear::new(px, w, h, &place, len);
    let found = edges(px, w, h, &place, INK_SOFTNESS);
    let (dark, light) = ((dark / 50.0).clamp(0.0, 1.0), (light / 50.0).clamp(0.0, 1.0));
    let row = (0.25 * smear.len).max(0.75);
    let src = px.to_vec();
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let pm = mix(unit(src.get(y * w + x).copied().unwrap_or([0; 4])), smear.stroke(SALT_INK, q, LEFT, row), 0.5);
        let (c, a) = unpremultiply(pm);
        if a <= 0.0 {
            *p = [0; 4];
            return;
        }
        let l = f64::from(luma(c));
        let (shadow, highlight) = (((0.75 - l) / 0.75).clamp(0.0, 1.0), ((l - 0.45) / 0.55).clamp(0.0, 1.0));
        let (down, up) = ((0.7 * dark * shadow) as f32, (0.7 * light * highlight) as f32);
        let c = c.map(|v| v * (1.0 - down)).map(|v| v + (1.0 - v) * up);
        // Ink lines along the strokes in the shadows, and the outlines.
        let hatch = ramp(2.0 * streak(SALT_INK + 1, q, LEFT, 1.5 * smear.len.max(1.0), 0.5) + 0.2) * shadow * (0.4 + 1.2 * dark);
        let outline = (1.5 * f64::from(found.get(y * w + x).copied().unwrap_or(0.0)) - 0.15).clamp(0.0, 1.0);
        let ink = (0.9 * outline).max(0.8 * hatch).min(1.0) as f32;
        *p = premultiply(c.map(|v| v * (1.0 - ink)), a);
    });
}

/// Spatter: the object as if sprayed with a spatter airbrush, each pixel taking the colour of one
/// up to [`spatter_shift`] away; `smoothness` (1..15) makes the spatter's grains larger and
/// smoother.
pub(super) fn spatter(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, radius: f64, smoothness: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let grain = 0.3 + 0.15 * smoothness.clamp(1.0, 15.0);
    let most = spatter_shift(radius);
    displace(px, w, h, &place, most, |q| {
        let n = |salt: u64| 0.6 * smooth_noise(salt, q, grain) + 0.4 * smooth_noise(salt + 2, q, 0.4 * grain);
        Vec2::new(n(SALT_SPATTER), n(SALT_SPATTER + 1)) * (1.8 * most)
    });
}

/// Sprayed Strokes: the object repainted in sprayed strokes `len` long along `dir`, scattered by
/// up to [`spray_shift`].
pub(super) fn sprayed_strokes(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, len: f64, radius: f64, dir: StrokeDirection) {
    let Some(place) = Place::new(space, w, h) else { return };
    let smear = Smear::new(px, w, h, &place, len);
    let (dir, spray) = (dir.along(), spray_shift(radius));
    let along = (0.5 * smear.len).max(0.5);
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let n = |salt: u64| streak(salt, q, dir, along, 0.5);
        let mut d = Vec2::new(n(SALT_SPRAYED), n(SALT_SPRAYED + 1)) * (1.6 * spray);
        let l = d.hypot();
        if l > spray && l.is_finite() {
            d *= spray / l;
        }
        let (c, a) = unpremultiply(smear.stroke(SALT_SPRAYED + 2, q + d, dir, (0.2 * smear.len).max(1.0)));
        *p = premultiply(c, a);
    });
}

/// Sumi-e: the object as if painted with a wet brush on rice paper, in strokes `width` (3..15)
/// across with soft, bleeding edges and rich blacks; `pressure` (0..15) loads the brush with more
/// ink and `contrast` (0..40) sets the contrast. It works in the ink (the value): the colours keep
/// their hue and saturation, and the object keeps its shape.
pub(super) fn sumi_e(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, width: f64, pressure: f64, contrast: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let smear = Smear::new(px, w, h, &place, width);
    // The ink around each pixel: the object's darkness by its coverage, softened.
    let mut wet: Vec<f32> = px.iter().map(|p| (1.0 - luma(straight(*p))) * f32::from(p[3]) / 255.0).collect();
    blur_plane(&mut wet, w, h, 0.2 * smear.len / place.px);
    let load = 1.0 + 0.06 * pressure.clamp(0.0, 15.0);
    // How far the ink moves along an S curve: up to once at 20, twice at 40.
    let gain = contrast.clamp(0.0, 40.0) / 20.0;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let (c, a) = unpremultiply(smear.stroke(SALT_SUMI, q, LEFT, (0.3 * smear.len).max(1.0)));
        // Where the stroke brings no paint, the pixel's own colour.
        let c = if a > 0.0 { c } else { straight(*p) };
        let l = f64::from(luma(c));
        // The ink bleeds into the paper around the darks, along its fibres.
        let bleed = 0.85 * f64::from(wet.get(y * w + x).copied().unwrap_or(0.0));
        let fibre = 1.0 + 0.1 * streak(SALT_SUMI + 1, q, Vec2::new(1.0, 0.0), 2.0, 0.35);
        let mut ink = ((1.0 - l).max(bleed) * load * fibre).clamp(0.0, 1.0);
        let mut left = gain;
        while left > 0.0 {
            ink += (ramp(ink) - ink) * left.min(1.0);
            left -= 1.0;
        }
        let tone = (1.0 - ink).clamp(0.0, 1.0);
        // Darker towards black, lighter towards the paper's white: the hue stays.
        let out = if tone <= l {
            let k = if l > 1e-3 { (tone / l) as f32 } else { 0.0 };
            c.map(|v| v * k)
        } else {
            let k = ((tone - l) / (1.0 - l).max(1e-3)).clamp(0.0, 1.0) as f32;
            c.map(|v| v + (1.0 - v) * k)
        };
        *p = premultiply(out, f32::from(p[3]) / 255.0);
    });
}
