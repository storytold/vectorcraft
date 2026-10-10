//! Pixelate › Color Halftone, Crystallize, Mezzotint and Pointillize.
//!
//! Their patterns (screens, crystals, grains and dots) lie in document space around the object's
//! centre, so they keep their size at any zoom or resolution and move with the object. The random
//! ones hash the pattern's cell indices: every redraw gives the same result.

use std::sync::OnceLock;

use vectorcraft_geom::{Affine, Point};

use super::{Channels, PixelSpace, Px16, gaussian, pick, to16};
use crate::util::noise;

/// Mezzotint's patterns, in its Type menu's order: (label, parameter value).
pub const MEZZOTINT_TYPES: [(&str, &str); 10] = [
    ("Fine Dots", "fineDots"),
    ("Medium Dots", "mediumDots"),
    ("Grainy Dots", "grainyDots"),
    ("Coarse Dots", "coarseDots"),
    ("Short Lines", "shortLines"),
    ("Medium Lines", "mediumLines"),
    ("Long Lines", "longLines"),
    ("Short Strokes", "shortStrokes"),
    ("Medium Strokes", "mediumStrokes"),
    ("Long Strokes", "longStrokes"),
];

/// A Mezzotint pattern ([`MEZZOTINT_TYPES`]): random dots, or horizontal lines or strokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mezzotint {
    /// Dots `size` points across (smooth blobs when `smooth`), plus fine grain when `grainy`.
    Dots { size: u8, smooth: bool, grainy: bool },
    /// Horizontal streaks about `length` points long and `width` points thick.
    Lines { length: u8, width: u8 },
}

/// The patterns of [`MEZZOTINT_TYPES`], in the same order.
const MEZZOTINT_KINDS: [Mezzotint; 10] = {
    const fn dots(size: u8, smooth: bool, grainy: bool) -> Mezzotint {
        Mezzotint::Dots { size, smooth, grainy }
    }
    const fn lines(length: u8, width: u8) -> Mezzotint {
        Mezzotint::Lines { length, width }
    }
    [
        dots(1, false, false),
        dots(2, true, false),
        dots(3, true, true),
        dots(4, true, false),
        lines(4, 1),
        lines(8, 1),
        lines(16, 1),
        lines(6, 2),
        lines(12, 2),
        lines(24, 2),
    ]
};

impl Mezzotint {
    /// Fine Dots, the first type.
    pub const FINE_DOTS: Mezzotint = Mezzotint::Dots { size: 1, smooth: false, grainy: false };

    /// The pattern of Type `value` (a [`MEZZOTINT_TYPES`] value, any case); Fine Dots otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&MEZZOTINT_TYPES, &MEZZOTINT_KINDS, value, Self::FINE_DOTS)
    }
}

/// Seeds of the filters' noise ("PIXELATE" and a filter number).
const SEED: u64 = 0x5049_5845_4C41_5445;

/// A random number in [0, 1) for cell (`i`, `j`) of pattern `salt`; `k` picks one of several.
pub(super) fn rand(salt: u64, i: i64, j: i64, k: u64) -> f64 {
    0.5 * (noise(SEED ^ salt, i as u64, j as u64, k) + 1.0)
}

/// The cell holding coordinate `v` (in cells); `None` far beyond any raster (or not finite).
pub(super) fn cell(v: f64) -> Option<i64> {
    let f = v.floor();
    (f.abs() < 1e15).then_some(f as i64)
}

/// Where a raster's pixels lie relative to the object's centre, in document units.
pub(super) struct Place {
    /// Pixel coordinates → document coordinates relative to the centre.
    pub(super) to_rel: Affine,
    /// The other way.
    pub(super) to_px: Affine,
    /// A pixel's size in document units.
    pub(super) px: f64,
    w: usize,
    h: usize,
}

impl Place {
    pub(super) fn new(space: &PixelSpace, w: usize, h: usize) -> Option<Self> {
        let to_rel = Affine::translate(-space.center.to_vec2()) * space.to_doc;
        let to_px = to_rel.inverse();
        (to_rel.is_finite() && to_px.is_finite() && space.px.is_finite() && space.px > 0.0).then_some(Self { to_rel, to_px, px: space.px, w, h })
    }

    /// The centre of pixel (`x`, `y`).
    pub(super) fn at(&self, x: usize, y: usize) -> Point {
        self.to_rel * Point::new(x as f64 + 0.5, y as f64 + 0.5)
    }

    /// The index of the pixel holding point `p`; `None` outside the raster.
    pub(super) fn index(&self, p: Point) -> Option<usize> {
        let q = self.to_px * p;
        (q.x >= 0.0 && q.y >= 0.0 && q.x < self.w as f64 && q.y < self.h as f64).then(|| q.y as usize * self.w + q.x as usize)
    }

    /// Run `f(x, y, pixel)` on every pixel.
    pub(super) fn each(&self, px: &mut [[u8; 4]], mut f: impl FnMut(usize, usize, &mut [u8; 4])) {
        for (y, row) in px.chunks_exact_mut(self.w.max(1)).enumerate().take(self.h) {
            for (x, p) in row.iter_mut().enumerate() {
                f(x, y, p);
            }
        }
    }
}

/// The cells around a pixel's cell, in the order they can hold its nearest random point: the
/// cell itself, its 8 neighbours, then the ring around them.
pub(super) const AROUND: [(i64, i64); 25] = [
    (0, 0),
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
    (-2, -2),
    (-1, -2),
    (0, -2),
    (1, -2),
    (2, -2),
    (-2, -1),
    (2, -1),
    (-2, 0),
    (2, 0),
    (-2, 1),
    (2, 1),
    (-2, 2),
    (-1, 2),
    (0, 2),
    (1, 2),
    (2, 2),
];

/// What the cells [`AROUND`] one cell hold, made when first asked for and kept while consecutive
/// pixels stay in that cell.
pub(super) struct Around<T> {
    cell: Option<(i64, i64)>,
    items: [Option<T>; 25],
}

impl<T: Copy> Around<T> {
    pub(super) fn new() -> Self {
        Self { cell: None, items: [None; 25] }
    }

    /// Item `k` of [`AROUND`] cell (`i`, `j`): `make(cell i', j')`.
    pub(super) fn get(&mut self, (i, j): (i64, i64), k: usize, make: impl FnOnce(i64, i64) -> T) -> Option<T> {
        if self.cell != Some((i, j)) {
            self.cell = Some((i, j));
            self.items = [None; 25];
        }
        let (di, dj) = AROUND.get(k)?;
        let slot = self.items.get_mut(k)?;
        Some(*slot.get_or_insert_with(|| make(i.wrapping_add(*di), j.wrapping_add(*dj))))
    }
}

/// The random point of cell (`i`, `j`) of pattern `salt`, in cells.
pub(super) fn point_in(salt: u64, i: i64, j: i64) -> Point {
    Point::new(i as f64 + rand(salt, i, j, 0), j as f64 + rand(salt, i, j, 1))
}

/// How far cell `c` lies from coordinate `t` (in cells) along one axis.
pub(super) fn gap(c: i64, t: f64) -> f64 {
    (c as f64 - t).max(t - c as f64 - 1.0).max(0.0)
}

/// Smooth random values in [0, 1) of pattern `salt` at whole coordinates, blended between them.
pub(super) fn value_noise(salt: u64, x: f64, y: f64) -> f64 {
    let (Some(i), Some(j)) = (cell(x), cell(y)) else { return 0.5 };
    let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
    let (fx, fy) = (smooth(x - i as f64), smooth(y - j as f64));
    let at = |di: i64, dj: i64| rand(salt, i.wrapping_add(di), j.wrapping_add(dj), 0);
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * fx;
    let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * fx;
    top + (bottom - top) * fy
}

/// Crystallize: every pixel takes the colour at the nearest of one random point per `cell` ×
/// `cell` square (a Voronoi diagram over a jittered grid): polygon crystals of solid colour.
pub(super) fn crystallize(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, cell_size: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(cell_size.is_finite() && cell_size > 0.0) {
        return;
    }
    let src = px.to_vec();
    let mut around = Around::new();
    place.each(px, |x, y, out| {
        let q = place.at(x, y);
        let (u, v) = (q.x / cell_size, q.y / cell_size);
        let (Some(i), Some(j)) = (cell(u), cell(v)) else { return };
        let mut best = (f64::INFINITY, None);
        for (k, (di, dj)) in AROUND.iter().enumerate() {
            // A cell farther than the best point so far can't hold a nearer one.
            let (ci, cj) = (i.wrapping_add(*di), j.wrapping_add(*dj));
            if gap(ci, u).hypot(gap(cj, v)) >= best.0 {
                continue;
            }
            if let Some(p) = around.get((i, j), k, |a, b| point_in(1, a, b)) {
                let d = (p.x - u).hypot(p.y - v);
                if d < best.0 {
                    best = (d, Some(p));
                }
            }
        }
        if let Some(p) = best.1 {
            let at = Point::new(p.x * cell_size, p.y * cell_size);
            *out = place.index(at).and_then(|i| src.get(i)).copied().unwrap_or([0; 4]);
        }
    });
}

/// The object's colours softened by a Gaussian of `sigma` pixels, with its coverage as it is: what
/// the cell filters (Pointillize, Patchwork, Stained Glass) fill their cells with.
pub(super) struct Soft {
    colour: Vec<Px16>,
    alpha: Vec<u8>,
}

impl Soft {
    pub(super) fn new(px: &[[u8; 4]], w: usize, sigma: f64) -> Self {
        let mut colour = to16(px);
        gaussian(&mut colour, w, sigma);
        Self { colour, alpha: px.iter().map(|p| p[3]).collect() }
    }

    /// The colour around pixel `k` (premultiplied, 0..1), as opaque as the object is at `k`; clear
    /// outside the raster.
    pub(super) fn at(&self, k: Option<usize>) -> [f32; 4] {
        let a = k.and_then(|k| self.alpha.get(k)).map_or(0.0, |a| f32::from(*a) / 255.0);
        match k.and_then(|k| self.colour.get(k)) {
            Some(p) if p[3] > 0 => {
                let s = a / f32::from(p[3]);
                [f32::from(p[0]) * s, f32::from(p[1]) * s, f32::from(p[2]) * s, a]
            }
            _ => [0.0; 4],
        }
    }
}

/// One of Pointillize's dots: where (in cells), its radius (document units), its stacking order
/// and its colour (premultiplied, 0..1).
#[derive(Clone, Copy)]
struct Dot {
    at: Point,
    radius: f64,
    z: f64,
    colour: [f32; 4],
}

/// The smallest dot radius and the spread above it, in cells.
const DOT_RADIUS: (f64, f64) = (0.45, 0.25);

/// Pointillize: a dot at one random point per `cell` × `cell` square, of random size around the
/// cell's, coloured by the (softened) object there, over a white canvas where the object is.
pub(super) fn pointillize(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, cell_size: f64) {
    let Some(place) = Place::new(space, w, h) else { return };
    if !(cell_size.is_finite() && cell_size > 0.0) {
        return;
    }
    // Each dot takes the colour around its centre, not of a single pixel there, and is as opaque
    // as the object at its centre (whole dots on the object, none beside it).
    let soft = Soft::new(px, w, cell_size / place.px / 4.0);
    let dot = |i: i64, j: i64| {
        let at = point_in(2, i, j);
        let colour = soft.at(place.index(Point::new(at.x * cell_size, at.y * cell_size)));
        Dot { at, radius: (DOT_RADIUS.0 + DOT_RADIUS.1 * rand(2, i, j, 2)) * cell_size, z: rand(2, i, j, 3), colour }
    };
    let mut around = Around::new();
    place.each(px, |x, y, out| {
        let q = place.at(x, y);
        let (u, v) = (q.x / cell_size, q.y / cell_size);
        let (Some(i), Some(j)) = (cell(u), cell(v)) else { return };
        // The dots covering the pixel (only the 3 × 3 cells around it reach it), bottom first.
        let mut hits: [(f64, f32, [f32; 4]); 9] = [(0.0, 0.0, [0.0; 4]); 9];
        let mut n = 0;
        for k in 0..9 {
            let Some(d) = around.get((i, j), k, dot) else { continue };
            let dist = (d.at.x - u).hypot(d.at.y - v) * cell_size;
            let cover = ((d.radius - dist) / place.px + 0.5).clamp(0.0, 1.0) as f32;
            if cover > 0.0
                && d.colour[3] > 0.0
                && let Some(slot) = hits.get_mut(n)
            {
                *slot = (d.z, cover, d.colour);
                n += 1;
            }
        }
        let hits = hits.get_mut(..n).unwrap_or_default();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let alpha = f32::from(out[3]) / 255.0;
        let mut acc = [alpha; 4];
        for (_, cover, c) in hits.iter() {
            let keep = 1.0 - c[3] * cover;
            for (a, v) in acc.iter_mut().zip(c) {
                *a = v * cover + *a * keep;
            }
        }
        let a = (acc[3].clamp(0.0, 1.0) * 255.0).round();
        let v = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round().min(a) as u8;
        *out = [v(acc[0]), v(acc[1]), v(acc[2]), a as u8];
    });
}

/// A pixel's colour as the channels the Pixelate filters treat apart (straight, 0..1; see
/// [`Channels`]), and how many of the four there are.
fn split(p: [u8; 4], channels: Channels) -> ([f32; 4], usize) {
    let a = f32::from(p[3]);
    let [r, g, b] = [p[0], p[1], p[2]].map(|v| if a > 0.0 { (f32::from(v) / a).min(1.0) } else { 0.0 });
    match channels {
        Channels::Rgb => ([r, g, b, 0.0], 3),
        Channels::Cmyk => {
            // A light black generation, as separations print: black ink takes over in the
            // shadows, coloured inks build the midtones.
            let k = (1.0 - r.max(g).max(b)).powi(2);
            let ink = |v: f32| if k < 1.0 { ((1.0 - v - k) / (1.0 - k)).clamp(0.0, 1.0) } else { 0.0 };
            ([ink(r), ink(g), ink(b), k], 4)
        }
        Channels::CmyPlane => ([1.0 - r, 1.0 - g, 1.0 - b, 0.0], 3),
        Channels::KPlane => ([1.0 - r, 0.0, 0.0, 0.0], 1),
    }
}

/// The pixel of channel values `v` ([`split`]'s) at coverage `alpha` (premultiplied).
fn join(v: [f32; 4], channels: Channels, alpha: u8) -> [u8; 4] {
    let [a, b, c, d] = v.map(|x| x.clamp(0.0, 1.0));
    let rgb = match channels {
        Channels::Rgb => [a, b, c],
        Channels::Cmyk => [(1.0 - a) * (1.0 - d), (1.0 - b) * (1.0 - d), (1.0 - c) * (1.0 - d)],
        Channels::CmyPlane => [1.0 - a, 1.0 - b, 1.0 - c],
        Channels::KPlane => [1.0 - a; 3],
    };
    let al = f32::from(alpha);
    let [r, g, b] = rgb.map(|x| (x * al).round().clamp(0.0, al) as u8);
    [r, g, b, alpha]
}

/// Which of Color Halftone's four screen angles channel `i` of `channels` uses: the K plane's
/// only channel is the fourth (black) one.
fn angle_of(channels: Channels, i: usize) -> usize {
    if channels == Channels::KPlane { 3 } else { i }
}

/// The share of a `1` × `1` square that a circle of radius `rho` around its centre covers.
fn circle_in_square(rho: f64) -> f64 {
    let half = 0.5;
    if rho <= half {
        std::f64::consts::PI * rho * rho
    } else if rho < std::f64::consts::FRAC_1_SQRT_2 {
        // The circle less the four caps beyond the square's sides.
        let cap = rho * rho * (half / rho).acos() - half * (rho * rho - half * half).sqrt();
        std::f64::consts::PI * rho * rho - 4.0 * cap
    } else {
        1.0
    }
}

/// The radius (in cells) of the dot that covers share `v` of its cell.
fn dot_radius(v: f64) -> f64 {
    const STEPS: usize = 256;
    static TABLE: OnceLock<[f64; STEPS + 1]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|k| {
            let target = k as f64 / STEPS as f64;
            let (mut lo, mut hi) = (0.0, std::f64::consts::FRAC_1_SQRT_2);
            for _ in 0..48 {
                let mid = 0.5 * (lo + hi);
                if circle_in_square(mid) < target {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            hi
        })
    });
    let x = if v.is_finite() { v.clamp(0.0, 1.0) * STEPS as f64 } else { 0.0 };
    let k = (x.floor() as usize).min(STEPS - 1);
    let (a, b) = (table.get(k).copied().unwrap_or(0.0), table.get(k + 1).copied().unwrap_or(0.0));
    a + (b - a) * (x - k as f64)
}

/// One channel's halftone screen: square cells at an angle, each with the dot of its mean value.
struct Screen {
    /// Document coordinates relative to the centre → cells.
    to_cells: Affine,
    /// The first cell's indices and the grid's size in cells.
    origin: (i64, i64),
    size: (usize, usize),
    /// Per cell: the channel's sum (× coverage) and the coverage's (then the dot's radius in
    /// document units, in `sums[k][0]`).
    sums: Vec<[f32; 2]>,
}

impl Screen {
    /// The screen at `angle`° with cells `side` wide over the raster `place` covers; `None` when
    /// it would have more than `limit` cells.
    fn new(place: &Place, angle: f64, side: f64, limit: usize) -> Option<Self> {
        let to_cells = Affine::scale(1.0 / side) * Affine::rotate(angle.to_radians());
        let corners = [(0.0, 0.0), (place.w as f64, 0.0), (0.0, place.h as f64), (place.w as f64, place.h as f64)];
        let pts = corners.map(|(x, y)| to_cells * (place.to_rel * Point::new(x, y)));
        let (lo, hi) = pts.iter().fold(((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY)), |(lo, hi), p| {
            ((lo.0.min(p.x), lo.1.min(p.y)), (hi.0.max(p.x), hi.1.max(p.y)))
        });
        let (i0, j0, i1, j1) = (cell(lo.0)?.checked_sub(1)?, cell(lo.1)?.checked_sub(1)?, cell(hi.0)?.checked_add(1)?, cell(hi.1)?.checked_add(1)?);
        let gw = usize::try_from(i1.checked_sub(i0)?.checked_add(1)?).ok()?;
        let gh = usize::try_from(j1.checked_sub(j0)?.checked_add(1)?).ok()?;
        (gw.checked_mul(gh)? <= limit).then(|| Self { to_cells, origin: (i0, j0), size: (gw, gh), sums: vec![[0.0; 2]; gw * gh] })
    }

    /// The index of cell (`i`, `j`) in the grid.
    fn slot(&self, i: i64, j: i64) -> Option<usize> {
        let x = usize::try_from(i.checked_sub(self.origin.0)?).ok().filter(|x| *x < self.size.0)?;
        let y = usize::try_from(j.checked_sub(self.origin.1)?).ok().filter(|y| *y < self.size.1)?;
        Some(y * self.size.0 + x)
    }
}

/// Color Halftone: each channel screened at its angle in square cells sized so that a dot of
/// `max_radius` fills one; each cell's dot covers the share of it that is the channel's mean
/// there (white stays white, black black). Coverage stays as it is.
pub(super) fn color_halftone(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, max_radius: f64, angles: [f64; 4]) {
    let Some(place) = Place::new(space, w, h) else { return };
    let side = max_radius * std::f64::consts::SQRT_2;
    // Cells under 3 pixels can't show their dots: on average a screen is the image itself.
    if !(side.is_finite() && side / place.px >= 3.0) {
        return;
    }
    let channels = space.channels;
    let n = split([0; 4], channels).1;
    let limit = w.saturating_mul(h).saturating_add(4096);
    let mut screens = Vec::with_capacity(n);
    for i in 0..n {
        let angle = angles.get(angle_of(channels, i)).copied().unwrap_or(0.0);
        let Some(s) = Screen::new(&place, angle, side, limit) else { return };
        screens.push(s);
    }
    // Each cell's mean of each channel, weighted by coverage.
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let (v, _) = split(*p, channels);
        let a = f32::from(p[3]) / 255.0;
        let q = place.at(x, y);
        for (s, v) in screens.iter_mut().zip(v) {
            let c = s.to_cells * q;
            if let (Some(i), Some(j)) = (cell(c.x), cell(c.y))
                && let Some(sum) = s.slot(i, j).and_then(|k| s.sums.get_mut(k))
            {
                sum[0] += v * a;
                sum[1] += a;
            }
        }
    });
    for s in &mut screens {
        for sum in &mut s.sums {
            let mean = if sum[1] > 1e-6 { f64::from(sum[0] / sum[1]) } else { 0.0 };
            // Full strength fills the cell, corners too; none leaves no speck.
            let radius = match mean {
                m if m >= 254.5 / 255.0 => f64::INFINITY,
                m if m <= 0.5 / 255.0 => 0.0,
                m => dot_radius(m) * side,
            };
            sum[0] = radius as f32;
        }
    }
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let mut v = [0.0f32; 4];
        for (s, out) in screens.iter().zip(v.iter_mut()) {
            let c = s.to_cells * q;
            let (Some(i), Some(j)) = (cell(c.x), cell(c.y)) else { continue };
            // A dot reaches past its cell into the neighbours on the pixel's side only.
            let (si, sj) = (if c.x - (i as f64) < 0.5 { -1 } else { 1 }, if c.y - (j as f64) < 0.5 { -1 } else { 1 });
            for (di, dj) in [(0, 0), (si, 0), (0, sj), (si, sj)] {
                let (ci, cj) = (i.wrapping_add(di), j.wrapping_add(dj));
                let radius = s.slot(ci, cj).and_then(|k| s.sums.get(k)).map_or(0.0, |sum| f64::from(sum[0]));
                if radius <= 0.0 {
                    continue;
                }
                let dist = (c.x - (ci as f64 + 0.5)).hypot(c.y - (cj as f64 + 0.5)) * side;
                *out = out.max(((radius - dist) / place.px + 0.5).clamp(0.0, 1.0) as f32);
            }
        }
        *p = join(v, channels, p[3]);
    });
}

/// Mezzotint: every channel turned fully on or off where it is above or below a random threshold
/// pattern (each channel its own), so the image becomes dots or streaks of pure colours.
pub(super) fn mezzotint(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, kind: Mezzotint) {
    let Some(place) = Place::new(space, w, h) else { return };
    let channels = space.channels;
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let q = place.at(x, y);
        let (mut v, n) = split(*p, channels);
        for (c, value) in v.iter_mut().enumerate().take(n) {
            let t = threshold(kind, 16 + c as u64, q);
            *value = if f64::from(*value) > t { 1.0 } else { 0.0 };
        }
        *p = join(v, channels, p[3]);
    });
}

/// Mezzotint's threshold in [0, 1) at point `q` (document units around the centre) of pattern
/// `salt`.
fn threshold(kind: Mezzotint, salt: u64, q: Point) -> f64 {
    let grain = |size: f64| match (cell(q.x / size), cell(q.y / size)) {
        (Some(i), Some(j)) => rand(salt, i, j, 1),
        _ => 0.5,
    };
    match kind {
        Mezzotint::Dots { size, smooth: false, .. } => grain(f64::from(size)),
        Mezzotint::Dots { size, grainy, .. } => {
            let s = f64::from(size);
            let blob = value_noise(salt, q.x / s, q.y / s);
            if grainy { 0.5 * (blob + grain(1.0)) } else { blob }
        }
        // Blended along x only: whole rows.
        Mezzotint::Lines { length, width } => value_noise(salt, q.x / f64::from(length), (q.y / f64::from(width)).floor()),
    }
}
