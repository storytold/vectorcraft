//! Paint conversion (solid, gradients, patterns) and image decoding.

use vectorcraft_color::{GradientKind, Paint};
use vectorcraft_doc::resample::{Resample, resized};
use vectorcraft_geom::{Rect, Vec2};
use vello_cpu::RenderContext;
use vello_cpu::peniko::{self, ColorStop};

use super::{BlendMode, Compose, FillRule, Mix, vello_cpu};
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
            // A gradient that repeats or reflects: one run of its stops over its period, which
            // the renderer extends past both ends.
            let spread = g.gradient.is_spread();
            let geom = if spread { g.resolve(bounds).period(g.gradient.kind, g.gradient.repeat_count()) } else { g.resolve(bounds) };
            let extend = match g.gradient.spread {
                _ if !spread => peniko::Extend::Pad,
                vectorcraft_color::GradientSpread::Reflect => peniko::Extend::Reflect,
                _ => peniko::Extend::Repeat,
            };
            let adjusted = f.ink.gradient(&g.gradient);
            let gradient = adjusted.as_ref().unwrap_or(&g.gradient);
            let ink = if adjusted.is_some() { f.ink.base() } else { f.ink };
            let stops: Vec<ColorStop> = gradient.expanded_stops().iter().map(|(o, c, a)| ColorStop::from((*o, ink.color(c, *a)))).collect();
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
                    // One turn about the origin, carried onto the extent ellipse: its frame starts
                    // the sweep along the vector, and a mirrored frame reverses it. Repeated `n`
                    // times, one run takes 1/n of the turn.
                    ctx.set_paint_transform(geom.conic_frame());
                    let turn = if spread { std::f32::consts::TAU / g.gradient.repeat_count() as f32 } else { std::f32::consts::TAU };
                    peniko::Gradient::new_sweep((0.0, 0.0), 0.0, turn).with_stops(stops.as_slice())
                }
                // peniko has no diamond gradient: it is drawn as pixels, as a dithered fill is.
                GradientKind::Diamond => return crate::dither::set_dithered_paint(ctx, g, bounds, bounds, f.ink, false),
                _ => {
                    ctx.reset_paint_transform();
                    let (s, e) =
                        if geom.start.distance(geom.end) < 1e-9 { (geom.start, geom.start + Vec2::new(1.0, 0.0)) } else { (geom.start, geom.end) };
                    peniko::Gradient::new_linear(s, e).with_stops(stops.as_slice())
                }
            };
            // Colour and opacity blend each on its own, as the model samples them and as the PDF
            // and SVG exports draw them (peniko's default premultiplies).
            ctx.set_paint(grad.with_extend(extend).with_interpolation_alpha_space(peniko::InterpolationAlphaSpace::Unpremultiplied));
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
            crate::dither::set_dithered_paint(ctx, g, bounds, bounds, f.ink, true)
        }
        _ => set_paint(ctx, p, bounds, f),
    }
}

/// [`set_paint`] for a stroke whose outline (miters, caps and arrowheads included) fits the box
/// `area` gives: a diamond gradient, drawn as pixels, is sampled over that box too, so the parts
/// of the outline past `bounds` get the diamond's colors there. `area` runs only for a diamond.
pub(crate) fn set_stroke_paint(ctx: &mut RenderContext, p: &Paint, bounds: Rect, area: impl FnOnce() -> Rect, f: &Frame) -> bool {
    match p {
        Paint::Gradient(g) if g.gradient.kind == GradientKind::Diamond && !g.gradient.stops.is_empty() => {
            crate::dither::set_dithered_paint(ctx, g, bounds, area(), f.ink, false)
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
    /// Images decoded, or tried, on this thread ([`image_decodes`]).
    static DECODES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// The decode allowance of one image on this thread in place of [`MAX_DECODE_ALLOC`], so a
    /// test can draw an image past it without decoding gigabytes.
    pub(crate) static TEST_ALLOWANCE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// The bytes [`fits_beside_pinned`] finds room for on this thread, in place of what the images
    /// paints hold in the shared store leave.
    pub(crate) static TEST_ROOM: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// The images [`decode_or_reduction`] has tried to decode on this thread.
    pub(crate) static DECODE_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The most memory decoding one image may take on this thread: [`MAX_DECODE_ALLOC`].
fn allowance() -> u64 {
    #[cfg(test)]
    if let Some(a) = TEST_ALLOWANCE.with(std::cell::Cell::get) {
        return a;
    }
    MAX_DECODE_ALLOC
}

/// Why an image has no pixels to draw ([`decode_pixmap_or_why`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Undecoded {
    /// Its pixels can't be read, at their own size or at [`reduced_size`]: decoding them again
    /// fails again.
    Unreadable,
    /// The images being painted leave no room for it within twice [`MAX_DECODE_ALLOC`]: it may
    /// be decoded once they are done.
    NoRoom,
}

/// How many images have failed to decode on this thread so far: an export compares the count
/// before and after it draws to say it left images out. Images that other threads (another export,
/// the canvas, thumbnails) fail to decode meanwhile are not counted.
pub fn decode_failures() -> u64 {
    DECODE_FAILURES.with(std::cell::Cell::get)
}

/// How many images [`decode_pixmap_or_why`] and [`decode_pixmap_fit`] have decoded on this thread so
/// far, failed decodes included. Images other threads decode are not counted.
pub fn image_decodes() -> u64 {
    DECODES.with(std::cell::Cell::get)
}

/// [`decode_in_room`]'s image or error.
#[cfg(test)]
pub(crate) fn decode_within_allowance(bytes: &[u8]) -> Result<image::DynamicImage, DecodeError> {
    decode_in_room(bytes).map_err(|(e, _)| e)
}

/// Why [`decode_in_room`] gave no image.
#[derive(Debug)]
pub(crate) enum DecodeError {
    /// The image takes more than the decode allowance, or than the room the images being painted
    /// leave: it may be read at [`reduced_size`].
    Limit(String),
    /// Its pixels can't be read.
    Unreadable(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Limit(why) => write!(f, "the image is too large to decode: {why}"),
            DecodeError::Unreadable(why) => write!(f, "the image's pixels can't be read: {why}"),
        }
    }
}

/// The formats the renderer draws, told from their bytes: those the image crate read for it
/// before craft-image-codecs did (#778). TGA has no signature to tell it by and was never drawn.
fn drawn_format(bytes: &[u8]) -> Option<craft_image_codecs::Format> {
    use craft_image_codecs::Format as F;
    craft_image_codecs::detect(bytes).filter(|f| matches!(f, F::Png | F::Jpeg | F::Gif | F::WebP | F::Tiff | F::Bmp))
}

/// `bytes` decoded as `format` within `allowance` bytes: the pixels as stored, not turned upright,
/// as the image crate gave them.
fn decode_as(format: craft_image_codecs::Format, bytes: &[u8], allowance: u64) -> Result<image::DynamicImage, craft_image_codecs::CodecError> {
    vectorcraft_doc::decode::decode_image(format, bytes, allowance).map(|(img, _)| img)
}

/// `bytes` decoded within [`MAX_DECODE_ALLOC`], counted as RGBA: the image's size is read from its
/// header first, and an image of more than that as RGBA gets a limit error before anything is
/// decoded (#1717). A failure comes with why the image gave no pixels ([`room_for`]).
fn decode_in_room(bytes: &[u8]) -> Result<image::DynamicImage, (DecodeError, Undecoded)> {
    let allowance = allowance();
    let unreadable = |e: String| (DecodeError::Unreadable(e), Undecoded::Unreadable);
    let format = drawn_format(bytes).ok_or_else(|| unreadable("not an image the renderer draws".into()))?;
    let (w, h) = match vectorcraft_doc::header_size(bytes) {
        Some(size) => size,
        None => {
            return Err(match craft_image_codecs::probe_size(bytes) {
                Err(craft_image_codecs::CodecError::LimitExceeded(e)) => (DecodeError::Limit(e), Undecoded::Unreadable),
                Err(e) => unreadable(e.to_string()),
                Ok(_) => unreadable("the image's size can't be read".into()),
            });
        }
    };
    let rgba = (u64::from(w) * u64::from(h)).saturating_mul(4);
    let pinned = crate::images::store().pinned_bytes() as u64;
    if let Err(why) = room_for(rgba, pinned) {
        return Err((DecodeError::Limit(format!("{w} × {h} pixels take {rgba} bytes as RGBA")), why));
    }
    // Within the allowance as RGBA and as decoded, a limit the decoder meets is a file that can't
    // hold the pixels its header declares: they can't be read.
    let samples_fit = vectorcraft_doc::decode::decoded_layout(bytes, format)
        .is_some_and(|(_, _, per_pixel)| (u64::from(w) * u64::from(h)).saturating_mul(per_pixel) <= allowance);
    decode_as(format, bytes, allowance).map_err(|e| match e {
        craft_image_codecs::CodecError::LimitExceeded(m) if !samples_fit => (DecodeError::Limit(m), Undecoded::Unreadable),
        e => unreadable(e.to_string()),
    })
}

/// Whether an image of `rgba` bytes may be decoded while the images being painted hold `pinned`
/// bytes: within the allowance itself, and within twice [`MAX_DECODE_ALLOC`] with them. An image
/// over the allowance may still be decoded at [`reduced_size`].
fn room_for(rgba: u64, pinned: u64) -> Result<(), Undecoded> {
    if rgba > allowance() {
        Err(Undecoded::Unreadable)
    } else if rgba.saturating_add(pinned) > 2 * MAX_DECODE_ALLOC {
        Err(Undecoded::NoRoom)
    } else {
        Ok(())
    }
}

/// Whether `w`×`h` pixels may be decoded whole ([`room_for`]) beside the images paints hold now.
pub(crate) fn within_allowance(w: u32, h: u32) -> bool {
    let bytes = (u64::from(w) * u64::from(h)).saturating_mul(4);
    room_for(bytes, crate::images::store().pinned_bytes() as u64).is_ok()
}

/// Whether `bytes` of decoded pixels fit beside the images paints hold, which can't be evicted from
/// the cache: a new image is decoded only while those and it fit twice [`MAX_DECODE_ALLOC`].
fn fits_beside_pinned(bytes: u64) -> bool {
    #[cfg(test)]
    if let Some(room) = TEST_ROOM.with(std::cell::Cell::get) {
        return bytes <= room;
    }
    let pinned = crate::images::store().pinned_bytes() as u64;
    bytes.saturating_add(pinned) <= 2 * MAX_DECODE_ALLOC
}

/// The size at which [`decode_pixmap_or_why`] decodes encoded image `bytes` that take more than
/// [`MAX_DECODE_ALLOC`] as RGBA, or whose decoded samples do (16 bits a sample): the largest that
/// fits it as RGBA, keeping its proportions, at most the image's own size, with sides of at most
/// `u16::MAX` pixels. `None` for an image within the allowance and for one that isn't decoded at
/// all: an image that isn't a PNG, JPEG, GIF or WebP; an image of more pixels than the allowance
/// has bytes (2,147,483,648 pixels, or 536,870,912 in the web app; 46,341 × 46,341 is more); and an
/// image whose decoded samples take more than the allowance, unless it is a PNG that isn't
/// interlaced whose reader's buffers ([`png_row_scratch`]) fit the allowance (read a row at a time).
pub fn reduced_size(bytes: &[u8]) -> Option<(u32, u32)> {
    reduction(bytes, allowance()).map(|r| r.to)
}

/// How [`decode_pixmap_or_why`] reads an image past the decode allowance ([`reduced_size`]).
struct Reduction {
    /// The image's size in pixels.
    size: (u32, u32),
    /// The size it is decoded at.
    to: (u32, u32),
    /// Read a row at a time (a PNG that isn't interlaced) rather than decoded whole first.
    rows: bool,
    /// The bytes reading the image takes besides the result: the PNG reader's row buffers, or the
    /// image decoded whole and a band of its rows as RGBA.
    scratch: u64,
}

/// [`reduced_size`]'s reading of `bytes` within `allowance`.
fn reduction(bytes: &[u8], allowance: u64) -> Option<Reduction> {
    use craft_image_codecs::Format as F;
    let format = drawn_format(bytes).filter(|f| matches!(f, F::Png | F::Jpeg | F::Gif | F::WebP))?;
    let (w, h, per_pixel) = vectorcraft_doc::decode::decoded_layout(bytes, format)?;
    let pixels = u64::from(w) * u64::from(h);
    let total_bytes = pixels.saturating_mul(per_pixel);
    // An image of more pixels than the allowance has bytes (four times what it holds as RGBA) is
    // not read.
    if pixels == 0 || pixels > allowance || (pixels.saturating_mul(4) <= allowance && total_bytes <= allowance) {
        return None;
    }
    let to = fitted_dims(w, h, allowance)?;
    // A PNG that isn't interlaced or animated (and upright in pre-pixel EXIF) is read a row at a time when the reader's buffers fit the allowance.
    let row_scratch = if format == F::Png {
        let streamed = craft_image_codecs::PngDecodeRows::new_streaming(bytes, &png_streaming_limits(allowance))
            .ok()
            .filter(|dec| dec.streamable())
            .and_then(|dec| png_row_scratch(&dec.header()));
        streamed.or_else(|| png_row_scratch_from_header(bytes, w, h, total_bytes)).filter(|&s| s <= allowance)
    } else {
        None
    };
    let rows = row_scratch.is_some();
    let scratch = row_scratch.unwrap_or_else(|| {
        let band = u64::from(BAND) * u64::from(w) * (per_pixel + 4);
        total_bytes.saturating_add(band)
    });
    (rows || total_bytes <= allowance).then_some(Reduction { size: (w, h), to, rows, scratch })
}

/// Streaming retains rows, not the declared full image. Preserve the renderer's source-pixel
/// allowance even when RGBA16's declared samples exceed the shared whole-image default budget.
/// Row scratch and the reduced pixmap are admitted separately before decoding starts.
fn png_streaming_limits(source_pixels: u64) -> craft_image_codecs::Limits {
    craft_image_codecs::Limits { max_width: u32::MAX, max_height: u32::MAX, max_pixels: source_pixels, max_alloc: source_pixels.saturating_mul(8) }
}

/// How many rows of an image decoded whole [`reduced_pixmap`] converts to RGBA at a time.
const BAND: u32 = 16;

/// The bytes of rows already read that png 0.18's reader keeps in its stream buffer, at least,
/// before it moves the rest of the buffer back to the start.
const PNG_KEPT_ROWS: u64 = 128 << 10;

/// The deflate window png 0.18's reader keeps after the rows it has unfiltered, and the step its
/// stream buffer grows by.
const PNG_WINDOW_AND_STEP: u64 = (32 << 10) + (8 << 10);

/// The most memory `craft-image-codecs`'s PNG row decoder takes in its buffers to read its image
/// a row at a time: the stream buffer holds the rows already read up to [`PNG_KEPT_ROWS`] or four
/// rows, whichever is more, the previous and current rows, and [`PNG_WINDOW_AND_STEP`]. The buffer
/// grows by doubling its reservation, and while it grows its old reservation and the new one, up
/// to twice the size it holds, are both allocated: three times that size at most. A copy of a
/// stored row and the row given out come on top. `None` when a row given out doesn't fit memory.
fn png_row_scratch(hdr: &craft_image_codecs::PngHeader) -> Option<u64> {
    let out = u64::from(hdr.width).checked_mul(hdr.layout.channels() as u64)?.checked_mul(hdr.sample.bytes() as u64)?;
    png_row_scratch_for_row(out)
}

/// The row decoder rejects a truncated PNG's tiny IDAT before it can report its header. For a
/// size estimate, the image decoder still gives us the declared output bytes; use those only when
/// the PNG header says it is a static, non-interlaced image, the format the renderer can stream.
fn png_row_scratch_from_header(bytes: &[u8], width: u32, height: u32, total_bytes: u64) -> Option<u64> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.get(..8)? != SIG || height == 0 || !total_bytes.is_multiple_of(u64::from(height)) {
        return None;
    }
    let mut at = 8usize;
    let mut header = false;
    loop {
        let len = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        let data_at = at.checked_add(8)?;
        let end = data_at.checked_add(len)?;
        let data = bytes.get(data_at..end)?;
        match kind {
            b"IHDR" if len == 13 && !header => {
                header = true;
                if at != 8
                    || u32::from_be_bytes(data[..4].try_into().ok()?) != width
                    || u32::from_be_bytes(data[4..8].try_into().ok()?) != height
                    || data[12] != 0
                {
                    return None;
                }
            }
            b"acTL" | b"eXIf" => return None,
            b"IDAT" if header => {
                let row = total_bytes / u64::from(height);
                return row.checked_add(1).and_then(png_row_scratch_for_row);
            }
            _ => {}
        }
        at = end.checked_add(4)?; // CRC
    }
}

fn png_row_scratch_for_row(out: u64) -> Option<u64> {
    // A filter byte and the row's samples.
    let stored = 1u64.checked_add(out)?;
    let kept = (stored * 4).next_multiple_of(64).max(PNG_KEPT_ROWS);
    let stream = kept + stored * 2 + PNG_WINDOW_AND_STEP;
    Some(stream * 3 + stored + out)
}

/// The largest size of at most `w`×`h` pixels, keeping their proportions, whose pixels take at most
/// `allowance` bytes as RGBA, with sides of at most `u16::MAX` pixels.
fn fitted_dims(w: u32, h: u32, allowance: u64) -> Option<(u32, u32)> {
    let pixels = u64::from(w) * u64::from(h);
    let s = ((allowance / 4) as f64 / pixels as f64).sqrt().min(u16::MAX as f64 / w.max(h) as f64).min(1.0);
    let side = |n: u32| ((n as f64 * s).floor() as u32).clamp(1, u16::MAX as u32);
    let (mut nw, mut nh) = (side(w), side(h));
    // Rounding can leave the product a pixel over: take a pixel off the longer side.
    while u64::from(nw) * u64::from(nh) * 4 > allowance && (nw > 1 || nh > 1) {
        if nw >= nh { nw -= 1 } else { nh -= 1 }
    }
    (u64::from(nw) * u64::from(nh) * 4 <= allowance).then_some((nw, nh))
}

/// `bytes` decoded at the size [`reduction`] gives. Each pixel of the result is the mean of the
/// image's pixels it covers. [`Undecoded::Unreadable`] when they can't be read, and
/// [`Undecoded::NoRoom`] when the result and what reading takes ([`Reduction::scratch`]) don't fit
/// beside the images paints hold.
fn reduced_pixmap(bytes: &[u8], r: &Reduction) -> Result<vello_cpu::Pixmap, Undecoded> {
    let (tw, th) = r.to;
    if !fits_beside_pinned((u64::from(tw) * u64::from(th) * 4).saturating_add(r.scratch)) {
        return Err(Undecoded::NoRoom);
    }
    shrunk(bytes, r).ok_or(Undecoded::Unreadable)
}

/// `bytes` read and shrunk as [`reduced_pixmap`] says.
fn shrunk(bytes: &[u8], r: &Reduction) -> Option<vello_cpu::Pixmap> {
    shrunk_with_png_limits(bytes, r, &png_streaming_limits(allowance()))
}

fn shrunk_with_png_limits(bytes: &[u8], r: &Reduction, png_limits: &craft_image_codecs::Limits) -> Option<vello_cpu::Pixmap> {
    let mut shrink = Shrink::new(r.size, r.to);
    if r.rows {
        use craft_image_codecs::{ChannelLayout, CodecError, Format, PngDecodeRows, SampleType};
        let mut dec = PngDecodeRows::new_streaming(bytes, png_limits).ok()?;
        if !dec.streamable() {
            return None;
        }
        let hdr = dec.header();
        // Yield after each row and stop once all pixels have arrived. The pinned decoder's
        // first tail batch only locates a text chunk; later batches inflate/retain its value.
        // Never run those batches: the renderer needs pixels, not ancillary text. Its underlying
        // PNG reader also ignores text chunks before IDAT, so neither location retains text.
        for _ in 0..hdr.height {
            let _extras = dec
                .step_stream::<CodecError>(1, &mut || false, &mut |_y, row| {
                    match (hdr.layout, hdr.sample) {
                        (ChannelLayout::Gray, SampleType::U8) => shrink.row(row.iter().map(|&g| [g, g, g, 255])),
                        (ChannelLayout::GrayA, SampleType::U8) => shrink.row(row.as_chunks::<2>().0.iter().map(|&[g, a]| [g, g, g, a])),
                        (ChannelLayout::Rgb, SampleType::U8) => shrink.row(row.as_chunks::<3>().0.iter().map(|&[r, g, b]| [r, g, b, 255])),
                        (ChannelLayout::Rgba, SampleType::U8) => shrink.row(row.as_chunks::<4>().0.iter().copied()),
                        (ChannelLayout::Gray, SampleType::U16) => shrink.row(row.as_chunks::<2>().0.iter().map(|&g| {
                            let v = (u16::from_ne_bytes(g) >> 8) as u8;
                            [v, v, v, 255]
                        })),
                        (ChannelLayout::GrayA, SampleType::U16) => shrink.row(row.as_chunks::<4>().0.iter().map(|&[g0, g1, a0, a1]| {
                            let g = (u16::from_ne_bytes([g0, g1]) >> 8) as u8;
                            let a = (u16::from_ne_bytes([a0, a1]) >> 8) as u8;
                            [g, g, g, a]
                        })),
                        (ChannelLayout::Rgb, SampleType::U16) => shrink.row(row.as_chunks::<6>().0.iter().map(|&[r0, r1, g0, g1, b0, b1]| {
                            [
                                (u16::from_ne_bytes([r0, r1]) >> 8) as u8,
                                (u16::from_ne_bytes([g0, g1]) >> 8) as u8,
                                (u16::from_ne_bytes([b0, b1]) >> 8) as u8,
                                255,
                            ]
                        })),
                        (ChannelLayout::Rgba, SampleType::U16) => {
                            shrink.row(row.as_chunks::<8>().0.iter().map(|&[r0, r1, g0, g1, b0, b1, a0, a1]| {
                                [
                                    (u16::from_ne_bytes([r0, r1]) >> 8) as u8,
                                    (u16::from_ne_bytes([g0, g1]) >> 8) as u8,
                                    (u16::from_ne_bytes([b0, b1]) >> 8) as u8,
                                    (u16::from_ne_bytes([a0, a1]) >> 8) as u8,
                                ]
                            }))
                        }
                        _ => {
                            return Err(CodecError::Unsupported { format: Format::Png, reason: "unsupported PNG channel layout".into() });
                        }
                    }
                    Ok(())
                })
                .ok()?;
        }
    } else {
        let img = decode_as(drawn_format(bytes)?, bytes, allowance()).ok()?;
        let (w, h) = (img.width(), img.height());
        for y in (0..h).step_by(BAND as usize) {
            let band = img.crop_imm(0, y, w, BAND.min(h - y)).to_rgba8();
            band.as_raw().as_chunks::<4>().0.chunks_exact(w as usize).for_each(|row| shrink.row(row.iter().copied()));
        }
    }
    shrink.finish()
}

/// An image shrunk as its RGBA rows are given ([`Shrink::row`]): each pixel of the result is the
/// mean of the image's pixels it covers, premultiplied.
struct Shrink {
    /// The image's size.
    size: (u32, u32),
    /// The result's size.
    to: (u32, u32),
    /// The sums of the result row being made, a pixel each: red, green and blue times alpha, and
    /// alpha.
    sums: Vec<[u64; 4]>,
    /// How many of the image's pixels each pixel of that row has.
    counts: Vec<u64>,
    /// The result row being made, and the next row of the image.
    at: (u32, u32),
    /// The finished rows.
    out: Vec<vello_cpu::color::PremulRgba8>,
}

impl Shrink {
    fn new(size: (u32, u32), to: (u32, u32)) -> Self {
        let n = to.0 as usize;
        Self { size, to, sums: vec![[0; 4]; n], counts: vec![0; n], at: (0, 0), out: Vec::with_capacity(n * to.1 as usize) }
    }

    /// Add the image's next row of RGBA pixels (pixels past its width are left out).
    fn row(&mut self, rgba: impl IntoIterator<Item = [u8; 4]>) {
        if self.at.1 >= self.size.1 {
            return;
        }
        let ty = (u64::from(self.at.1) * u64::from(self.to.1) / u64::from(self.size.1)) as u32;
        if ty != self.at.0 {
            self.flush();
            self.at.0 = ty;
        }
        let (w, tw) = (u64::from(self.size.0), u64::from(self.to.0));
        for (x, p) in (0..w).zip(rgba) {
            // `tx` is the result column that the image's column `x` falls in.
            let tx = (x * tw / w) as usize;
            let a = u64::from(p[3]);
            if let Some(s) = self.sums.get_mut(tx) {
                s[0] += u64::from(p[0]) * a;
                s[1] += u64::from(p[1]) * a;
                s[2] += u64::from(p[2]) * a;
                s[3] += a;
            }
            if let Some(c) = self.counts.get_mut(tx) {
                *c += 1;
            }
        }
        self.at.1 += 1;
    }

    /// Finish the result row being made.
    fn flush(&mut self) {
        for (s, n) in self.sums.iter_mut().zip(&mut self.counts) {
            let k = (*n).max(1);
            let mean = |v: u64, d: u64| ((v + d / 2) / d).min(255) as u8;
            self.out.push(vello_cpu::color::PremulRgba8 { r: mean(s[0], 255 * k), g: mean(s[1], 255 * k), b: mean(s[2], 255 * k), a: mean(s[3], k) });
            *s = [0; 4];
            *n = 0;
        }
    }

    /// The shrunk image once every row of the image was given.
    fn finish(mut self) -> Option<vello_cpu::Pixmap> {
        if self.at.1 < self.size.1 {
            return None;
        }
        self.flush();
        let (w, h) = (u16::try_from(self.to.0).ok()?, u16::try_from(self.to.1).ok()?);
        (self.out.len() == w as usize * h as usize).then(|| vello_cpu::Pixmap::from_parts(self.out, w, h))
    }
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
    resized(img, nw, nh, Resample::Filter(image::imageops::FilterType::Triangle))
}

/// [`decode_pixmap_or_why`]'s pixels.
#[cfg(test)]
pub fn decode_pixmap(bytes: &[u8]) -> Option<vello_cpu::Pixmap> {
    decode_pixmap_or_why(bytes).ok()
}

/// Decode encoded image bytes into a premultiplied pixmap (shrunk to fit a pixmap's sides), or say
/// why there are no pixels. An image that takes more than the allowance as RGBA is decoded at
/// [`reduced_size`] when it has one; a failure is counted in [`decode_failures`].
pub(crate) fn decode_pixmap_or_why(bytes: &[u8]) -> Result<vello_cpu::Pixmap, Undecoded> {
    counted(match decode_or_reduction(bytes) {
        Ok(img) => decoded_pixmap(img).ok_or(Undecoded::Unreadable),
        Err(r) => r.and_then(|r| reduced_pixmap(bytes, &r)),
    })
}

/// `bytes` decoded within the allowance ([`decode_in_room`]), or, when it refuses them for their
/// size, how [`reduced_pixmap`] reads them past it; otherwise why there is no image.
fn decode_or_reduction(bytes: &[u8]) -> Result<image::DynamicImage, Result<Reduction, Undecoded>> {
    #[cfg(test)]
    DECODE_ATTEMPTS.with(|n| n.set(n.get() + 1));
    DECODES.with(|n| n.set(n.get() + 1));
    match decode_in_room(bytes) {
        Ok(img) => Ok(img),
        Err((DecodeError::Limit(_), Undecoded::Unreadable)) => Err(reduction(bytes, allowance()).ok_or(Undecoded::Unreadable)),
        Err((_, why)) => Err(Err(why)),
    }
}

/// `pm`, with a failure counted in [`decode_failures`] when there is none.
fn counted(pm: Result<vello_cpu::Pixmap, Undecoded>) -> Result<vello_cpu::Pixmap, Undecoded> {
    if pm.is_err() {
        add_decode_failures(1);
    }
    pm
}

/// [`decode_pixmap_or_why`] shrunk to `w`×`h` pixels when the image is larger (never enlarged). An
/// image past the decode allowance is decoded straight at `w`×`h` when that is no larger on either
/// side and fits the allowance, otherwise at [`reduced_size`].
pub fn decode_pixmap_fit(bytes: &[u8], w: u16, h: u16) -> Option<vello_cpu::Pixmap> {
    let img = match decode_or_reduction(bytes) {
        Ok(img) => img,
        Err(r) => {
            return counted(r.and_then(|mut r| {
                let fit = (u32::from(w).max(1), u32::from(h).max(1));
                if fit.0 <= r.to.0 && fit.1 <= r.to.1 {
                    r.to = fit;
                }
                reduced_pixmap(bytes, &r)
            }))
            .ok();
        }
    };
    let img = if img.width() > w as u32 || img.height() > h as u32 {
        resized(img, w as u32, h as u32, Resample::Filter(image::imageops::FilterType::Triangle))
    } else {
        img
    };
    pixmap_of(img.to_rgba8())
}

/// Count `n` images that couldn't be decoded on this thread ([`decode_failures`]).
/// [`decode_pixmap_or_why`] and [`decode_pixmap_fit`] count each failure, and a raster kept in a cache
/// counts the failures it was drawn with again each time it is drawn.
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

/// Decoded image `img` as [`decode_pixmap_or_why`] makes it a pixmap.
pub(crate) fn decoded_pixmap(img: image::DynamicImage) -> Option<vello_cpu::Pixmap> {
    pixmap_of(fit_pixmap(img).to_rgba8())
}

/// The substitution highlights' pink: Preferences › Highlight Substituted Fonts and Highlight
/// Alternate Glyphs, and Document Setup's Highlight Substituted Glyphs (screen only).
pub(crate) const SUBSTITUTED: peniko::Color = peniko::Color::from_rgba8(255, 120, 190, 110);

pub(crate) fn fill_rule(r: FillRule) -> peniko::Fill {
    match r {
        FillRule::NonZero => peniko::Fill::NonZero,
        FillRule::EvenOdd => peniko::Fill::EvenOdd,
    }
}

pub(crate) fn blend_mode(b: vectorcraft_color::BlendMode) -> BlendMode {
    use vectorcraft_color::BlendMode as B;
    let mix = match b {
        B::Normal => Mix::Normal,
        B::Darken => Mix::Darken,
        B::Multiply => Mix::Multiply,
        B::ColorBurn => Mix::ColorBurn,
        B::Lighten => Mix::Lighten,
        B::Screen => Mix::Screen,
        B::ColorDodge => Mix::ColorDodge,
        B::Overlay => Mix::Overlay,
        B::SoftLight => Mix::SoftLight,
        B::HardLight => Mix::HardLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
    };
    BlendMode::new(mix, Compose::SrcOver)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image over the allowance as RGBA never fits, so it is noted as undecodable; one that only
    /// the images being painted leave no room for is not, and is tried again later.
    #[test]
    fn only_the_room_painted_images_take_is_tried_again() {
        assert_eq!(room_for(MAX_DECODE_ALLOC + 1, 0), Err(Undecoded::Unreadable));
        assert_eq!(room_for(MAX_DECODE_ALLOC + 1, 2 * MAX_DECODE_ALLOC), Err(Undecoded::Unreadable));
        assert_eq!(room_for(MAX_DECODE_ALLOC, MAX_DECODE_ALLOC + 1), Err(Undecoded::NoRoom));
        assert_eq!(room_for(MAX_DECODE_ALLOC, MAX_DECODE_ALLOC), Ok(()));
    }

    #[test]
    fn streaming_header_accepts_rgba16_above_whole_image_default() {
        // Header-only fixture: opening the stream checks declared size without allocating or
        // decoding the 1.2 GB canvas. Padding meets the decoder's compressed-size plausibility
        // bound; no rows are requested from this deliberately incomplete image.
        let (w, h) = (10_000u32, 15_000u32);
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let ihdr = [&b"IHDR"[..], &w.to_be_bytes(), &h.to_be_bytes(), &[16, 6, 0, 0, 0]].concat();
        let mut idat = b"IDAT\x78\x9c".to_vec();
        idat.resize(1_200_000, 0);
        for chunk in [&ihdr[..], &idat, b"IEND"] {
            bytes.extend((chunk.len() as u32 - 4).to_be_bytes());
            bytes.extend(chunk);
            let mut crc = flate2::Crc::new();
            crc.update(chunk);
            bytes.extend(crc.sum().to_be_bytes());
        }
        let default = craft_image_codecs::Limits { max_alloc: 1 << 30, max_pixels: 150_000_000, ..Default::default() };
        assert!(craft_image_codecs::PngDecodeRows::new_streaming(&bytes, &default).is_err());
        let rows = craft_image_codecs::PngDecodeRows::new_streaming(&bytes, &png_streaming_limits(150_000_000)).unwrap();
        assert_eq!((rows.header().width, rows.header().height), (w, h));
        assert!(rows.streamable());
        assert!(craft_image_codecs::PngDecodeRows::new_streaming(&bytes, &png_streaming_limits(149_999_999)).is_err());
    }

    #[test]
    fn streaming_reduction_does_not_retain_text_after_pixels() {
        let mut bytes = Vec::new();
        striped(8).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let mut chunk = b"tEXtComment\0".to_vec();
        chunk.extend(std::iter::repeat_n(b'x', 1 << 20));
        // PNG's IEEE CRC over type and payload, kept local to this fixture.
        let mut crc = u32::MAX;
        for &byte in &chunk {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        let mut tail = ((chunk.len() - 4) as u32).to_be_bytes().to_vec();
        tail.extend(chunk);
        tail.extend((!crc).to_be_bytes());
        bytes.splice(bytes.len() - 12..bytes.len() - 12, tail);
        let limits = craft_image_codecs::Limits { max_alloc: 1 << 19, ..Default::default() };
        let mut full = craft_image_codecs::PngDecodeRows::new_streaming(&bytes, &limits).unwrap();
        assert!(full.step_stream::<craft_image_codecs::CodecError>(1, &mut || true, &mut |_, _| Ok(())).is_err());
        // The renderer stops after the pixels instead of retaining ancillary text.
        let reduced = shrunk_with_png_limits(&bytes, &Reduction { size: (8, 1), to: (4, 1), rows: true, scratch: 0 }, &limits).unwrap();
        assert_eq!((reduced.width(), reduced.height()), (4, 1));
        assert!(red_at_its_coverage(&reduced));
    }

    /// The renderer's decoding through craft-image-codecs gives the pixels the image crate gave it
    /// (#778): the same layout and samples for PNGs of each color type and depth, GIF, WebP, TIFF and
    /// BMP, and JPEG samples within two steps (the two decoders' IDCTs round differently). The EXIF
    /// orientation is left to the caller, as before.
    #[test]
    fn decoded_pixels_are_those_the_image_crate_gave() {
        use image::{DynamicImage as D, ImageFormat as F};
        let rgba = image::RgbaImage::from_fn(13, 7, |x, y| image::Rgba([(x * 19) as u8, (y * 37) as u8, ((x + y) * 11) as u8, (x * y * 5) as u8]));
        let rgba16 = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(13, 7, |x, y| {
            image::Rgba([(x * 4_999) as u16, (y * 9_001) as u16, 300, 65_535 - (x * y) as u16])
        });
        let images = [
            D::ImageRgba8(rgba.clone()),
            D::ImageRgb8(D::ImageRgba8(rgba.clone()).to_rgb8()),
            D::ImageLuma8(D::ImageRgba8(rgba.clone()).to_luma8()),
            D::ImageLumaA8(D::ImageRgba8(rgba.clone()).to_luma_alpha8()),
            D::ImageRgba16(rgba16.clone()),
            D::ImageRgb16(D::ImageRgba16(rgba16.clone()).to_rgb16()),
            D::ImageLuma16(D::ImageRgba16(rgba16.clone()).to_luma16()),
            D::ImageLumaA16(D::ImageRgba16(rgba16).to_luma_alpha16()),
        ];
        let encoded = |img: &D, f: F| {
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), f).ok().map(|()| out)
        };
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for img in &images {
            for f in [F::Png, F::Tiff, F::Bmp, F::Gif, F::WebP, F::Jpeg] {
                if let Some(bytes) = encoded(img, f) {
                    files.push((format!("{f:?} {:?}", img.color()), bytes));
                }
            }
        }
        // A palette PNG with a transparent entry and a 1-bit gray PNG, as other writers make them.
        let png = |w: u32, h: u32, depth: u8, color: u8, extra: &[&[u8]], rows: &[u8]| {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut z, rows).unwrap();
            let idat = [&b"IDAT"[..], &z.finish().unwrap()].concat();
            let ihdr = [&b"IHDR"[..], &w.to_be_bytes(), &h.to_be_bytes(), &[depth, color, 0, 0, 0]].concat();
            let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
            for chunk in [&ihdr[..]].into_iter().chain(extra.iter().copied()).chain([&idat[..], b"IEND"]) {
                out.extend((chunk.len() as u32 - 4).to_be_bytes());
                out.extend(chunk);
                let mut crc = flate2::Crc::new();
                crc.update(chunk);
                out.extend(crc.sum().to_be_bytes());
            }
            out
        };
        let plte: &[u8] = b"PLTE\xff\0\0\0\xff\0\0\0\xff";
        files.push(("indexed PNG".into(), png(4, 2, 8, 3, &[plte, b"tRNS\xff\0"], &[0, 0, 1, 2, 1, 0, 2, 1, 0, 0])));
        files.push(("indexed PNG, opaque".into(), png(4, 2, 8, 3, &[plte], &[0, 0, 1, 2, 1, 0, 2, 1, 0, 0])));
        files.push(("1-bit PNG".into(), png(10, 2, 1, 0, &[], &[0, 0b1010_1100, 0b0100_0000, 0, 0b0011_1111, 0b1100_0000])));
        assert!(files.len() > 20, "{}", files.len());
        for (what, bytes) in &files {
            let want = image::load_from_memory(bytes).unwrap();
            let got = decode_in_room(bytes).map_err(|(e, _)| e).unwrap();
            let jpeg = what.starts_with("Jpeg");
            if !jpeg {
                assert_eq!(got.color(), want.color(), "{what}");
            }
            let (g, w) = (got.to_rgba16(), want.to_rgba16());
            assert_eq!(g.dimensions(), w.dimensions(), "{what}");
            let tolerance = if jpeg { 2 * 257 } else { 0 };
            let worst = g.as_raw().iter().zip(w.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= tolerance, "{what}: samples {worst} apart");
            // Read past the allowance at reduced size, the same image is read whole first.
            let whole = shrunk(bytes, &Reduction { size: w.dimensions(), to: w.dimensions(), rows: false, scratch: 0 }).unwrap();
            assert_eq!((u32::from(whole.width()), u32::from(whole.height())), w.dimensions(), "{what}");
        }
    }

    /// Opaque red beside clear black, every other column of a `w` × 1 image.
    fn striped(w: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(w, 1, |x, _| if x % 2 == 0 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) })
    }

    /// Does each pixel of `pm` hold red at its own coverage (premultiplied, within a step)?
    fn red_at_its_coverage(pm: &vello_cpu::Pixmap) -> bool {
        let m = |c: u8, a: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
        pm.data().iter().all(|p| p.r.abs_diff(m(200, p.a)) <= 1 && p.g.abs_diff(m(30, p.a)) <= 1 && p.b.abs_diff(m(30, p.a)) <= 1)
    }

    /// A placed image drawn smaller keeps the color of its opaque pixels at its cut-out edges: the
    /// clear pixels beside them do not darken them.
    #[test]
    fn an_image_drawn_smaller_keeps_its_color_beside_clear_pixels() {
        let mut png = Vec::new();
        striped(8).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let pm = decode_pixmap_fit(&png, 2, 1).unwrap();
        assert_eq!((pm.width(), pm.height()), (2, 1));
        assert!(pm.data().iter().all(|p| p.a > 0));
        assert!(red_at_its_coverage(&pm), "{:?}", pm.data());
    }

    /// An image wider than a pixmap may be is shrunk to fit with its colors weighted by alpha the same way.
    #[test]
    fn an_image_shrunk_to_fit_a_pixmap_keeps_its_color_beside_clear_pixels() {
        let pm = decoded_pixmap(image::DynamicImage::ImageRgba8(striped(u32::from(u16::MAX) + 4465))).unwrap();
        assert_eq!(pm.width(), u16::MAX);
        assert!(red_at_its_coverage(&pm));
    }
}
