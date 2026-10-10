//! Texture › Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass and Texturizer.
//!
//! Like the Pixelate filters' patterns, theirs (cracks, grain, tiles, squares, panes and the
//! Texturizer's surfaces, all generated here, no image files) lie in document space around the
//! object's centre: the same at any zoom or resolution, moving with the object, and the same on
//! every redraw (hash noise of the pattern's cell indices). Surfaces in relief are height fields
//! whose slopes, taken in document units, are lit from one side.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use vectorcraft_geom::{Point, Vec2};

use super::pixelate::{AROUND, Around, Place, Soft, cell, gap, point_in, rand, value_noise};
use super::{PixelSpace, blur_plane, pick};

/// Grain's types, in its Grain Type menu's order: (label, parameter value).
pub const GRAIN_TYPES: [(&str, &str); 10] = [
    ("Regular", "regular"),
    ("Soft", "soft"),
    ("Sprinkles", "sprinkles"),
    ("Clumped", "clumped"),
    ("Contrasty", "contrasty"),
    ("Enlarged", "enlarged"),
    ("Stippled", "stippled"),
    ("Horizontal", "horizontal"),
    ("Vertical", "vertical"),
    ("Speckle", "speckle"),
];

/// A Grain type ([`GRAIN_TYPES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grain {
    Regular,
    Soft,
    Sprinkles,
    Clumped,
    Contrasty,
    Enlarged,
    Stippled,
    Horizontal,
    Vertical,
    Speckle,
}

impl Grain {
    const ALL: [Grain; 10] = [
        Grain::Regular,
        Grain::Soft,
        Grain::Sprinkles,
        Grain::Clumped,
        Grain::Contrasty,
        Grain::Enlarged,
        Grain::Stippled,
        Grain::Horizontal,
        Grain::Vertical,
        Grain::Speckle,
    ];

    /// The type of Grain Type `value` (a [`GRAIN_TYPES`] value, any case); Regular otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&GRAIN_TYPES, &Self::ALL, value, Grain::Regular)
    }
}

/// Texturizer's textures, in its Texture menu's order: (label, parameter value).
pub const TEXTURES: [(&str, &str); 4] = [("Brick", "brick"), ("Burlap", "burlap"), ("Canvas", "canvas"), ("Sandstone", "sandstone")];

/// A Texturizer surface ([`TEXTURES`]), made in code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    Brick,
    Burlap,
    Canvas,
    Sandstone,
}

impl Texture {
    const ALL: [Texture; 4] = [Texture::Brick, Texture::Burlap, Texture::Canvas, Texture::Sandstone];

    /// The texture of `value` (a [`TEXTURES`] value, any case); Canvas otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&TEXTURES, &Self::ALL, value, Texture::Canvas)
    }

    /// The surface's height at `p` (points at 100 %), in points: slopes of about 1 at most.
    pub(super) fn height(self, p: Point) -> f64 {
        match self {
            Texture::Brick => brick(p),
            Texture::Burlap => weave(p, 7.0, true),
            Texture::Canvas => weave(p, 4.0, false),
            Texture::Sandstone => sandstone(p),
        }
    }
}

/// Texturizer's light directions, in its Light menu's order: (label, parameter value).
pub const LIGHT_DIRECTIONS: [(&str, &str); 8] = [
    ("Bottom", "bottom"),
    ("Bottom Left", "bottomLeft"),
    ("Left", "left"),
    ("Top Left", "topLeft"),
    ("Top", "top"),
    ("Top Right", "topRight"),
    ("Right", "right"),
    ("Bottom Right", "bottomRight"),
];

/// Where a surface is lit from ([`LIGHT_DIRECTIONS`]), on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
}

impl Light {
    const ALL: [Light; 8] =
        [Light::Bottom, Light::BottomLeft, Light::Left, Light::TopLeft, Light::Top, Light::TopRight, Light::Right, Light::BottomRight];

    /// The direction of `value` (a [`LIGHT_DIRECTIONS`] value, any case); Top otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&LIGHT_DIRECTIONS, &Self::ALL, value, Light::Top)
    }

    /// The unit vector towards the light (document axes, y down).
    fn toward(self) -> Vec2 {
        let d = FRAC_1_SQRT_2;
        match self {
            Light::Bottom => Vec2::new(0.0, 1.0),
            Light::BottomLeft => Vec2::new(-d, d),
            Light::Left => Vec2::new(-1.0, 0.0),
            Light::TopLeft => Vec2::new(-d, -d),
            Light::Top => Vec2::new(0.0, -1.0),
            Light::TopRight => Vec2::new(d, -d),
            Light::Right => Vec2::new(1.0, 0.0),
            Light::BottomRight => Vec2::new(d, d),
        }
    }
}

/// Seeds of the Texture filters' patterns (Pixelate's use smaller salts).
const SALT_CRACKS: u64 = 64;
const SALT_GRAIN: u64 = 72;
const SALT_TILES: u64 = 88;
const SALT_PATCH: u64 = 96;
const SALT_GLASS: u64 = 104;
const SALT_SURFACE: u64 = 112;

/// How soft Craquelure's tones are, as a share of the crack spacing: their contours become cracks.
pub(super) const CONTOUR_SOFTNESS: f64 = 1.0 / 6.0;

/// A pixel's straight colour (0..1).
pub(super) fn straight(p: [u8; 4]) -> [f32; 3] {
    let a = f32::from(p[3]);
    [p[0], p[1], p[2]].map(|v| if a > 0.0 { (f32::from(v) / a).min(1.0) } else { 0.0 })
}

/// The pixel of straight colour `c` at coverage `alpha` (0..1), premultiplied.
pub(super) fn premultiply(c: [f32; 3], alpha: f32) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round();
    let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * a).round().min(a) as u8);
    [r, g, b, a as u8]
}

/// Colour `c` lit by `s`: darkened towards black below 0, lightened towards white above.
fn lit(c: [f32; 3], s: f64) -> [f32; 3] {
    let s = if s.is_finite() { s.clamp(-1.0, 1.0) as f32 } else { 0.0 };
    c.map(|v| if s < 0.0 { v * (1.0 + s) } else { v + (1.0 - v) * s })
}

/// How a surface whose height rises along `slope` (height per length) faces a light from
/// `toward`: 0 flat, positive facing it, negative turned away.
fn facing(slope: Vec2, toward: Vec2) -> f64 {
    -slope.dot(toward)
}

/// The share of a pixel `px` wide that lies within `d` of a line (negative `d`: beyond it).
fn cover(d: f64, px: f64) -> f64 {
    (d / px + 0.5).clamp(0.0, 1.0)
}

/// The luma of colour `c` (Rec. 601 weights).
pub(super) fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// The two random points nearest to (`u`, `v`) (in cells), `point(i, j)` being cell (`i`, `j`)'s,
/// found among the cells [`AROUND`] the one holding it.
fn nearest_two(u: f64, v: f64, around: &mut Around<Point>, point: impl Fn(i64, i64) -> Point) -> Option<(Point, Point)> {
    let (i, j) = (cell(u)?, cell(v)?);
    let mut best: [(f64, Option<Point>); 2] = [(f64::INFINITY, None); 2];
    for (k, (di, dj)) in AROUND.iter().enumerate() {
        // A cell farther than the second point so far can't hold a nearer one.
        if gap(i.wrapping_add(*di), u).hypot(gap(j.wrapping_add(*dj), v)) >= best[1].0 {
            continue;
        }
        let Some(p) = around.get((i, j), k, &point) else { continue };
        let d = (p.x - u).hypot(p.y - v);
        if d < best[0].0 {
            best = [(d, Some(p)), best[0]];
        } else if d < best[1].0 {
            best[1] = (d, Some(p));
        }
    }
    Some((best[0].1?, best[1].1?))
}

/// How far (`u`, `v`) lies from the border between the cells of `near` and `next` (its nearest
/// points, [`nearest_two`]), and the way away from it (unit), in cells.
fn edge(u: f64, v: f64, near: Point, next: Point) -> (f64, Vec2) {
    let q = Point::new(u, v);
    let apart = near - next;
    let len = apart.hypot();
    if !(len.is_finite() && len > 1e-12) {
        return (0.0, Vec2::ZERO);
    }
    (((q - next).hypot2() - (q - near).hypot2()) / (2.0 * len), apart / len)
}

/// The slope of a rise of 1 over `width` from a border at distance `d` (smooth at both ends).
fn bevel(d: f64, width: f64) -> f64 {
    if !(width.is_finite() && width > 0.0) {
        return 0.0;
    }
    let t = (d / width).clamp(0.0, 1.0);
    6.0 * t * (1.0 - t)
}

/// The object's straight luminance (0..1) softened by a Gaussian of `sigma` pixels, weighted by
/// coverage so the transparency around it doesn't darken its edges.
pub(super) struct Tones {
    v: Vec<f32>,
    w: usize,
    h: usize,
}

impl Tones {
    pub(super) fn new(px: &[[u8; 4]], w: usize, h: usize, sigma: f64) -> Self {
        let mut l: Vec<f32> = px.iter().map(|p| luma([p[0], p[1], p[2]].map(f32::from)) / 255.0).collect();
        let mut a: Vec<f32> = px.iter().map(|p| f32::from(p[3]) / 255.0).collect();
        blur_plane(&mut l, w, h, sigma);
        blur_plane(&mut a, w, h, sigma);
        let v = l.iter().zip(&a).map(|(l, a)| if *a > 1e-4 { (l / a).clamp(0.0, 1.0) } else { 0.0 }).collect();
        Self { v, w, h }
    }

    pub(super) fn get(&self, x: usize, y: usize) -> f64 {
        let (x, y) = (x.min(self.w.saturating_sub(1)), y.min(self.h.saturating_sub(1)));
        self.v.get(y * self.w + x).map_or(0.0, |v| f64::from(*v))
    }

    /// The tone at pixel (`x`, `y`) and its slope per document unit along the document's axes.
    fn slope(&self, x: usize, y: usize, place: &Place) -> (f64, Vec2) {
        let gx = 0.5 * (self.get(x + 1, y) - self.get(x.saturating_sub(1), y));
        let gy = 0.5 * (self.get(x, y + 1) - self.get(x, y.saturating_sub(1)));
        // Per pixel → per document unit: through the transpose of the document → pixel map.
        let [a, b, c, d, _, _] = place.to_px.as_coeffs();
        (self.get(x, y), Vec2::new(gx * a + gy * b, gx * c + gy * d))
    }
}

/// Craquelure: the object painted on relief plaster that cracks into plates about `spacing` apart
/// and along the contours of its tones. `depth` (0..10) sets how deep and wide the cracks are and
/// how high the plates and the tones stand; `brightness` (0..10) how brightly the surface is lit.
pub(super) fn craquelure(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, spacing: f64, depth: f64, brightness: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(spacing.is_finite() && spacing > 0.0) {
        return;
    }
    let depth = (depth / 10.0).clamp(0.0, 1.0);
    let light = (0.5 + 0.05 * brightness.clamp(0.0, 10.0)) as f32;
    let soft = CONTOUR_SOFTNESS * spacing;
    let tones = Tones::new(px, w, h, soft / place.px);
    let half = (0.15 + 0.6 * depth).min(0.15 * spacing);
    let rise = (3.0 * half).max(0.12 * spacing);
    let toward = Light::TopLeft.toward();
    let mut around = Around::new();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let (u, v) = (q.x / spacing, q.y / spacing);
        // The cracks between the plates; each plate's edge bevels down into them.
        let (mut crack, mut slope) = (0.0, Vec2::ZERO);
        if let Some((near, next)) = nearest_two(u, v, &mut around, |i, j| point_in(SALT_CRACKS, i, j)) {
            let (e, away) = edge(u, v, near, next);
            crack = cover(half - e * spacing, place.px);
            slope = away * (0.6 * bevel(e * spacing - half, rise));
        }
        // The cracks along the tones' contours (quarter, half and three-quarter tone).
        let (tone, grad) = tones.slope(x, y, &place);
        let steep = grad.hypot();
        if steep > 1e-4 {
            let level = (tone * 4.0).round().clamp(1.0, 3.0) / 4.0;
            crack = crack.max(cover(0.8 * half - (tone - level).abs() / steep, place.px));
        }
        slope += grad * (0.8 * soft);
        let c = lit(straight(*p), 0.5 * depth * facing(slope, toward));
        let dark = (1.0 - 0.85 * crack * depth) as f32;
        *p = premultiply(c.map(|v| v * light * dark), f32::from(p[3]) / 255.0);
    });
}

/// A random value in [-1, 1) for the `size`-point cell holding `q`; `k` picks one of several.
pub(super) fn cell_noise(salt: u64, q: Point, size: f64, k: u64) -> f64 {
    match (cell(q.x / size), cell(q.y / size)) {
        (Some(i), Some(j)) => 2.0 * rand(salt, i, j, k) - 1.0,
        _ => 0.0,
    }
}

/// [`value_noise`] in [-1, 1) over cells `size` points wide.
pub(super) fn smooth_noise(salt: u64, q: Point, size: f64) -> f64 {
    2.0 * value_noise(salt, q.x / size, q.y / size) - 1.0
}

/// Grain: noise of `kind` over the object in 1-point grains (2 for Enlarged, clumps for Clumped),
/// as strong as `intensity` (0..100); `contrast` (0..100, 50 leaves it) sets the image's
/// contrast. Sprinkles and Stippled use the background colour, white.
pub(super) fn grain(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, intensity: f64, contrast: f64, kind: Grain) {
    let Some(place) = Place::new(space, w, h) else { return };
    let a = (intensity / 100.0).clamp(0.0, 1.0);
    let k = (0.5 + contrast / 100.0).clamp(0.5, 1.5) as f32;
    let s = SALT_GRAIN;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let mut c = straight(*p).map(|v| (v - 0.5) * k + 0.5);
        let chance = 0.5 * (cell_noise(s, q, 1.0, 3) + 1.0);
        match kind {
            Grain::Regular => add(&mut c, |ch| cell_noise(s, q, 1.0, ch), 0.5 * a),
            Grain::Soft => add(&mut c, |ch| smooth_noise(s + ch, q, 1.0), 0.4 * a),
            Grain::Clumped => add(&mut c, |ch| 0.65 * smooth_noise(s + ch, q, 3.0) + 0.35 * cell_noise(s, q, 1.0, ch), 0.6 * a),
            Grain::Enlarged => add(&mut c, |ch| 0.5 * (smooth_noise(s + ch, q, 2.0) + cell_noise(s, q, 2.0, ch)), 0.5 * a),
            Grain::Contrasty => {
                c = c.map(|v| (v - 0.5) * 1.4 + 0.5);
                add(&mut c, |_| cell_noise(s, q, 1.0, 4), 0.6 * a);
            }
            Grain::Horizontal | Grain::Vertical => {
                // Streaks one point thick: a value per row (column) drifting along it.
                let (along, across) = if kind == Grain::Horizontal { (q.x, q.y) } else { (q.y, q.x) };
                let line = cell(across).unwrap_or(0);
                let n = 0.6 * (2.0 * rand(s, 0, line, 6) - 1.0) + 0.4 * (2.0 * value_noise(s + 7, along / 12.0, line as f64) - 1.0);
                add(&mut c, |_| n, 0.5 * a);
            }
            Grain::Sprinkles => {
                if chance < 0.35 * a {
                    c = c.map(|v| v + (1.0 - v) * 0.9);
                }
            }
            Grain::Speckle => {
                if chance < 0.25 * a {
                    c = c.map(|v| v * 0.25);
                } else {
                    add(&mut c, |_| cell_noise(s, q, 1.0, 5), 0.1 * a);
                }
            }
            Grain::Stippled => {
                // Dots of the colour where it is dark, the background between them.
                let dots = f64::from(1.0 - luma(c).clamp(0.0, 1.0)) * (0.35 + 0.65 * a);
                c = if chance < dots { c.map(|v| v * 0.6) } else { [1.0; 3] };
            }
        }
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    });
}

/// Add noise `n(channel)` (-1..1) times `amount` to colour `c`.
fn add(c: &mut [f32; 3], n: impl Fn(u64) -> f64, amount: f64) {
    for (ch, v) in c.iter_mut().enumerate() {
        *v += (amount * n(ch as u64)) as f32;
    }
}

/// A random point of cell (`i`, `j`) of pattern `salt` within `jitter` (0..1, a share of the
/// cell) of its centre, in cells.
fn jittered(salt: u64, i: i64, j: i64, jitter: f64) -> Point {
    Point::new(i as f64 + 0.5 + jitter * (rand(salt, i, j, 0) - 0.5), j as f64 + 0.5 + jitter * (rand(salt, i, j, 1) - 0.5))
}

/// Mosaic Tiles: the object laid in irregular tiles about `tile` across, bevelled at their edges,
/// with sunken grout between them half a point wide per step of `grout` (1..15), as light as
/// `lighten` (0..10) says.
pub(super) fn mosaic_tiles(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, tile: f64, grout: f64, lighten: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(tile.is_finite() && tile > 0.0 && grout.is_finite()) {
        return;
    }
    let half = (0.25 * grout).clamp(0.0, 0.225 * tile);
    let rise = (1.2 * half).max(0.08 * tile);
    let lighten = (0.08 * lighten.clamp(0.0, 10.0)) as f32;
    let toward = Light::TopLeft.toward();
    let mut around = Around::new();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let (u, v) = (q.x / tile, q.y / tile);
        let Some((near, next)) = nearest_two(u, v, &mut around, |i, j| jittered(SALT_TILES, i, j, 0.5)) else { return };
        let (e, away) = edge(u, v, near, next);
        let e = e * tile;
        let c = straight(*p);
        let face = lit(c, 0.35 * facing(away * bevel(e - half, rise), toward));
        // Sunk in shade, then lightened.
        let sunk = c.map(|v| 0.5 * v + (1.0 - 0.5 * v) * lighten);
        let g = cover(half - e, place.px) as f32;
        let mut out = face;
        for (o, s) in out.iter_mut().zip(sunk) {
            *o = *o * (1.0 - g) + s * g;
        }
        *p = premultiply(out, f32::from(p[3]) / 255.0);
    });
}

/// Patchwork: the object in squares `square` points across, each filled with the colour around
/// its centre and raised to a height of its own (brighter squares stand higher, each a little
/// more or less at random), its bevelled sides lit from the top left as strongly as `relief`
/// (0..25) says.
pub(super) fn patchwork(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, square: f64, relief: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    let side = square.max(1.0);
    if !side.is_finite() {
        return;
    }
    let soft = Soft::new(px, w, side / 3.0 / place.px);
    let relief = (relief / 25.0).clamp(0.0, 1.0);
    let rise = 0.25 * side;
    let toward = Light::TopLeft.toward();
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let (Some(i), Some(j)) = (cell(q.x / side), cell(q.y / side)) else { return };
        let (x0, y0) = (i as f64 * side, j as f64 * side);
        let [r, g, b, alpha] = soft.at(place.index(Point::new(x0 + 0.5 * side, y0 + 0.5 * side)));
        if alpha <= 0.0 {
            *p = [0; 4];
            return;
        }
        let c = [r, g, b].map(|v| v / alpha);
        let height = relief * (0.35 + 0.65 * (0.5 * rand(SALT_PATCH, i, j, 0) + 0.5 * f64::from(luma(c))));
        // The bevels rise inwards from each side.
        let (fx, fy) = (q.x - x0, q.y - y0);
        let ramp = |d: f64| cover(rise - d, place.px);
        let slope = Vec2::new(ramp(fx) - ramp(side - fx), ramp(fy) - ramp(side - fy)) * (1.5 * height);
        *p = premultiply(lit(c, 0.9 * facing(slope, toward)), alpha);
    });
}

/// Stained Glass: the object as panes of single colours around random points about `cell_size`
/// apart, leaded in the foreground colour (black) half a point wide per step of `border` (1..20),
/// lit from behind at its centre as strongly as `light` (0..10) says, fading out towards its
/// corners.
pub(super) fn stained_glass(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, cell_size: f64, border: f64, light: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(cell_size.is_finite() && cell_size > 0.0 && border.is_finite()) {
        return;
    }
    let soft = Soft::new(px, w, cell_size / place.px / 4.0);
    let half = (0.25 * border).clamp(0.0, 0.25 * cell_size);
    let light = (light / 10.0).clamp(0.0, 1.0);
    let extent = space.extent;
    let mut around = Around::new();
    place.each(px, |x, y, p| {
        let q = place.at(x, y);
        let (u, v) = (q.x / cell_size, q.y / cell_size);
        let Some((near, next)) = nearest_two(u, v, &mut around, |i, j| point_in(SALT_GLASS, i, j)) else { return };
        let pane = |at: Point| soft.at(place.index(Point::new(at.x * cell_size, at.y * cell_size)));
        let (glass, other) = (pane(near), pane(next));
        let (e, _) = edge(u, v, near, next);
        let lead = cover(half - e * cell_size, place.px) as f32;
        let glow = if extent.is_finite() && extent > 0.0 { light * (1.0 - (q.to_vec2().hypot() / extent).powi(2)).max(0.0) } else { 0.0 };
        let [r, g, b, a] = glass;
        let c = if a > 0.0 { lit([r, g, b].map(|v| v / a), 0.6 * glow) } else { [0.0; 3] };
        // The lead holds the panes on both sides of it, as opaque as the more opaque one.
        let alpha = a * (1.0 - lead) + a.max(other[3]) * lead;
        let rgb = c.map(|v| v * a * (1.0 - lead));
        *p = if alpha > 0.0 { premultiply(rgb.map(|v| v / alpha), alpha) } else { [0; 4] };
    });
}

/// Brick in stretcher bond (bricks 20 × 8 points, 1.5-point mortar): each brick raised with
/// 1-point bevels and a little higher or lower than the next, rough faced, the mortar sandy.
fn brick(p: Point) -> f64 {
    const LENGTH: f64 = 20.0;
    const COURSE: f64 = 8.0;
    const MORTAR: f64 = 1.5;
    let Some(row) = cell(p.y / COURSE) else { return 0.0 };
    let x = p.x + if row.rem_euclid(2) == 1 { 0.5 * LENGTH } else { 0.0 };
    let Some(col) = cell(x / LENGTH) else { return 0.0 };
    let (fx, fy) = (x - col as f64 * LENGTH, p.y - row as f64 * COURSE);
    let edge = fx.min(LENGTH - fx).min(fy).min(COURSE - fy) - 0.5 * MORTAR;
    if edge <= 0.0 {
        return 0.2 * value_noise(SALT_SURFACE, p.x / 0.6, p.y / 0.6);
    }
    edge.min(1.0) + 0.4 * rand(SALT_SURFACE, col, row, 2) + 0.3 * value_noise(SALT_SURFACE + 1, p.x / 1.5, p.y / 1.5)
}

/// A plain weave of threads `period` points apart, each passing over one thread and under the
/// next; `coarse` threads (burlap) wander and are hairy, fine ones (canvas) are even.
fn weave(p: Point, period: f64, coarse: bool) -> f64 {
    let (wander, hair) = if coarse { (0.35, 0.35) } else { (0.08, 0.12) };
    let drift = |salt: u64, t: f64| wander * (2.0 * value_noise(salt, t / (4.0 * period), 0.5) - 1.0);
    let x = p.x / period + drift(SALT_SURFACE + 3, p.y);
    let y = p.y / period + drift(SALT_SURFACE + 4, p.x);
    let (Some(i), Some(j)) = (cell(x), cell(y)) else { return 0.0 };
    let (fx, fy) = (x - i as f64, y - j as f64);
    // Each thread is round across and rises and falls along its length, over one crossing thread
    // and under the next; the higher of the two threads shows.
    let profile = |t: f64| (PI * t).sin().max(0.0).sqrt();
    let over = if i.wrapping_add(j).rem_euclid(2) == 0 { 0.5 } else { -0.5 };
    let down = profile(fx) * (0.5 + over * (PI * fy).sin());
    let across = profile(fy) * (0.5 - over * (PI * fx).sin());
    let thread = down.max(across);
    let fibres = hair * value_noise(SALT_SURFACE + 5, p.x / (0.25 * period), p.y / (0.25 * period));
    (thread + fibres) * period / PI
}

/// Sandstone: four octaves of smooth noise (8 down to 1 point) over a fine sandy grain.
fn sandstone(p: Point) -> f64 {
    let (mut h, mut size, mut amp) = (0.0, 8.0, 1.6);
    for octave in 0..4 {
        h += amp * value_noise(SALT_SURFACE + 8 + octave, p.x / size, p.y / size);
        size *= 0.5;
        amp *= 0.5;
    }
    h + 0.06 * cell_noise(SALT_SURFACE + 12, p, 0.5, 0)
}

/// Texturizer: the object on a `texture` surface scaled by `scaling` (0.5..2), in relief as
/// strong as `relief` (0..50) and lit from `light`; `invert` turns the surface's heights over.
#[allow(clippy::too_many_arguments)] // the effect's five options
pub(super) fn texturizer(
    px: &mut [[u8; 4]],
    w: usize,
    h: usize,
    space: &PixelSpace,
    texture: Texture,
    scaling: f64,
    relief: f64,
    light: Light,
    invert: bool,
) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(scaling.is_finite() && scaling > 0.0) {
        return;
    }
    // Slopes over a quarter point of the texture, or half a pixel where pixels are larger.
    let step = (0.25f64).max(0.5 * place.px / scaling);
    let strength = 0.1 * relief.clamp(0.0, 50.0) * if invert { -1.0 } else { 1.0 };
    let toward = light.toward();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let t = Point::new(q.x / scaling, q.y / scaling);
        if !(t.x.is_finite() && t.y.is_finite()) {
            return;
        }
        let at = |dx: f64, dy: f64| texture.height(Point::new(t.x + dx, t.y + dy));
        let (left, right, up, down) = (at(-step, 0.0), at(step, 0.0), at(0.0, -step), at(0.0, step));
        let slope = Vec2::new(right - left, down - up) / (2.0 * step);
        // Ridges catch some light from any side and hollows stay in shade, so the surface shows
        // whichever way it runs.
        let ridge = (at(0.0, 0.0) - 0.25 * (left + right + up + down)) / step;
        *p = premultiply(lit(straight(*p), strength * (facing(slope, toward) + ridge)), f32::from(p[3]) / 255.0);
    });
}
