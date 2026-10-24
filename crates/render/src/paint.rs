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
            // Colour and opacity blend each on its own, as the model samples them and as the PDF
            // and SVG exports draw them (peniko's default premultiplies).
            ctx.set_paint(grad.with_interpolation_alpha_space(peniko::InterpolationAlphaSpace::Unpremultiplied));
            true
        }
        Paint::Pattern { pattern, xf } => crate::pattern::set_pattern_paint(ctx, pattern, *xf, f),
    }
}

/// [`set_paint`] for a fill: a dithered gradient (dither applies to fills only) with stops is
/// drawn as pixels with fine noise ([`crate::dither`]).
pub(crate) fn set_fill_paint(ctx: &mut RenderContext, p: &Paint, bounds: Rect, f: &Frame) -> bool {
    match p {
        Paint::Gradient(g) if g.gradient.dither && g.gradient.kind != GradientKind::Freeform && !g.gradient.stops.is_empty() => {
            crate::dither::set_dithered_paint(ctx, g, bounds, f.ink)
        }
        _ => set_paint(ctx, p, bounds, f),
    }
}

/// The most memory decoding one image may take: print rasters run to gigabytes (a 300 ppi roll,
/// 6614 x 59055 px, is 1.56 GB as RGBA), past the decoder's 512 MB default. The web keeps it.
const MAX_DECODE_ALLOC: u64 = if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2 << 30 };

thread_local! {
    /// Images that couldn't be decoded (and so drew nothing) on this thread, counted for exports to
    /// report.
    static DECODE_FAILURES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many images have failed to decode on this thread so far: an export compares the count
/// before and after it draws to say it left images out. Images that other threads (another export,
/// the canvas, thumbnails) fail to decode meanwhile are not counted.
pub fn decode_failures() -> u64 {
    DECODE_FAILURES.with(std::cell::Cell::get)
}

/// `bytes` decoded within [`MAX_DECODE_ALLOC`] ([`decode_within_allowance`]); a failure is counted
/// in [`decode_failures`].
fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    let img = decode_within_allowance(bytes).ok();
    if img.is_none() {
        add_decode_failures(1);
    }
    img
}

/// `bytes` decoded within [`MAX_DECODE_ALLOC`], counted as RGBA: the image's size is read from its
/// header first, and an image of more than that as RGBA gets a limit error before anything is
/// decoded (#1717).
pub(crate) fn decode_within_allowance(bytes: &[u8]) -> image::ImageResult<image::DynamicImage> {
    let reader = || -> image::ImageResult<_> {
        let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(MAX_DECODE_ALLOC);
        r.limits(limits);
        Ok(r)
    };
    let (w, h) = reader()?.into_dimensions()?;
    let bytes = (u64::from(w) * u64::from(h)).saturating_mul(4);
    // The images paints hold can't be evicted from the cache, so a document of many images held
    // them all at once: a new one is decoded only while those and it fit twice the allowance.
    let pinned = crate::images::store().pinned_bytes() as u64;
    if bytes > MAX_DECODE_ALLOC || bytes.saturating_add(pinned) > 2 * MAX_DECODE_ALLOC {
        return Err(image::ImageError::Limits(image::error::LimitError::from_kind(image::error::LimitErrorKind::InsufficientMemory)));
    }
    reader()?.decode()
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

/// Count `n` images that couldn't be decoded on this thread ([`decode_failures`]). [`decode`]
/// counts each failure, and a raster kept in a cache counts the failures it was drawn with again
/// each time it is drawn.
pub(crate) fn add_decode_failures(n: u64) {
    DECODE_FAILURES.with(|c| c.set(c.get() + n));
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
