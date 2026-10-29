//! VectorCraft's macOS Quick Look extensions (#663): the thumbnail Finder shows for a
//! `.vectorcraft` (or `.drawcraft`) file, and the preview the Space bar opens.
//!
//! Two app extensions inside `VectorCraft.app/Contents/PlugIns` (packaging/macos/quicklook):
//! - **Thumbnail** (`com.apple.quicklook.thumbnail`): the PNG a save embeds (File Handling → Embed
//!   Preview, [`vectorcraft_format::preview_checked`]) scaled to the size Finder asks for
//!   ([`thumbnail`]). The document itself is never read, so a thumbnail costs little.
//! - **Preview** (`com.apple.quicklook.preview`, data-based): the first artboard (else the art)
//!   rendered by `vectorcraft-render` at most [`Limits::max_side`] pixels a side and
//!   [`Limits::max_pixels`] in all, within [`Limits::time`]; when that can't be done (too large, too
//!   slow, nothing drawn) the embedded preview stands in ([`preview`]).
//!
//! Everything a file can say is untrusted. A file this version doesn't read (not a native file, a
//! newer version, unpacking past [`THUMBNAIL_CAPS`], tighter than the format's own `MAX_*` caps,
//! larger than [`MAX_FILE`] on disk) or without anything to show is an error, which the extension hands back to
//! Quick Look so Finder shows the document icon. This library is the extensions' whole read path:
//! it builds everywhere (its tests and the engine's import fuzzing run on Linux), while the
//! Objective-C side (`mac`, objc2) builds on macOS only.

// `deny`, not `forbid`: the Objective-C side (`mac`, macOS only) needs `unsafe` and allows it for
// that module alone, as `apps/vectorcraft` does for AppKit. Everything else may not use it.
#![deny(unsafe_code)]

use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use vectorcraft_format::{FormatError, ReadCaps};
use vectorcraft_render::{Renderer, raster_size};

#[cfg(target_os = "macos")]
pub mod mac;

/// Largest file the extensions read (bytes on disk); a larger one keeps the document icon.
pub const MAX_FILE: u64 = 512 << 20;
/// Largest side of a thumbnail in pixels, whatever Finder asks for.
pub const MAX_THUMBNAIL_SIDE: u32 = 2048;
/// Largest side of an embedded preview the extensions decode (VectorCraft writes at most
/// [`vectorcraft_format::PREVIEW_MAX`]; other writers may differ).
pub const MAX_EMBEDDED_SIDE: u32 = 4096;
/// Most bytes decoding an embedded preview may allocate.
const MAX_DECODE_ALLOC: u64 = 64 << 20;
/// Most pixels per point a preview is rendered at (a small artboard isn't blown up further).
const MAX_PREVIEW_SCALE: f64 = 8.0;
/// What reading a compressed file for its embedded preview may unpack: the JSON, where VectorCraft
/// keeps the preview, and the data after it when another writer keeps the preview there. Finder
/// asks for thumbnails of every file it shows, unasked, so these are a quarter of what opening
/// the document may unpack (`vectorcraft_format::MAX_JSON`, 1 GiB); a file past them keeps its
/// icon.
pub const THUMBNAIL_CAPS: ReadCaps = ReadCaps { json: 256 << 20, blobs: 256 << 20 };

/// Why the extensions have nothing to show for a file (Quick Look then shows its icon).
#[derive(Debug, thiserror::Error)]
pub enum QuickLookError {
    #[error("can't read the file: {0}")]
    Read(String),
    #[error("the file is {} MB; Quick Look reads at most {} MB", .0 >> 20, MAX_FILE >> 20)]
    TooLarge(u64),
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error("the file has no preview (saved with Embed Preview off, or by an older VectorCraft)")]
    NoPreview,
    #[error("the embedded preview can't be read: {0}")]
    BadPreview(String),
    #[error("the document shows nothing")]
    Empty,
    #[error("rendering took longer than {0:?}")]
    TimedOut(Duration),
    #[error("rendering failed: {0}")]
    Render(String),
    #[error("previews that ran out of time are still rendering")]
    Busy,
}

pub type Result<T> = std::result::Result<T, QuickLookError>;

/// Error codes the extensions report to Quick Look (`NSError` codes in their own domain).
impl QuickLookError {
    pub fn code(&self) -> isize {
        match self {
            Self::Read(_) => 1,
            Self::TooLarge(_) => 2,
            Self::Format(FormatError::TooNew(_)) => 3,
            Self::Format(FormatError::TooBig) => 4,
            Self::Format(_) => 5,
            Self::NoPreview => 6,
            Self::BadPreview(_) => 7,
            Self::Empty => 8,
            Self::TimedOut(_) => 9,
            Self::Render(_) => 10,
            Self::Busy => 11,
        }
    }
}

/// The file at `path`, refused past `max` bytes (checked on the open file and while reading, so a
/// file that grows meanwhile is still capped).
pub fn read_file(path: &Path, max: u64) -> Result<Vec<u8>> {
    let read_err = |e: std::io::Error| QuickLookError::Read(e.to_string());
    let f = std::fs::File::open(path).map_err(read_err)?;
    let len = f.metadata().map_err(read_err)?.len();
    if len > max {
        return Err(QuickLookError::TooLarge(len));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    f.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(read_err)?;
    let read = bytes.len() as u64;
    if read > max {
        return Err(QuickLookError::TooLarge(read));
    }
    Ok(bytes)
}

/// Straight-alpha RGBA8 pixels, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// `width` × `height` fitted into `max_w` × `max_h` (each capped at [`MAX_THUMBNAIL_SIDE`]),
/// keeping its proportions, at least 1 pixel a side; `None` for an empty size or a bound that
/// isn't a positive number.
pub fn fit(width: u32, height: u32, max_w: f64, max_h: f64) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || !(max_w.is_finite() && max_h.is_finite() && max_w >= 1.0 && max_h >= 1.0) {
        return None;
    }
    let cap = f64::from(MAX_THUMBNAIL_SIDE);
    let (mw, mh) = (max_w.min(cap), max_h.min(cap));
    let k = (mw / f64::from(width)).min(mh / f64::from(height));
    let side = |v: u32, m: f64| (f64::from(v) * k).round().clamp(1.0, m) as u32;
    Some((side(width, mw), side(height, mh)))
}

/// Finder's thumbnail of a native file: its embedded preview scaled to fit `max_w` × `max_h`
/// pixels. The document isn't read.
pub fn thumbnail(bytes: &[u8], max_w: f64, max_h: f64) -> Result<Picture> {
    let png = vectorcraft_format::preview_checked_within(bytes, THUMBNAIL_CAPS)?.ok_or(QuickLookError::NoPreview)?;
    let img = decode_png(&png)?;
    let (w, h) = fit(img.width(), img.height(), max_w, max_h).ok_or(QuickLookError::Empty)?;
    let img = if (w, h) == img.dimensions() { img } else { image::imageops::resize(&img, w, h, image::imageops::FilterType::CatmullRom) };
    Ok(Picture { width: w, height: h, rgba: img.into_raw() })
}

/// An embedded preview, decoded within [`MAX_EMBEDDED_SIDE`] and a memory cap.
fn decode_png(png: &[u8]) -> Result<image::RgbaImage> {
    let bad = |e: image::ImageError| QuickLookError::BadPreview(e.to_string());
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EMBEDDED_SIDE);
    limits.max_image_height = Some(MAX_EMBEDDED_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(bad)?.into_rgba8();
    if img.width() == 0 || img.height() == 0 {
        return Err(QuickLookError::Empty);
    }
    Ok(img)
}

/// How much a Space-bar preview may cost.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Most pixels on the longer side.
    pub max_side: u32,
    /// Most pixels in all.
    pub max_pixels: u64,
    /// Longest the document may take to read and render; past it the embedded preview shows.
    pub time: Duration,
    /// Largest file (bytes on disk) whose document is read and rendered; a larger one shows its
    /// embedded preview.
    pub max_render_file: u64,
    /// What reading the document to render it may unpack of a compressed file; past it, the
    /// embedded preview shows.
    pub caps: ReadCaps,
    /// Most renders that ran out of time may still be running before no more are started (the
    /// embedded preview shows instead): a render can't be stopped, so paging through large
    /// documents with the Space bar would otherwise pile them up.
    pub max_late: usize,
    /// Renders that ran out of time and are still running, shared by the clones of these limits
    /// (the extension keeps one set for its whole process).
    pub late: Arc<AtomicUsize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_side: 2048,
            max_pixels: 2048 * 2048,
            time: Duration::from_secs(5),
            max_render_file: 128 << 20,
            caps: ReadCaps { json: 64 << 20, blobs: 256 << 20 },
            max_late: 1,
            late: Arc::default(),
        }
    }
}

/// A Space-bar preview: a PNG whose resolution (pHYs) gives its size in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The size to show it at, in points (the artboard's size for a rendered one).
    pub points: (f64, f64),
    /// Rendered from the document (`false`: the embedded preview).
    pub rendered: bool,
}

/// The Space-bar preview of a native file: the first artboard (else the art) rendered within
/// `limits`, or the embedded preview when rendering is refused, fails or runs out of time. A file
/// this version can't read is an error at once (nothing is rendered).
pub fn preview(bytes: Vec<u8>, limits: &Limits) -> Result<Preview> {
    // A document whose JSON unpacks past the render caps isn't rendered, but its preview shows.
    let (embedded, small) = match vectorcraft_format::preview_checked_within(&bytes, limits.caps) {
        Ok(embedded) => (embedded, true),
        Err(FormatError::TooBig) => (vectorcraft_format::preview_checked_within(&bytes, THUMBNAIL_CAPS)?, false),
        Err(e) => return Err(e.into()),
    };
    let rendered = if !small {
        Err(QuickLookError::Format(FormatError::TooBig))
    } else if bytes.len() as u64 > limits.max_render_file {
        Err(QuickLookError::TooLarge(bytes.len() as u64))
    } else {
        render_in_time(bytes, limits)
    };
    match (rendered, embedded) {
        (Ok(p), _) => Ok(p),
        (Err(_), Some(png)) => embedded_preview(png),
        (Err(e), None) => Err(e),
    }
}

/// A render's state, shared by its thread and the request waiting for it.
const RUNNING: u8 = 0;
const DONE: u8 = 1;
/// The request stopped waiting: the render counts in [`Limits::late`] until it ends.
const LATE: u8 = 2;

/// Ends a render on its thread (also when it panics): a late one stops counting.
struct Ending {
    state: Arc<AtomicU8>,
    late: Arc<AtomicUsize>,
}

impl Drop for Ending {
    fn drop(&mut self) {
        if self.state.swap(DONE, Ordering::SeqCst) == LATE {
            self.late.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// [`render`] on a thread of its own, given up on after `limits.time`. The renderer can't be
/// stopped part-way: a render that runs late finishes unseen, and while [`Limits::max_late`] of
/// them are still running no other is started ([`QuickLookError::Busy`]).
fn render_in_time(bytes: Vec<u8>, limits: &Limits) -> Result<Preview> {
    if limits.late.load(Ordering::SeqCst) >= limits.max_late {
        return Err(QuickLookError::Busy);
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let l = limits.clone();
    let state = Arc::new(AtomicU8::new(RUNNING));
    let ending = Ending { state: state.clone(), late: limits.late.clone() };
    std::thread::Builder::new()
        .name("quicklook-render".into())
        .spawn(move || {
            let _ending = ending;
            // Nobody listens any more once the time is up.
            let _ = tx.send(render(&bytes, &l));
        })
        .map_err(|e| QuickLookError::Render(e.to_string()))?;
    match rx.recv_timeout(limits.time) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Counted before it is marked late, so the thread never takes back what wasn't added.
            limits.late.fetch_add(1, Ordering::SeqCst);
            if state.compare_exchange(RUNNING, LATE, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                // It ended meanwhile.
                limits.late.fetch_sub(1, Ordering::SeqCst);
            }
            Err(QuickLookError::TimedOut(limits.time))
        }
        // The thread ended without an answer (a bug the renderer panicked on).
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(QuickLookError::Render("the renderer stopped".into())),
    }
}

/// Pixels per point to render a `w` × `h` point area at within `limits` (`None`: an empty or
/// unbounded area).
pub fn preview_scale(w: f64, h: f64, limits: &Limits) -> Option<f64> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let side = f64::from(limits.max_side.max(1)) / w.max(h);
    let area = (limits.max_pixels.max(1) as f64 / (w * h)).sqrt();
    let s = side.min(area).min(MAX_PREVIEW_SCALE);
    (s.is_finite() && s > 0.0).then_some(s)
}

/// The first artboard (else the art) of the document in `bytes` as a PNG within `limits`.
pub fn render(bytes: &[u8], limits: &Limits) -> Result<Preview> {
    let doc = vectorcraft_format::load_within(bytes, limits.caps)?;
    let (rect, white) = match doc.artboards.first() {
        Some(a) => (a.rect, true),
        None => (vectorcraft_render::encode::art_bounds(&doc).ok_or(QuickLookError::Empty)?, false),
    };
    let scale = preview_scale(rect.width(), rect.height(), limits).ok_or(QuickLookError::Empty)?;
    let (w, h) = raster_size(rect, scale).map_err(QuickLookError::Render)?;
    if w == 0 || h == 0 {
        return Err(QuickLookError::Empty);
    }
    let out = Renderer::new().render_region(&doc, rect, scale, white);
    let png = encode_png(out.width, out.height, &out.to_straight(), scale)?;
    Ok(Preview { png, width: out.width, height: out.height, points: (rect.width(), rect.height()), rendered: true })
}

/// The embedded preview as a [`Preview`] (checked by decoding it), shown at 1 point a pixel.
fn embedded_preview(png: Vec<u8>) -> Result<Preview> {
    let img = decode_png(&png)?;
    let (width, height) = img.dimensions();
    Ok(Preview { png, width, height, points: (f64::from(width), f64::from(height)), rendered: false })
}

/// Straight-alpha RGBA as a PNG of `scale` pixels per point (72 × `scale` pixels per inch).
fn encode_png(width: u32, height: u32, rgba: &[u8], scale: f64) -> Result<Vec<u8>> {
    let err = |e: png::EncodingError| QuickLookError::Render(e.to_string());
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let per_metre = (72.0 * scale / 0.0254).round().clamp(1.0, f64::from(u32::MAX)) as u32;
    enc.set_pixel_dims(Some(png::PixelDimensions { xppu: per_metre, yppu: per_metre, unit: png::Unit::Meter }));
    let mut w = enc.write_header().map_err(err)?;
    w.write_image_data(rgba).map_err(err)?;
    w.finish().map_err(err)?;
    Ok(out)
}

/// What an extension executable does on a system without Quick Look: say so and fail.
pub fn not_macos(name: &str) -> std::process::ExitCode {
    eprintln!("{name} is a macOS Quick Look extension: it runs inside VectorCraft.app on macOS only");
    std::process::ExitCode::FAILURE
}

#[cfg(test)]
mod tests;
