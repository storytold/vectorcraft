//! Dithered linear and radial gradients.
//!
//! vello interpolates gradients straight into 8-bit colour, so slow ramps show bands. A gradient
//! with `dither` on is instead sampled per device pixel in floating point over the painted box,
//! with an ordered (Bayer) threshold of ±½ level added before rounding, and drawn as an image paint
//! (nearest-neighbour, so the dither pattern isn't blurred away). The fill or stroke being painted
//! clips it. Pixmaps are cached by content and size like freeform grids.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use vectorcraft_color::freeform::painted_box;
use vectorcraft_color::{Color, GradientGeom, GradientKind, GradientPaint};
use vectorcraft_geom::{Affine, Point, Rect};
use vello_cpu::peniko::{Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext};

use crate::ink::Ink;

/// Pixmaps kept before the cache starts over.
const MAX_PIXMAPS: usize = 32;
/// Largest side of a dither pixmap: bigger boxes are sampled a little coarser than device pixels.
const MAX_SIDE: f64 = 2048.0;
/// Entries in the colour table each pixmap is sampled from.
const LUT: usize = 1024;

/// The 8×8 Bayer matrix, thresholds 0..64.
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

thread_local! {
    static PIXMAPS: RefCell<HashMap<u64, Arc<Pixmap>>> = RefCell::new(HashMap::new());
}

fn hash_color(c: &Color, h: &mut impl Hasher) {
    match *c {
        Color::Rgb { r, g, b } => (0u8, r.to_bits(), g.to_bits(), b.to_bits()).hash(h),
        Color::Cmyk { c, m, y, k } => (1u8, c.to_bits(), m.to_bits(), y.to_bits(), k.to_bits()).hash(h),
        Color::Gray { k } => (2u8, k.to_bits()).hash(h),
        Color::Lab { l, a, b } => (3u8, l.to_bits(), a.to_bits(), b.to_bits()).hash(h),
    }
}

fn key(g: &GradientPaint, geom: &GradientGeom, b: Rect, cols: u16, rows: u16, ink: Ink) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for s in &g.gradient.stops {
        (s.offset.to_bits(), s.opacity.to_bits(), s.midpoint.to_bits()).hash(&mut h);
        hash_color(&s.color, &mut h);
    }
    (g.gradient.kind as u8, g.gradient.interpolation as u8).hash(&mut h);
    for p in [geom.start, geom.end] {
        (p.x.to_bits(), p.y.to_bits()).hash(&mut h);
    }
    geom.aspect.to_bits().hash(&mut h);
    geom.focal.map(|f| (f.x.to_bits(), f.y.to_bits())).hash(&mut h);
    (b.x0.to_bits(), b.y0.to_bits(), b.x1.to_bits(), b.y1.to_bits(), cols, rows).hash(&mut h);
    (Arc::as_ptr(&vectorcraft_color::cms::active()) as usize, ink).hash(&mut h);
    h.finish()
}

/// Position along the gradient (0..1, padded) of each point, for a linear or radial gradient.
pub(crate) struct Ramp {
    kind: GradientKind,
    /// Linear: start and the vector divided by its squared length.
    start: Point,
    dir: (f64, f64),
    /// Radial: the inverse of the aspect squash, the centre, radius and focal point (in the
    /// unsquashed space).
    unsquash: Affine,
    radius: f64,
    focal: Option<Point>,
}

impl Ramp {
    pub(crate) fn new(kind: GradientKind, geom: &GradientGeom) -> Self {
        let v = geom.end - geom.start;
        let len2 = v.hypot2().max(1e-12);
        let unsquash = geom.radial_squash().inverse();
        Self {
            kind,
            start: geom.start,
            dir: (v.x / len2, v.y / len2),
            unsquash,
            radius: geom.length().max(1e-6),
            focal: geom.focal.map(|f| unsquash * f),
        }
    }

    pub(crate) fn t(&self, p: Point) -> f64 {
        let t = match self.kind {
            GradientKind::Radial => {
                let q = self.unsquash * p;
                match self.focal {
                    // Two-point radial from the focal point (radius 0) to the extent circle: the
                    // t whose circle, centred on f + t (c - f) with radius t r, passes through q.
                    Some(f) if f.distance(self.start) > 1e-9 => {
                        let cd = self.start - f;
                        let pd = q - f;
                        let a = cd.hypot2() - self.radius * self.radius;
                        let b = pd.dot(cd);
                        let c = pd.hypot2();
                        if a.abs() < 1e-9 {
                            if b.abs() < 1e-12 { 0.0 } else { c / (2.0 * b) }
                        } else {
                            let disc = b * b - a * c;
                            if disc < 0.0 { 1.0 } else { (b - disc.sqrt()) / a }
                        }
                    }
                    _ => q.distance(self.start) / self.radius,
                }
            }
            _ => (p.x - self.start.x) * self.dir.0 + (p.y - self.start.y) * self.dir.1,
        };
        if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// Sample `g` on a `cols` × `rows` grid over `b`, dithered (premultiplied, colours as `ink`
/// paints them).
pub(crate) fn rasterize(g: &GradientPaint, geom: &GradientGeom, b: Rect, cols: u16, rows: u16, ink: Ink) -> Pixmap {
    let lut: Vec<[f32; 4]> = (0..LUT)
        .map(|i| {
            let (c, a) = g.gradient.sample(i as f32 / (LUT - 1) as f32);
            let [r, gr, bl] = ink.rgb(&c);
            let a = a.clamp(0.0, 1.0);
            [r.clamp(0.0, 1.0) * a, gr.clamp(0.0, 1.0) * a, bl.clamp(0.0, 1.0) * a, a]
        })
        .collect();
    let ramp = Ramp::new(g.gradient.kind, geom);
    let (sx, sy) = (b.width() / f64::from(cols), b.height() / f64::from(rows));
    let mut data = Vec::with_capacity(usize::from(cols) * usize::from(rows));
    for y in 0..rows {
        for x in 0..cols {
            let p = Point::new(b.x0 + (f64::from(x) + 0.5) * sx, b.y0 + (f64::from(y) + 0.5) * sy);
            let f = ramp.t(p) * (LUT - 1) as f64;
            let i = (f.floor() as usize).min(LUT - 1);
            let u = (f - i as f64) as f32;
            let (Some(c0), Some(c1)) = (lut.get(i), lut.get((i + 1).min(LUT - 1))) else { continue };
            let threshold = BAYER.get(usize::from(y) % 8).and_then(|r| r.get(usize::from(x) % 8)).copied().unwrap_or(32);
            let d = (f32::from(threshold) + 0.5) / 64.0 - 0.5;
            let q = |k: usize| {
                let v = c0[k] + (c1[k] - c0[k]) * u;
                (v * 255.0 + d).round().clamp(0.0, 255.0) as u8
            };
            let a = q(3);
            // Premultiplied channels never exceed alpha.
            data.push(vello_cpu::color::PremulRgba8 { r: q(0).min(a), g: q(1).min(a), b: q(2).min(a), a });
        }
    }
    Pixmap::from_parts(data, cols, rows)
}

/// Set dithered linear or radial gradient `g` on box `bounds` (in the paint's space, which the
/// context's current transform maps to pixels) as the context paint. Returns false for an empty box.
pub(crate) fn set_dithered_paint(ctx: &mut RenderContext, g: &GradientPaint, bounds: Rect, ink: Ink) -> bool {
    let Some(b) = painted_box(bounds) else { return false };
    // Strokes reach past the bounds a little; the padded edge covers the rest.
    let b = b.inflate(b.width() * 0.05, b.height() * 0.05);
    let scale = ctx.transform().determinant().abs().sqrt();
    let side = |v: f64| {
        let px = v * scale;
        if px.is_finite() { px.clamp(1.0, MAX_SIDE).ceil() as u16 } else { 1 }
    };
    let (cols, rows) = (side(b.width()), side(b.height()));
    let geom = g.resolve(bounds);
    let k = key(g, &geom, b, cols, rows, ink);
    let pm = PIXMAPS.with(|c| c.borrow().get(&k).cloned()).unwrap_or_else(|| {
        let pm = Arc::new(rasterize(g, &geom, b, cols, rows, ink));
        PIXMAPS.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() >= MAX_PIXMAPS {
                c.clear();
            }
            c.insert(k, pm.clone());
        });
        pm
    });
    let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Low, alpha: 1.0 };
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
    ctx.set_paint_transform(
        Affine::translate(b.origin().to_vec2()) * Affine::scale_non_uniform(b.width() / f64::from(cols), b.height() / f64::from(rows)),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::{Gradient, GradientStop};

    fn ramp_paint(kind: GradientKind) -> GradientPaint {
        let mut g = GradientPaint::new(Gradient::new(
            kind,
            vec![GradientStop::new(0.0, Color::rgb(0.2, 0.2, 0.2)), GradientStop::new(1.0, Color::rgb(0.25, 0.25, 0.25))],
        ));
        g.gradient.dither = true;
        g
    }

    #[test]
    fn linear_ramp_positions() {
        let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(100.0, 0.0), aspect: 1.0, focal: None };
        let r = Ramp::new(GradientKind::Linear, &geom);
        assert!((r.t(Point::new(50.0, 7.0)) - 0.5).abs() < 1e-9);
        assert_eq!(r.t(Point::new(-5.0, 0.0)), 0.0);
        assert_eq!(r.t(Point::new(500.0, 0.0)), 1.0);
    }

    #[test]
    fn radial_ramp_positions_with_and_without_focal() {
        let mut geom = GradientGeom { start: Point::new(50.0, 50.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None };
        let r = Ramp::new(GradientKind::Radial, &geom);
        assert!((r.t(Point::new(75.0, 50.0)) - 0.5).abs() < 1e-9);
        geom.focal = Some(Point::new(30.0, 50.0));
        let r = Ramp::new(GradientKind::Radial, &geom);
        assert!(r.t(Point::new(30.0, 50.0)) < 1e-6, "the focal point is where the first stop sits");
        assert!((r.t(Point::new(100.0, 50.0)) - 1.0).abs() < 1e-6, "the extent circle is the last stop");
    }

    #[test]
    fn dithering_breaks_up_bands() {
        // A ramp across 13 levels over 256 pixels: undithered it is 13 flat bands; dithered,
        // neighbouring pixels within a band differ.
        let g = ramp_paint(GradientKind::Linear);
        let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(256.0, 0.0), aspect: 1.0, focal: None };
        let pm = rasterize(&g, &geom, Rect::new(0.0, 0.0, 256.0, 8.0), 256, 8, Ink::default());
        let row: Vec<u8> = pm.data().iter().take(256).map(|p| p.r).collect();
        let changes = row.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes > 40, "dithered rows alternate between levels ({changes} changes)");
        let mean = |s: &[u8]| s.iter().map(|v| f64::from(*v)).sum::<f64>() / s.len() as f64;
        assert!((mean(&row[..16]) - 0.2 * 255.0).abs() < 1.0 && (mean(&row[240..]) - 0.25 * 255.0).abs() < 1.0, "dither keeps the average colour");
    }
}
