//! Dithered gradient fills in the renderer.
//!
//! A dithered linear or radial gradient is drawn as pixels: the gradient sampled on a grid over
//! the painted box with a cell per device pixel or finer (in quarter-octave steps, so a pan reuses
//! the grid and a zoom rarely misses), each colour channel given fine noise of up to a level
//! ([`vectorcraft_color::gradient::dither_noise`]) before it is rounded, and drawn as an image
//! paint that the fill clips. Past [`MAX_CELLS`] a side (a box larger than that on screen), cells
//! grow beyond a pixel and are mixed bilinearly, so the gradient stays smooth. Only the cells the
//! render target can show are sampled ([`window`]), so a large box mostly off-screen costs what
//! its visible part does.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use vectorcraft_color::GradientPaint;
use vectorcraft_color::freeform::painted_box;
use vectorcraft_geom::{Affine, Rect};
use vello_cpu::peniko::{Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext};

use crate::ink::Ink;
use crate::lru::{self, FrameCache};

/// Most cells along a side of a dithered gradient's grid (64 MiB for a square grid).
pub(crate) const MAX_CELLS: f64 = 4096.0;

/// Cells along a side of the part of a grid that is sampled snap out to multiples of this
/// ([`window`]), so a pan within them keeps the sampled part.
const TILE: f64 = 256.0;

thread_local! {
    /// Grids by [`key`], within [`lru::DITHER_BUDGET`], least recently used first out.
    static GRIDS: RefCell<FrameCache<u64, Arc<Pixmap>>> = RefCell::new(FrameCache::new(lru::DITHER_BUDGET, usize::MAX));
}

/// Bytes of the grids this thread keeps, after `f` runs on an empty cache.
#[cfg(test)]
pub(crate) fn grid_bytes_after(f: impl FnOnce()) -> usize {
    GRIDS.with(|c| c.borrow_mut().clear());
    f();
    GRIDS.with(|c| c.borrow().bytes())
}

/// Cells along a side `px` device pixels long: at least one per pixel, rounded up to a
/// quarter-octave step, 1 to [`MAX_CELLS`].
fn cells(px: f64) -> u16 {
    let steps = (4.0 * px.max(1.0).log2()).ceil();
    2f64.powf(steps / 4.0).ceil().clamp(1.0, MAX_CELLS) as u16
}

/// Cache key of gradient `g` placed on box `b` as a `cols` × `rows` grid, cells `win` of it,
/// painted with `ink` (and the active colour settings, which turn colours into display RGB or
/// inks).
fn key(g: &GradientPaint, b: Rect, cols: u16, rows: u16, win: &[Range<u16>; 2], ink: Ink) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let gr = &g.gradient;
    (gr.kind as u8, gr.dither, gr.method as u8).hash(&mut h);
    for s in &gr.stops {
        (s.offset.to_bits(), s.opacity.to_bits(), s.midpoint.to_bits()).hash(&mut h);
        s.color.key().hash(&mut h);
    }
    let geom = g.resolve(b);
    (geom.start.x.to_bits(), geom.start.y.to_bits(), geom.end.x.to_bits(), geom.end.y.to_bits(), geom.aspect.to_bits()).hash(&mut h);
    geom.focal.map(|f| (f.x.to_bits(), f.y.to_bits())).hash(&mut h);
    (b.x0.to_bits(), b.y0.to_bits(), b.x1.to_bits(), b.y1.to_bits(), cols, rows).hash(&mut h);
    win.hash(&mut h);
    (Arc::as_ptr(&vectorcraft_color::cms::active()) as usize, ink).hash(&mut h);
    h.finish()
}

/// The cells of a `cols` × `rows` grid over box `b` that a `w` × `h` render target, which
/// transform `xf` maps the box onto, can show: (columns, rows), a cell more each way for the
/// bilinear mix, out to whole [`TILE`]s. The whole grid where `xf` has no inverse; None where
/// no cell shows.
fn window(xf: Affine, w: u16, h: u16, b: Rect, cols: u16, rows: u16) -> Option<[Range<u16>; 2]> {
    let seen = xf.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
    let span = |lo: f64, hi: f64, start: f64, len: f64, n: u16| {
        let cell = len / f64::from(n);
        let (first, last) = (((lo - start) / cell).floor() - 1.0, ((hi - start) / cell).ceil() + 1.0);
        if !(first.is_finite() && last.is_finite()) {
            return Some(0..n);
        }
        let a = ((first / TILE).floor() * TILE).clamp(0.0, f64::from(n));
        let z = ((last / TILE).ceil() * TILE).clamp(0.0, f64::from(n));
        (a < z).then_some(a as u16..z as u16)
    };
    Some([span(seen.x0, seen.x1, b.x0, b.width(), cols)?, span(seen.y0, seen.y1, b.y0, b.height(), rows)?])
}

/// Cells `win` (columns, rows) of `g` placed on `b` as a `cols` × `rows` grid, as a
/// premultiplied pixmap, colours as `ink` paints them.
pub(crate) fn rasterize(g: &GradientPaint, b: Rect, cols: u16, rows: u16, win: [Range<u16>; 2], ink: Ink) -> Pixmap {
    let (w, h) = (win[0].len() as u16, win[1].len() as u16);
    let adjusted = ink.gradient(&g.gradient).map(|gradient| GradientPaint { gradient, ..g.clone() });
    let ink = if adjusted.is_some() { ink.base() } else { ink };
    let g = adjusted.as_ref().unwrap_or(g);
    let data = g
        .pixels_within(b, b, cols, rows, win, true, &|c| ink.rgb(c))
        .map(|[r, gr, bl, a]| {
            let pm = |v: u8| ((u16::from(v) * u16::from(a) + 127) / 255) as u8;
            vello_cpu::color::PremulRgba8 { r: pm(r), g: pm(gr), b: pm(bl), a }
        })
        .collect();
    Pixmap::from_parts(data, w, h)
}

/// Set dithered gradient `g` on box `bounds` (in the paint's space, which the context's current
/// transform maps to pixels) as the context paint. Returns false for an empty box or one off the
/// render target.
pub(crate) fn set_dithered_paint(ctx: &mut RenderContext, g: &GradientPaint, bounds: Rect, ink: Ink) -> bool {
    let Some(b) = painted_box(bounds) else { return false };
    // Device pixels per unit along the box's sides.
    let [m0, m1, m2, m3, _, _] = ctx.transform().as_coeffs();
    let (sx, sy) = (m0.hypot(m1), m2.hypot(m3));
    let (wpx, hpx) = (b.width() * sx, b.height() * sy);
    if !(wpx.is_finite() && hpx.is_finite()) {
        return false;
    }
    let (cols, rows) = (cells(wpx), cells(hpx));
    let Some(win) = window(*ctx.transform(), ctx.width(), ctx.height(), b, cols, rows) else { return false };
    let (first_col, first_row) = (win[0].start, win[1].start);
    let k = key(g, b, cols, rows, &win, ink);
    let pm = GRIDS
        .with(|c| {
            let mut c = c.borrow_mut();
            let t = c.tick();
            c.get(&k, t).cloned()
        })
        .unwrap_or_else(|| {
            let pm = Arc::new(rasterize(g, b, cols, rows, win, ink));
            GRIDS.with(|c| {
                let mut c = c.borrow_mut();
                let t = c.tick();
                c.insert(k, pm.clone(), usize::from(pm.width()) * usize::from(pm.height()) * 4, t);
            });
            pm
        });
    // A cell per pixel or finer: each pixel takes one cell, its noise intact. Coarser (a box past
    // the grid's largest size on screen): mixed bilinearly, so no cell shows as a block.
    let fine = f64::from(cols) >= wpx && f64::from(rows) >= hpx;
    let quality = if fine { ImageQuality::Low } else { ImageQuality::Medium };
    let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality, alpha: 1.0 };
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
    let (cw, ch) = (b.width() / f64::from(cols), b.height() / f64::from(rows));
    let origin = (b.x0 + f64::from(first_col) * cw, b.y0 + f64::from(first_row) * ch);
    ctx.set_paint_transform(Affine::translate(origin) * Affine::scale_non_uniform(cw, ch));
    true
}
