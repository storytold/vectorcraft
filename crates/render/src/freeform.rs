//! Freeform gradients in the renderer.
//!
//! vello has no freeform shading, so the colour field ([`vectorcraft_color::freeform`]) is
//! sampled on a grid of cells over the painted box and drawn as an image paint (bilinear, edges
//! padded): the fill or stroke being painted clips it. The grid is sized from the box's size on
//! screen (a cell per few device pixels, in power-of-two steps so zooming reuses it) and cached by
//! the gradient's content, so panning and redraws don't resample it.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use vectorcraft_color::freeform::{grid_size, painted_box, spread_scale};
use vectorcraft_color::{Freeform, GradientPaint};
use vectorcraft_geom::{Affine, Rect};
use vello_cpu::peniko::{Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext};

use crate::ink::Ink;
use crate::lru::{self, FrameCache};

thread_local! {
    /// Grids by [`key`], within [`lru::FREEFORM_BUDGET`], least recently used first out.
    static GRIDS: RefCell<FrameCache<u64, Arc<Pixmap>>> = RefCell::new(FrameCache::new(lru::FREEFORM_BUDGET, usize::MAX));
}

/// Cache key of the grid of `f` on box `b` at `cols` × `rows` painted with `ink` (and the active
/// colour settings, which turn colours into display RGB or inks).
fn key(f: &Freeform, b: Rect, cols: u16, rows: u16, ink: Ink) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in &f.points {
        (p.at.x.to_bits(), p.at.y.to_bits(), p.opacity.to_bits(), p.spread.to_bits()).hash(&mut h);
        p.color.key().hash(&mut h);
    }
    f.lines.hash(&mut h);
    (b.x0.to_bits(), b.y0.to_bits(), b.x1.to_bits(), b.y1.to_bits(), cols, rows).hash(&mut h);
    (Arc::as_ptr(&vectorcraft_color::cms::active()) as usize, ink).hash(&mut h);
    h.finish()
}

/// Sample `f` at the centres of a `cols` × `rows` grid over `b` (premultiplied, colours as `ink`
/// paints them).
pub(crate) fn rasterize(f: &Freeform, b: Rect, cols: u16, rows: u16, ink: Ink) -> Pixmap {
    let field = f.field_with(spread_scale(b), &|c| ink.rgb(c));
    let data = field
        .grid(b, cols, rows)
        .map(|([r, g, bl], a)| {
            let a = a.clamp(0.0, 1.0);
            let q = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0).round() as u8;
            vello_cpu::color::PremulRgba8 { r: q(r), g: q(g), b: q(bl), a: (a * 255.0).round() as u8 }
        })
        .collect();
    Pixmap::from_parts(data, cols, rows)
}

/// Set freeform gradient `g` on box `bounds` (in the paint's space, which the context's current
/// transform maps to pixels) as the context paint. Returns false for an empty box.
pub(crate) fn set_freeform_paint(ctx: &mut RenderContext, g: &GradientPaint, bounds: Rect, ink: Ink) -> bool {
    let Some(b) = painted_box(bounds) else { return false };
    let device = b.width().max(b.height()) * ctx.transform().determinant().abs().sqrt();
    let (cols, rows) = grid_size(b, device);
    let f = g.freeform_on(b);
    let k = key(&f, b, cols, rows, ink);
    let pm = GRIDS
        .with(|c| {
            let mut c = c.borrow_mut();
            let t = c.tick();
            c.get(&k, t).cloned()
        })
        .unwrap_or_else(|| {
            let pm = Arc::new(rasterize(&f, b, cols, rows, ink));
            GRIDS.with(|c| {
                let mut c = c.borrow_mut();
                let t = c.tick();
                c.insert(k, pm.clone(), cols as usize * rows as usize * 4, t);
            });
            pm
        });
    let sampler = ImageSampler { x_extend: Extend::Pad, y_extend: Extend::Pad, quality: ImageQuality::Medium, alpha: 1.0 };
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
    ctx.set_paint_transform(Affine::translate(b.origin().to_vec2()) * Affine::scale_non_uniform(b.width() / cols as f64, b.height() / rows as f64));
    true
}
