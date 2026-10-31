//! Paint conversion (solid, gradients, patterns) and image decoding.

use vectorcraft_color::{GradientKind, Paint};
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
            let geom = g.resolve(bounds);
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
pub(crate) fn decode_within_allowance(bytes: &[u8]) -> image::ImageResult<image::DynamicImage> {
    decode_in_room(bytes).map_err(|(e, _)| e)
}

/// `bytes` decoded within [`MAX_DECODE_ALLOC`], counted as RGBA: the image's size is read from its
/// header first, and an image of more than that as RGBA gets a limit error before anything is
/// decoded (#1717). A failure comes with why the image gave no pixels ([`room_for`]).
fn decode_in_room(bytes: &[u8]) -> Result<image::DynamicImage, (image::ImageError, Undecoded)> {
    let allowance = allowance();
    let unreadable = |e| (e, Undecoded::Unreadable);
    let reader = || -> image::ImageResult<_> {
        let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(allowance);
        r.limits(limits);
        Ok(r)
    };
    let (w, h) = reader().and_then(|r| r.into_dimensions()).map_err(unreadable)?;
    let bytes = (u64::from(w) * u64::from(h)).saturating_mul(4);
    let pinned = crate::images::store().pinned_bytes() as u64;
    if let Err(why) = room_for(bytes, pinned) {
        return Err((image::ImageError::Limits(image::error::LimitError::from_kind(image::error::LimitErrorKind::InsufficientMemory)), why));
    }
    reader().and_then(|r| r.decode()).map_err(unreadable)
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
/// interlaced and is at most the allowance divided by [`ROW_SCRATCH`] pixels wide (read a row at a
/// time).
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
    use image::ImageDecoder as _;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let format = reader.format()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::WebP) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(allowance);
    reader.limits(limits);
    let d = reader.into_decoder().ok()?;
    let (w, h) = d.dimensions();
    let pixels = u64::from(w) * u64::from(h);
    // An image of more pixels than the allowance has bytes (four times what it holds as RGBA) is
    // not read.
    if pixels == 0 || pixels > allowance || (pixels.saturating_mul(4) <= allowance && d.total_bytes() <= allowance) {
        return None;
    }
    let to = fitted_dims(w, h, allowance)?;
    // The PNG reader's row buffers take at most `ROW_SCRATCH` bytes a column, also within the allowance.
    let rows = format == image::ImageFormat::Png
        && u64::from(w).saturating_mul(ROW_SCRATCH) <= allowance
        && png_reader(bytes).is_some_and(|r| !r.info().interlaced);
    let scratch = if rows {
        u64::from(w) * ROW_SCRATCH
    } else {
        let band = u64::from(BAND) * u64::from(w) * (u64::from(d.color_type().bytes_per_pixel()) + 4);
        d.total_bytes().saturating_add(band)
    };
    (rows || d.total_bytes() <= allowance).then_some(Reduction { size: (w, h), to, rows, scratch })
}

/// How many rows of an image decoded whole [`reduced_pixmap`] converts to RGBA at a time.
const BAND: u32 = 16;

/// The most memory a column of a PNG read a row at a time takes in the reader's row buffers: the
/// row as stored (up to 8 bytes a pixel and a filter byte), the row before it, and the row given
/// out (up to 4 bytes a pixel).
const ROW_SCRATCH: u64 = 32;

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

/// A PNG reader of `bytes` that gives 8-bit gray, gray with alpha, RGB or RGBA rows, with its
/// header and the chunks before the pixels read.
fn png_reader(bytes: &[u8]) -> Option<png::Reader<std::io::Cursor<&[u8]>>> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    d.read_info().ok()
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
    let mut shrink = Shrink::new(r.size, r.to);
    if r.rows {
        let mut reader = png_reader(bytes)?;
        let (color, _) = reader.output_color_type();
        while let Some(row) = reader.next_row().ok()? {
            let data = row.data();
            match color {
                png::ColorType::Grayscale => shrink.row(data.iter().map(|&g| [g, g, g, 255])),
                png::ColorType::GrayscaleAlpha => shrink.row(data.as_chunks::<2>().0.iter().map(|&[g, a]| [g, g, g, a])),
                png::ColorType::Rgb => shrink.row(data.as_chunks::<3>().0.iter().map(|&[r, g, b]| [r, g, b, 255])),
                png::ColorType::Rgba => shrink.row(data.as_chunks::<4>().0.iter().copied()),
                png::ColorType::Indexed => return None,
            }
        }
    } else {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(allowance());
        reader.limits(limits);
        let img = reader.decode().ok()?;
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
    img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
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
        Err((image::ImageError::Limits(_), Undecoded::Unreadable)) => Err(reduction(bytes, allowance()).ok_or(Undecoded::Unreadable)),
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
        img.resize_exact(w as u32, h as u32, image::imageops::FilterType::Triangle)
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
}
