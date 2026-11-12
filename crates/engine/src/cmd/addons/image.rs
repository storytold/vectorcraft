//! Image Tools (`addon.image.*`), a built-in add-on. Remove Background makes the background of the
//! selected images transparent in their own pixels, as one undo step. The background is the most
//! common color along an image's edges, the average of its four corners, or colors the caller
//! picks. It is cleared where it reaches in from an edge, or wherever it is. The work is done here,
//! on the pixels; nothing is sent anywhere. Color Mask ([`mask`]) hides colors matched the same
//! way while the image keeps its pixels. They are VectorCraft additions. Its submenu (Object ›
//! Plug-ins › Image Tools) and dialogs are the UI's.

mod mask;
#[cfg(test)]
mod tests;

pub(crate) use mask::refresh as refresh_color_masks;

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::{Arc, Weak};

use image::{DynamicImage, ImageBuffer, LumaA, Rgba};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::{ImageBlob, ImageObject, NodeId, NodeKind};
use vectorcraft_geom::Affine;

use super::super::fileio::{MAX_RASTER_ALLOC, ppi};
use super::super::menucmds::has_image;
use super::super::paint::{check_colorized, check_colorized_kept};
use super::super::{CommandSpec, Result, Session, bad, checked_ids_param, cmd, hex_with_alpha, number_or, switch_or, text_param};

/// Remove Background's command.
pub const REMOVE_BACKGROUND: &str = "addon.image.removeBackground";
const C: &str = REMOVE_BACKGROUND;

/// The most colors `colors` lists.
const MAX_COLORS: usize = 16;

/// Bytes a pixel takes while it is worked on, beside its pixels: its level, whether the edge search
/// reached it, and its place in the search's stack (4 bytes, twice that while the stack grows).
const WORK_BYTES: u64 = 10;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![cmd!(
        "addon.image.removeBackground",
        "Remove Background…",
        ["Object", "Plug-ins", "Image Tools"],
        None,
        "{background?: \"edges\" (default: the most common color along the image's edges; nothing is cleared when the edges hold at least as many clear pixels as pixels of that color) | \"corners\" (the average of the image's four corner pixels that are not clear; nothing is cleared when all four are clear) | \"colors\" (the colors in `colors`), colors?: [color…] or one string of them split by commas or spaces (1 to 16 colors, each a hex color \"#rrggbb\" or \"#rgb\" with or without the # or one of black, white, red, green and blue; needed with background: colors, and checked but not used with edges or corners), contiguous?: bool (true: only background pixels that reach in from an edge are cleared; false: they are cleared wherever they are; true), match?: \"distance\" (default: a color's RGB distance from a background color) | \"channels\" (its largest difference on one of red, green and blue), tolerance?: 0..100 (with distance, % of the largest RGB distance; with channels, % of 255 steps on one channel; a pixel this near a background color is cleared; 10), softness?: 0..100 (% past the tolerance over which pixels fade from cleared to kept; 10), ids?: [id…] (default: the selected images)} make the background of each image transparent in its own pixels, as one undo step: the image stays an image, embedded as a PNG with an alpha channel at the depth of its pixels (16-bit images keep 16 bits and grayscale images stay gray) with the source's color profile and resolution, and a linked image is embedded; a CMYK image, a linked image that shows its preview (embed or relink it first), an image too large to change, a result with no pixel left, and a grayscale image colorized with a process color (which can't colorize gray pixels with transparency) are refused, and an image whose pixels do not change is left as it is (no undo step when none changes) → {changed: the images changed, ids, cleared: the pixels made fully clear in them}",
        has_image,
        remove_background
    )];
    v.extend(mask::specs());
    v
}

/// Where the background's colors come from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Background {
    /// The most common color along the image's edges.
    Edges,
    /// The average of the image's four corner pixels that are not clear.
    Corners,
    /// These colors.
    Colors(Vec<[u8; 3]>),
}

/// How near a color is to a background color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Match {
    /// The RGB distance, as a percentage of the largest one (black to white).
    Distance,
    /// The largest difference on one of red, green and blue, as a percentage of a channel's range.
    Channels,
}

/// What Remove Background clears: the background, whether only where it reaches in from an edge,
/// and how near a color must be to it (a percentage, measured as `measure` says) to be cleared, and
/// to fade out past that.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub background: Background,
    pub contiguous: bool,
    pub measure: Match,
    pub tolerance: f64,
    pub softness: f64,
}

impl Settings {
    /// The settings `p` gives to command `cmd`, each checked.
    fn read(p: &Value, cmd: &str) -> Result<Self> {
        let percent = |key: &str| -> Result<f64> {
            let v = number_or(p, key, 10.0, cmd)?;
            if v.is_finite() && (0.0..=100.0).contains(&v) { Ok(v) } else { Err(bad(cmd, format!("`{key}` must be from 0 to 100"))) }
        };
        let (tolerance, softness) = (percent("tolerance")?, percent("softness")?);
        let colors = match p.get("colors") {
            None | Some(Value::Null) => None,
            Some(v) => Some(colors(v, cmd)?),
        };
        let background = match text_param(p, "background", cmd)? {
            None | Some("edges") => Background::Edges,
            Some("corners") => Background::Corners,
            Some("colors") => Background::Colors(colors.ok_or_else(|| colors_error(cmd))?),
            Some(_) => return Err(bad(cmd, "`background` must be edges, corners or colors")),
        };
        let measure = match text_param(p, "match", cmd)? {
            None | Some("distance") => Match::Distance,
            Some("channels") => Match::Channels,
            Some(_) => return Err(bad(cmd, "`match` must be distance or channels")),
        };
        let contiguous = switch_or(p, "contiguous", true, cmd)?;
        Ok(Self { background, contiguous, measure, tolerance, softness })
    }
}

fn colors_error(cmd: &str) -> crate::EngineError {
    bad(cmd, "`colors` must list 1 to 16 colors, each hex (#rrggbb or #rgb) or black, white, red, green or blue")
}

/// The colors `v` lists to command `cmd`: an array of strings, or one string of them split by
/// commas or spaces. Each is read as [`Color::from_hex`] reads it; one with an alpha part is refused.
fn colors(v: &Value, cmd: &str) -> Result<Vec<[u8; 3]>> {
    // One past the most is enough to tell a list that is too long.
    let texts: Vec<&str> = match v {
        Value::String(s) => s.split([',', ' ', '\t', '\n']).filter(|t| !t.is_empty()).take(MAX_COLORS + 1).collect(),
        Value::Array(a) => a.iter().take(MAX_COLORS + 1).map(Value::as_str).collect::<Option<_>>().ok_or_else(|| colors_error(cmd))?,
        _ => return Err(colors_error(cmd)),
    };
    if texts.is_empty() || texts.len() > MAX_COLORS {
        return Err(colors_error(cmd));
    }
    texts.into_iter().map(|t| rgb8(t).ok_or_else(|| colors_error(cmd))).collect()
}

/// The 8-bit RGB of the color `t` writes.
fn rgb8(t: &str) -> Option<[u8; 3]> {
    if hex_with_alpha(t) {
        return None;
    }
    let [r, g, b] = Color::from_hex(t)?.to_rgb_uncalibrated();
    Some([r, g, b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}

fn remove_background(s: &mut Session, p: &Value) -> Result<Value> {
    let settings = Settings::read(p, C)?;
    // A preview step keeps the pixels it decodes for the next step; other runs keep none.
    let mut kept = std::mem::take(&mut s.decoded_images);
    let previewing = s.previewing;
    let found = changes(s, p, &settings, previewing.then_some(&mut kept));
    s.decoded_images = kept;
    let changes = found?;
    let ids: Vec<u64> = changes.iter().map(|(id, ..)| id.0).collect();
    let made_clear: u64 = changes.iter().map(|(_, out, _)| out.cleared).sum();
    if !changes.is_empty() {
        let colorized_ok = check_colorized(&s.doc()?.doc).is_ok();
        s.edit("Remove Background", |d, _| {
            for (id, out, xf) in changes {
                let key = out.blob.content_key();
                d.images.entry(key.clone()).or_insert(out.blob);
                if let Some(NodeKind::Image(im)) = d.node_mut(id).map(|n| &mut n.kind) {
                    im.key = key;
                    (im.width, im.height, im.xf) = (out.width, out.height, xf);
                    im.link = None;
                }
            }
            // A grayscale image colorized with a process color can't take an alpha channel.
            check_colorized_kept(colorized_ok, d).map_err(|e| bad(C, e))
        })?;
    }
    Ok(json!({ "changed": ids.len(), "ids": ids, "cleared": made_clear }))
}

/// The images `p` names (`ids`, or the selected images) whose pixels change, each with its pixels
/// cleared as `settings` say and the transform that keeps its place and size. Decoded pixels are
/// taken from and kept in `kept` when it is given.
fn changes(s: &Session, p: &Value, settings: &Settings, mut kept: Option<&mut DecodedImages>) -> Result<Vec<(NodeId, Cleared, Affine)>> {
    let images = images(s, p, C)?;
    // Each image's pixels once, however many objects show them.
    let mut cleared: HashMap<&str, Option<Cleared>> = HashMap::new();
    let mut changes = vec![];
    for (id, im, blob) in images {
        let out = match cleared.get(im.key.as_str()) {
            Some(out) => out.clone(),
            None => {
                let out = clear(blob, settings, kept.as_deref_mut(), C)?;
                cleared.insert(&im.key, out.clone());
                out
            }
        };
        if let Some(out) = out {
            // A file read at another size than the object's keeps the object's place and size.
            let xf = if (out.width, out.height) == (im.width, im.height) {
                im.xf
            } else {
                im.xf * Affine::scale_non_uniform(f64::from(im.width) / f64::from(out.width), f64::from(im.height) / f64::from(out.height))
            };
            changes.push((id, out, xf));
        }
    }
    Ok(changes)
}

/// The image objects command `cmd` acts on (`ids`, each once, or the selected images), each with
/// its pixels, all checked before any is decoded: an object that is not an image, missing pixels,
/// a linked image that shows its preview and a CMYK image are errors, as is no image at all.
fn images<'a>(s: &'a Session, p: &Value, cmd: &str) -> Result<Vec<(NodeId, &'a ImageObject, &'a ImageBlob)>> {
    let st = s.doc()?;
    let mut images = vec![];
    for id in targets(s, p, cmd)? {
        let Some(NodeKind::Image(im)) = st.doc.node(id).map(|n| &n.kind) else { continue };
        let blob = st.doc.images.get(&im.key).ok_or_else(|| bad(cmd, "the image's pixels are missing"))?;
        // The preview stands in for a file that hasn't been read, or that is kept at its preview.
        if im.link.is_some() && blob.is_proxy() {
            return Err(bad(cmd, "the linked image shows its preview: embed or relink it first"));
        }
        if blob.is_cmyk() {
            return Err(cmyk_refusal(cmd));
        }
        images.push((id, &**im, blob));
    }
    Ok(images)
}

/// The image objects command `cmd` acts on: `ids`, each once, or the selected images. An object in
/// `ids` that is not an image is an error, as is no image at all.
fn targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let ids: Vec<NodeId> = if p.get("ids").is_some_and(|v| !v.is_null()) {
        // Each image once, however often `ids` names it.
        let mut given = HashSet::new();
        let ids: Vec<NodeId> = checked_ids_param(s, p, "ids", cmd)?.into_iter().filter(|id| given.insert(*id)).collect();
        if let Some(id) = ids.iter().find(|id| !matches!(st.doc.node(**id).map(|n| &n.kind), Some(NodeKind::Image(_)))) {
            return Err(bad(cmd, format!("object {} is not an image", id.0)));
        }
        ids
    } else {
        st.selection.objects.iter().copied().filter(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_)))).collect()
    };
    if ids.is_empty() {
        return Err(bad(cmd, "select an image"));
    }
    Ok(ids)
}

/// Whether `cmd` is one of Color Mask's commands, which hide pixels instead of clearing them.
fn masks(cmd: &str) -> bool {
    matches!(cmd, mask::COLOR_MASK | mask::REMOVE_COLOR_MASK | mask::APPLY_COLOR_MASK)
}

/// Command `cmd`'s refusal of a CMYK image.
fn cmyk_refusal(cmd: &str) -> crate::EngineError {
    if masks(cmd) { bad(cmd, "a CMYK image can't take a color mask") } else { bad(cmd, "a CMYK image has no transparency to set") }
}

/// An image with its background cleared: its PNG, its size in pixels, and how many of its pixels
/// were made fully clear.
#[derive(Clone, Debug)]
pub(crate) struct Cleared {
    pub blob: ImageBlob,
    pub width: u32,
    pub height: u32,
    pub cleared: u64,
}

/// An image's pixels as straight RGBA, at 8 or 16 bits a sample.
#[derive(Clone)]
enum Pixels {
    Eight(image::RgbaImage),
    Sixteen(ImageBuffer<Rgba<u16>, Vec<u16>>),
}

/// The most bytes of decoded pixels and ICC profiles [`DecodedImages`] keeps (128 MiB, or 32 MiB in
/// the web app).
const KEEP_BYTES: u64 = MAX_RASTER_ALLOC / 16;

/// The images Remove Background decoded during an interaction's preview steps, the most recently
/// used first, at most [`KEEP_BYTES`] in all. The next preview step runs the command again on the
/// same bytes and takes their pixels from here instead of decoding them again. The session holds
/// it and empties it when the interaction ends (OK or Cancel). A run that is not a preview step
/// does not read or fill it.
#[derive(Default)]
pub(crate) struct DecodedImages(Vec<Decoded>);

/// An image decoded by [`clear`]: the bytes it was decoded from (held weakly, so they are not kept
/// alive for it, and an entry whose bytes are gone is dropped), its pixels, whether it was gray,
/// and its ICC profile.
struct Decoded {
    source: Weak<Vec<u8>>,
    pixels: Arc<Pixels>,
    gray: bool,
    icc: Option<Vec<u8>>,
}

#[cfg(test)]
thread_local! {
    /// How many times this thread decoded an image in [`clear`].
    pub(crate) static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl DecodedImages {
    /// Drops every image kept.
    pub(crate) fn empty(&mut self) {
        self.0.clear();
    }

    /// How many images are kept.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// The pixels, gray flag and ICC profile kept for `bytes`, if they are kept. Images whose bytes
    /// are gone are dropped first.
    fn get(&mut self, bytes: &Arc<Vec<u8>>) -> Option<(Arc<Pixels>, bool, Option<Vec<u8>>)> {
        self.0.retain(|d| d.source.strong_count() > 0);
        let at = self.0.iter().position(|d| std::ptr::eq(d.source.as_ptr(), Arc::as_ptr(bytes)))?;
        let d = self.0.remove(at);
        let found = (Arc::clone(&d.pixels), d.gray, d.icc.clone());
        self.0.insert(0, d);
        Some(found)
    }

    /// Keeps `pixels` decoded from `bytes` first, in place of anything kept for the same bytes, and
    /// drops images whose bytes are gone and the least recently used past [`KEEP_BYTES`].
    fn keep(&mut self, bytes: &Arc<Vec<u8>>, pixels: Arc<Pixels>, gray: bool, icc: Option<Vec<u8>>) {
        self.0.retain(|d| d.source.strong_count() > 0 && !std::ptr::eq(d.source.as_ptr(), Arc::as_ptr(bytes)));
        self.0.insert(0, Decoded { source: Arc::downgrade(bytes), pixels, gray, icc });
        let mut total = 0u64;
        self.0.retain(|d| {
            total = total.saturating_add(d.bytes());
            total <= KEEP_BYTES
        });
    }
}

impl Decoded {
    /// How many bytes the entry keeps: its pixels and its ICC profile.
    fn bytes(&self) -> u64 {
        self.pixels.bytes().saturating_add(self.icc.as_ref().map_or(0, |icc| icc.len() as u64))
    }
}

impl Pixels {
    /// How many bytes the pixels take.
    fn bytes(&self) -> u64 {
        match self {
            Self::Eight(img) => img.as_raw().len() as u64,
            Self::Sixteen(img) => img.as_raw().len() as u64 * 2,
        }
    }

    /// The width and height in pixels.
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Eight(img) => img.dimensions(),
            Self::Sixteen(img) => img.dimensions(),
        }
    }

    /// Pixel `i` (row by row from the top left) at 16 bits a sample (an 8-bit sample `v` is
    /// `v * 257`).
    fn rgba16(&self, i: usize) -> [u16; 4] {
        let mut out = [0u16; 4];
        match self {
            Self::Eight(img) => {
                for (o, v) in out.iter_mut().zip(img.as_raw().get(i * 4..i * 4 + 4).unwrap_or_default()) {
                    *o = u16::from(*v) * 257;
                }
            }
            Self::Sixteen(img) => {
                for (o, v) in out.iter_mut().zip(img.as_raw().get(i * 4..i * 4 + 4).unwrap_or_default()) {
                    *o = *v;
                }
            }
        }
        out
    }
}

/// `blob` with its background cleared as `settings` say, or `None` when no pixel changes. A CMYK
/// image, one that can't be decoded or is too large to change, and a result with no pixel left are
/// errors of command `cmd`. The PNG keeps the source's ICC profile and resolution. The decoded
/// pixels are taken from and kept in `kept` when it is given.
pub(crate) fn clear(blob: &ImageBlob, settings: &Settings, mut kept: Option<&mut DecodedImages>, cmd: &str) -> Result<Option<Cleared>> {
    if blob.is_cmyk() {
        return Err(cmyk_refusal(cmd));
    }
    let found = kept.as_deref_mut().and_then(|k| k.get(&blob.bytes));
    let (source, gray, icc) = match found {
        Some(found) => found,
        None => decode(blob, kept, cmd)?,
    };
    // The pixels kept for the next run stay as they are; ones not kept are worked on in place.
    let mut px = Arc::try_unwrap(source).unwrap_or_else(|kept| Pixels::clone(&kept));
    let (w, h) = px.dimensions();
    let levels = levels(w as usize, h as usize, &|i| px.rgba16(i), settings);
    let Some(cleared) = apply(&mut px, &levels) else { return Ok(None) };
    let left = match &px {
        Pixels::Eight(img) => img.pixels().any(|p| p.0[3] > 0),
        Pixels::Sixteen(img) => img.pixels().any(|p| p.0[3] > 0),
    };
    if !left {
        return Err(if masks(cmd) {
            bad(cmd, "the mask would hide every pixel")
        } else {
            bad(cmd, "every pixel matches the background: no image would be left")
        });
    }
    let out = match (px, gray) {
        (Pixels::Eight(img), false) => DynamicImage::ImageRgba8(img),
        (Pixels::Sixteen(img), false) => DynamicImage::ImageRgba16(img),
        (Pixels::Eight(img), true) => {
            DynamicImage::ImageLumaA8(ImageBuffer::from_fn(w, h, |x, y| LumaA([img.get_pixel(x, y).0[0], img.get_pixel(x, y).0[3]])))
        }
        (Pixels::Sixteen(img), true) => {
            DynamicImage::ImageLumaA16(ImageBuffer::from_fn(w, h, |x, y| LumaA([img.get_pixel(x, y).0[0], img.get_pixel(x, y).0[3]])))
        }
    };
    let mut png = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
    if let Some(icc) = icc {
        use image::ImageEncoder as _;
        encoder.set_icc_profile(icc).map_err(|e| bad(cmd, e.to_string()))?;
    }
    out.write_with_encoder(encoder).map_err(|e| bad(cmd, e.to_string()))?;
    // The resolution the file declares, which sizes the image at 100% when it is replaced or relinked.
    if let Some(r) = ppi::resolution(&blob.bytes) {
        png = ppi::with_png_resolution(&png, r);
    }
    Ok(Some(Cleared { blob: ImageBlob::png(png), width: w, height: h, cleared }))
}

/// `blob`'s pixels as straight RGBA, whether they were gray, and its ICC profile. An image that
/// can't be decoded or is too large to change is an error. The pixels are kept in `kept`, when it
/// is given, if a copy of them fits the decode allowance beside the work. Errors are command `cmd`'s.
fn decode(blob: &ImageBlob, kept: Option<&mut DecodedImages>, cmd: &str) -> Result<(Arc<Pixels>, bool, Option<Vec<u8>>)> {
    let reader = || {
        image::ImageReader::new(Cursor::new(blob.bytes.as_slice())).with_guessed_format().map_err(|_| bad(cmd, "the image's pixels can't be read"))
    };
    let mut decoder = reader()?.into_decoder().map_err(|e| bad(cmd, format!("the image's pixels can't be read: {e}")))?;
    // The ICC profile the colors are in goes with them into the PNG.
    let (w, h, kind, icc) = {
        use image::ImageDecoder as _;
        let (w, h) = decoder.dimensions();
        (w, h, decoder.color_type(), decoder.icc_profile().ok().flatten())
    };
    drop(decoder);
    // The file's pixels, those worked on (as many again for the PNG made of them) and the work,
    // checked from the header before anything is decoded.
    let deep = kind.bytes_per_pixel() > kind.channel_count();
    let per_pixel = u64::from(kind.bytes_per_pixel()) + 2 * if deep { 8 } else { 4 } + WORK_BYTES;
    if u64::from(w).checked_mul(u64::from(h)).and_then(|n| n.checked_mul(per_pixel)).is_none_or(|n| n > MAX_RASTER_ALLOC) {
        return Err(bad(cmd, format!("an image of {w} × {h} pixels is too large to change")));
    }
    let mut reader = reader()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_RASTER_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| bad(cmd, format!("the image's pixels can't be read: {e}")))?;
    #[cfg(test)]
    DECODES.with(|c| c.set(c.get() + 1));
    let gray = matches!(img.color(), image::ColorType::L8 | image::ColorType::La8 | image::ColorType::L16 | image::ColorType::La16);
    let px = Arc::new(if deep { Pixels::Sixteen(img.into_rgba16()) } else { Pixels::Eight(img.into_rgba8()) });
    // A kept copy is worked on as a copy: one more set of the pixels worked on.
    let with_copy = per_pixel + if deep { 8 } else { 4 };
    if let Some(kept) = kept
        && u64::from(w).checked_mul(u64::from(h)).and_then(|n| n.checked_mul(with_copy)).is_some_and(|n| n <= MAX_RASTER_ALLOC)
        && px.bytes().saturating_add(icc.as_ref().map_or(0, |icc| icc.len() as u64)) <= KEEP_BYTES
    {
        kept.keep(&blob.bytes, Arc::clone(&px), gray, icc.clone());
    }
    Ok((px, gray, icc))
}

/// Each pixel's alpha scaled by its level (255 keeps it). Returns how many pixels were made fully
/// clear, or `None` when no pixel changed.
fn apply(px: &mut Pixels, levels: &[u8]) -> Option<u64> {
    let (mut changed, mut cleared) = (false, 0u64);
    match px {
        Pixels::Eight(img) => {
            for (p, &l) in img.pixels_mut().zip(levels) {
                let a = ((u32::from(p.0[3]) * u32::from(l) + 127) / 255) as u8;
                changed |= a != p.0[3];
                cleared += u64::from(a == 0 && p.0[3] != 0);
                p.0[3] = a;
            }
        }
        Pixels::Sixteen(img) => {
            for (p, &l) in img.pixels_mut().zip(levels) {
                let a = ((u32::from(p.0[3]) * u32::from(l) + 127) / 255) as u16;
                changed |= a != p.0[3];
                cleared += u64::from(a == 0 && p.0[3] != 0);
                p.0[3] = a;
            }
        }
    }
    changed.then_some(cleared)
}

/// How much of each of the `w` × `h` pixels `px` gives (row by row, straight RGBA at 16 bits a
/// sample) stays, from 0 (cleared) to 255 (kept), as `settings` say. Each pixel's distance to the
/// background is worked out once. A pixel past the first `u32::MAX` is left kept by the edge search.
pub(crate) fn levels(w: usize, h: usize, px: &dyn Fn(usize) -> [u16; 4], settings: &Settings) -> Vec<u8> {
    let n = w.saturating_mul(h);
    let refs: Vec<[u16; 3]> = match &settings.background {
        Background::Colors(refs) => refs.iter().map(|c| c.map(|v| u16::from(v) * 257)).collect(),
        Background::Edges => match edge_color(w, h, px) {
            Some(edge) => vec![edge],
            None => return vec![255; n],
        },
        Background::Corners => match corner_color(w, h, px) {
            Some(corner) => vec![corner],
            None => return vec![255; n],
        },
    };
    let mut out: Vec<u8> = (0..n)
        .map(|i| {
            let [r, g, b, _] = px(i);
            level([r, g, b], &refs, settings)
        })
        .collect();
    if settings.contiguous {
        // Out from the edges, through clear pixels and pixels that lose some alpha; the pixels it
        // doesn't reach are kept.
        let reaches = |out: &[u8], i: usize| out.get(i).is_some_and(|l| *l < 255) || px(i)[3] == 0;
        let mut seen = vec![false; n];
        let mut stack: Vec<u32> = vec![];
        for i in border(w, h).filter(|&i| reaches(&out, i)) {
            if let (Some(s), Ok(at)) = (seen.get_mut(i), u32::try_from(i)) {
                *s = true;
                stack.push(at);
            }
        }
        while let Some(at) = stack.pop() {
            let i = at as usize;
            let (x, y) = (i % w, i / w);
            let next = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in next.into_iter().flatten() {
                if let Some(s) = seen.get_mut(j)
                    && !*s
                    && reaches(&out, j)
                    && let Ok(at) = u32::try_from(j)
                {
                    *s = true;
                    stack.push(at);
                }
            }
        }
        for (l, s) in out.iter_mut().zip(&seen) {
            if !*s {
                *l = 255;
            }
        }
    }
    out
}

/// The level of a pixel of color `c`: 0 within the tolerance of the nearest of `refs`, 255 from
/// the tolerance plus the softness on, and in proportion between them.
fn level(c: [u16; 3], refs: &[[u16; 3]], settings: &Settings) -> u8 {
    let Settings { tolerance, softness, measure, .. } = *settings;
    let d = percent(refs.iter().map(|r| gap(c, *r, measure)).fold(f64::INFINITY, f64::min), measure);
    if d <= tolerance {
        0
    } else if d >= tolerance + softness {
        255
    } else {
        ((d - tolerance) / softness * 255.0).round().clamp(0.0, 255.0) as u8
    }
}

/// How far apart `a` and `b` are, measured as `measure` says, before `percent` converts it: the
/// squared RGB distance, or the largest difference on one channel.
fn gap(a: [u16; 3], b: [u16; 3], measure: Match) -> f64 {
    match measure {
        Match::Distance => a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(y)).powi(2)).sum(),
        Match::Channels => f64::from(a.iter().zip(b).map(|(x, y)| x.abs_diff(y)).max().unwrap_or(0)),
    }
}

/// A `gap` as a percentage: of the largest RGB distance (black to white) with distance, or of the
/// channel's range with channels. A difference of `k` 8-bit steps on one channel is exactly
/// `k / 255 × 100` with channels.
fn percent(gap: f64, measure: Match) -> f64 {
    match measure {
        Match::Distance => gap.sqrt() / (65_535.0 * 3f64.sqrt()) * 100.0,
        Match::Channels => gap / 65_535.0 * 100.0,
    }
}

/// The pixels on the edges of a `w` × `h` image, each once.
fn border(w: usize, h: usize) -> impl Iterator<Item = usize> {
    let top = (0..w).filter(move |_| h > 0);
    let bottom = (0..w).filter(move |_| h > 1).map(move |x| (h - 1) * w + x);
    let sides = (1..h.saturating_sub(1)).flat_map(move |y| [Some(y * w), (w > 1).then(|| y * w + w - 1)].into_iter().flatten());
    top.chain(bottom).chain(sides)
}

/// The most common color along an image's edges: its edge pixels grouped by 16 levels a channel,
/// with the clear ones as a group of their own, as the average of the largest group. `None` when
/// the clear edge pixels are at least as many as that group: the background is already cleared.
fn edge_color(w: usize, h: usize, px: &dyn Fn(usize) -> [u16; 4]) -> Option<[u16; 3]> {
    let mut groups: HashMap<[u16; 3], (u64, [u64; 3])> = HashMap::new();
    let mut clear = 0u64;
    for i in border(w, h) {
        let [r, g, b, a] = px(i);
        if a == 0 {
            clear += 1;
            continue;
        }
        let e = groups.entry([r >> 12, g >> 12, b >> 12]).or_default();
        e.0 += 1;
        for (s, v) in e.1.iter_mut().zip([r, g, b]) {
            *s += u64::from(v);
        }
    }
    // Equal groups go by color, so the result doesn't depend on the map's order.
    let (_, (n, sums)) = groups.into_iter().max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.0.cmp(&a.0)))?;
    if clear >= n {
        return None;
    }
    Some(sums.map(|s| ((s + n / 2) / n.max(1)) as u16))
}

/// The average of the four corner pixels of a `w` × `h` image that are not clear (a corner of a
/// one-pixel-wide image counts twice). `None` when all four are clear: the background is already
/// cleared.
fn corner_color(w: usize, h: usize, px: &dyn Fn(usize) -> [u16; 4]) -> Option<[u16; 3]> {
    if w == 0 || h == 0 {
        return None;
    }
    let corners = [0, w - 1, (h - 1) * w, (h - 1) * w + w - 1];
    let (mut n, mut sums) = (0u64, [0u64; 3]);
    for [r, g, b, a] in corners.map(px) {
        if a > 0 {
            n += 1;
            for (s, v) in sums.iter_mut().zip([r, g, b]) {
                *s += u64::from(v);
            }
        }
    }
    (n > 0).then(|| sums.map(|s| ((s + n / 2) / n) as u16))
}
