//! Paint conversion (solid, gradients, patterns) and image decoding.

use vectorcraft_color::{GradientKind, Paint};
use vectorcraft_geom::{Affine, Rect, Vec2};
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
                GradientKind::Conic => {
                    // One turn around the origin, carried onto the extent ellipse (its frame turns
                    // the sweep to start along the vector, and a mirrored frame reverses it). A
                    // vector without length sweeps from the right around its start.
                    let frame = geom.unit_frame().unwrap_or_else(|| Affine::translate(geom.start.to_vec2()));
                    ctx.set_paint_transform(frame);
                    peniko::Gradient::new_sweep((0.0, 0.0), 0.0, std::f32::consts::TAU).with_stops(stops.as_slice())
                }
                _ => {
                    ctx.reset_paint_transform();
                    let (s, e) =
                        if geom.start.distance(geom.end) < 1e-9 { (geom.start, geom.start + Vec2::new(1.0, 0.0)) } else { (geom.start, geom.end) };
                    peniko::Gradient::new_linear(s, e).with_stops(stops.as_slice())
                }
            };
            // Colour and opacity blend each on its own, as the model samples them and as the PDF
            // and SVG exports draw them (peniko's default premultiplies).
            ctx.set_paint(grad.with_interpolation_alpha_space(peniko::InterpolationAlphaSpace::Unpremultiplied));
            true
        }
        Paint::Pattern { pattern, xf } => crate::pattern::set_pattern_paint(ctx, pattern, *xf, f),
    }
}

/// The most memory decoding one image may take: print rasters run to gigabytes (a 300 ppi roll,
/// 6614 x 59055 px, is 1.56 GB as RGBA), past the decoder's 512 MB default. The web keeps it.
const MAX_DECODE_ALLOC: u64 = if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2 << 30 };

/// Images that couldn't be decoded (and so drew nothing), counted for exports to report.
static DECODE_FAILURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many images have failed to decode so far (process-wide): an export compares the count
/// before and after to say it left images out.
pub fn decode_failures() -> u64 {
    DECODE_FAILURES.load(std::sync::atomic::Ordering::Relaxed)
}

/// `bytes` decoded within [`MAX_DECODE_ALLOC`]; a failure is counted in [`decode_failures`].
fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    let img = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok().and_then(|mut r| {
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(MAX_DECODE_ALLOC);
        r.limits(limits);
        r.decode().ok()
    });
    if img.is_none() {
        DECODE_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    img
}

/// `img` shrunk, keeping its proportions, to fit a pixmap's sides (at most `u16::MAX` pixels).
fn fit_pixmap(img: image::DynamicImage) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let max = u16::MAX as u32;
    if w <= max && h <= max {
        return img;
    }
    let s = max as f64 / w.max(h) as f64;
    let (nw, nh) = (((w as f64 * s).round() as u32).clamp(1, max), ((h as f64 * s).round() as u32).clamp(1, max));
    img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
}

/// Decode encoded image bytes into a premultiplied pixmap (shrunk to fit a pixmap's sides).
pub fn decode_pixmap(bytes: &[u8]) -> Option<vello_cpu::Pixmap> {
    pixmap_of(fit_pixmap(decode(bytes)?).to_rgba8())
}

/// [`decode_pixmap`] shrunk to `w`×`h` pixels when the image is larger (never enlarged).
pub fn decode_pixmap_fit(bytes: &[u8], w: u16, h: u16) -> Option<vello_cpu::Pixmap> {
    let img = decode(bytes)?;
    let img = if img.width() > w as u32 || img.height() > h as u32 {
        img.resize_exact(w as u32, h as u32, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    pixmap_of(img.to_rgba8())
}

/// An RGBA image as a premultiplied pixmap.
fn pixmap_of(img: image::RgbaImage) -> Option<vello_cpu::Pixmap> {
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
