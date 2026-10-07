//! Paint conversion (solid, gradients, patterns) and image decoding.

use vectorcraft_color::{GradientKind, Paint};
use vectorcraft_geom::{Rect, Vec2};
use vello_cpu::RenderContext;
use vello_cpu::peniko::{self, ColorStop};

use crate::Frame;

/// Set the context paint (colours as frame `f` paints them). Returns false if nothing should be
/// drawn.
pub(crate) fn set_paint(ctx: &mut RenderContext, p: &Paint, bounds: Rect, f: &Frame) -> bool {
    match p {
        Paint::None => false,
        Paint::Solid { color: c, .. } => {
            ctx.set_paint(f.ink.color(c, 1.0));
            true
        }
        Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => crate::freeform::set_freeform_paint(ctx, g, bounds, f.ink),
        Paint::Gradient(g) if g.gradient.dither => crate::dither::set_dithered_paint(ctx, g, bounds, f.ink),
        Paint::Gradient(g) => {
            let geom = g.resolve(bounds);
            let stops: Vec<ColorStop> = g.gradient.expanded_stops().iter().map(|(o, c, a)| ColorStop::from((*o, f.ink.color(c, *a)))).collect();
            if stops.is_empty() {
                return false;
            }
            let grad = match g.gradient.kind {
                GradientKind::Radial => {
                    let r = geom.length().max(1e-6) as f32;
                    // The aspect ratio squashes the circle across the gradient vector.
                    let squash = geom.radial_squash();
                    ctx.set_paint_transform(squash);
                    let g = match geom.focal {
                        // An off-centre focal point: a two-point radial from it to the extent circle.
                        Some(f) => peniko::Gradient::new_two_point_radial(squash.inverse() * f, 0.0, geom.start, r),
                        None => peniko::Gradient::new_radial(geom.start, r),
                    };
                    g.with_stops(stops.as_slice())
                }
                _ => {
                    ctx.reset_paint_transform();
                    let (s, e) =
                        if geom.start.distance(geom.end) < 1e-9 { (geom.start, geom.start + Vec2::new(1.0, 0.0)) } else { (geom.start, geom.end) };
                    peniko::Gradient::new_linear(s, e).with_stops(stops.as_slice())
                }
            };
            ctx.set_paint(grad);
            true
        }
        Paint::Pattern { pattern, xf } => crate::pattern::set_pattern_paint(ctx, pattern, *xf, f),
    }
}

/// Decode encoded image bytes into a premultiplied pixmap.
pub fn decode_pixmap(bytes: &[u8]) -> Option<vello_cpu::Pixmap> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return None;
    }
    let data: Vec<vello_cpu::color::PremulRgba8> = img
        .pixels()
        .map(|p| {
            let a = p[3] as u16;
            let m = |c: u8| ((c as u16 * a + 127) / 255) as u8;
            vello_cpu::color::PremulRgba8 { r: m(p[0]), g: m(p[1]), b: m(p[2]), a: p[3] }
        })
        .collect();
    Some(vello_cpu::Pixmap::from_parts(data, w as u16, h as u16))
}
