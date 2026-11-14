//! CMYK images: the ink amounts of image blobs that hold CMYK samples.
//!
//! Two encodings hold them: a four-component JPEG, whose samples are stored inverted with Adobe's
//! marker (the convention JPEG readers and PDF writers take a CMYK JPEG to follow), and a CMYK
//! TIFF (8 or 16 bits, ink amounts as they are, each pixel's inks together or one plane per ink).
//! [`ImageBlob::cmyk_tiff`] writes the latter, inks together, Deflate-compressed. Every other image
//! decodes to RGB(A) only.

use std::io::Cursor;
use std::sync::Arc;

use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image as CodecImage, Limits as CodecLimits, SampleType};

use crate::ImageBlob;

/// Most pixels a CMYK image is decoded with (256 MB of samples): larger ones decode as RGB.
pub const MAX_CMYK_PIXELS: u64 = 1 << 26;

/// The ink amounts of a CMYK image: C, M, Y and K bytes per pixel, 0 is no ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Inks {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Inks {
    /// `data` as a `width` × `height` image; `None` when it doesn't hold that many pixels, the
    /// image is empty or it has more than [`MAX_CMYK_PIXELS`].
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        let n = pixels(width, height)?;
        (data.len() as u64 == n * 4).then_some(Self { width, height, data })
    }
}

/// `width` × `height` when it is a CMYK image's size: not empty, at most [`MAX_CMYK_PIXELS`].
fn pixels(width: u32, height: u32) -> Option<u64> {
    let n = width as u64 * height as u64;
    (n > 0 && n <= MAX_CMYK_PIXELS).then_some(n)
}

/// The number of colour components of JPEG `bytes` (its frame header); `None` when they aren't a
/// JPEG.
pub fn jpeg_components(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    craft_image_codecs::jpeg_info(bytes).map(|info| info.layout.channels())
}

fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")
}

/// The width and height of the first image of TIFF `bytes` when it has more strips or tiles than
/// the `tiff` decoder can count: strips per plane times planes past `u32::MAX`, or tiles per plane
/// times planes past `usize::MAX`. A file that stores one plane per channel (PlanarConfiguration 2)
/// has a plane for each channel; other files have one. The decoder multiplies these counts without
/// a check, which panics where overflow checks are on. Such an image, when it is at least a pixel
/// wide, claims at least 2^32 samples.
/// Where a tag repeats, the value giving the most strips or tiles counts. `None` for other images
/// and for bytes that aren't a TIFF.
pub fn oversized_tiff(bytes: &[u8]) -> Option<(u32, u32)> {
    let le = bytes.starts_with(b"II*\0");
    if !le && !bytes.starts_with(b"MM\0*") {
        return None;
    }
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = bytes.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    // (tag, the entry's single BYTE, SHORT, LONG or IFD value, which sits in the entry, or its
    // single LONG8 or IFD8 value that fits in 32 bits, which sits where the entry points)
    let entry = |i: usize| -> Option<(u16, Option<u32>)> {
        let e = ifd.checked_add(i.checked_mul(12)?.checked_add(2)?)?;
        let (ty, count, at) = (u16_at(e.checked_add(2)?)?, u32_at(e.checked_add(4)?)?, e.checked_add(8)?);
        let value = match (ty, count) {
            (1, 1) => bytes.get(at).map(|&b| u32::from(b)),
            (3, 1) => u16_at(at).map(u32::from),
            (4 | 13, 1) => u32_at(at),
            (16 | 18, 1) => {
                let at = usize::try_from(u32_at(at)?).ok()?;
                let (lo, hi) = (u32_at(at)?, u32_at(at.checked_add(4)?)?);
                let wide = if le { u64::from(hi) << 32 | u64::from(lo) } else { u64::from(lo) << 32 | u64::from(hi) };
                u32::try_from(wide).ok()
            }
            _ => None,
        };
        Some((u16_at(e)?, value))
    };
    let (mut width, mut height, mut planes, mut planar) = (0u32, 0u32, 1u32, false);
    let (mut rows, mut tile_w, mut tile_h) = (None::<u32>, None::<u32>, None::<u32>);
    let least = |was: Option<u32>, v: u32| Some(was.map_or(v, |w| w.min(v)));
    for i in 0..usize::from(u16_at(ifd)?) {
        let Some((tag, value)) = entry(i) else { break };
        let Some(v) = value else { continue };
        match tag {
            256 => width = width.max(v),
            257 => height = height.max(v),
            277 => planes = planes.max(v),
            278 => rows = least(rows, v),
            284 => planar |= v == 2,
            322 => tile_w = least(tile_w, v),
            323 => tile_h = least(tile_h, v),
            _ => {}
        }
    }
    let planes = if planar { u128::from(planes) } else { 1 };
    let tiles = match (tile_w, tile_h) {
        (Some(tw @ 1..), Some(th @ 1..)) => u128::from(width.div_ceil(tw)) * u128::from(height.div_ceil(th)) * planes > usize::MAX as u128,
        _ => false,
    };
    let rows = rows.unwrap_or(height);
    let strips = rows > 0 && u128::from(height.saturating_sub(1) / rows + 1) * planes > u128::from(u32::MAX);
    (tiles || strips).then_some((width, height))
}

/// Parsed TIFF `bytes`; `None` for non-TIFF or [`oversized_tiff`] images.
pub(crate) fn parse_tiff(bytes: &[u8]) -> Option<craft_tiff::Tiff> {
    if !is_tiff(bytes) || oversized_tiff(bytes).is_some() {
        return None;
    }
    craft_tiff::Tiff::parse(bytes).ok()
}

/// Image metadata of TIFF `bytes`' first directory; `None` for non-TIFF or [`oversized_tiff`].
pub(crate) fn tiff_image_info(bytes: &[u8]) -> Option<craft_tiff::image::ImageInfo> {
    parse_tiff(bytes)?.ifds.first()?.image().ok()
}

/// Bit depth (8 or 16) when `info` describes a 4-channel CMYK TIFF.
fn is_cmyk_tiff_info(info: &craft_tiff::image::ImageInfo) -> Option<u16> {
    if info.photometric != 5 || info.samples_per_pixel != 4 {
        return None;
    }
    let bps = *info.bits_per_sample.first()?;
    if !matches!(bps, 8 | 16) || info.bits_per_sample.iter().any(|&b| b != bps) {
        return None;
    }
    Some(bps)
}

/// The Orientation tag of TIFF `bytes`' first image, of any unsigned type; `None` for other images
/// and TIFFs without one. Reads only its header.
pub fn tiff_orientation(bytes: &[u8]) -> Option<u16> {
    parse_tiff(bytes)?.ifds.first()?.u16(craft_tiff::tags::ORIENTATION)
}

/// The EXIF orientation of encoded image `bytes` when it turns or flips the pixels; `None` when
/// it leaves them as stored, or the image has none. A TIFF the image decoder can't open (one with
/// extra samples) has its Orientation tag read from its first image ([`tiff_orientation`]). Reads
/// only the image's header and metadata; `None` for an [`oversized_tiff`], which no decoder opens.
pub fn exif_orientation(bytes: &[u8]) -> Option<image::metadata::Orientation> {
    use image::metadata::Orientation;
    if oversized_tiff(bytes).is_some() {
        return None;
    }
    let raw = if is_tiff(bytes) {
        tiff_orientation(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_exif_bytes(bytes).map(craft_image_codecs::exif_orientation)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_exif_bytes(bytes).map(craft_image_codecs::exif_orientation)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        webp_exif_bytes(bytes).map(craft_image_codecs::exif_orientation)
    } else {
        None
    };
    raw.and_then(|v| Orientation::from_exif(u8::try_from(v).ok()?)).filter(|o| *o != Orientation::NoTransforms)
}

/// EXIF payload from a JPEG's APP1 `Exif\0\0` segment, if present before SOS.
fn jpeg_exif_bytes(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        let marker = *bytes.get(i)?;
        i += 1;
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]));
        if len < 2 || i + len > bytes.len() {
            return None;
        }
        let payload = &bytes[i + 2..i + len];
        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") {
            return Some(&payload[6..]);
        }
        i += len;
    }
    None
}

/// EXIF payload from a PNG's `eXIf` chunk, if present.
fn png_exif_bytes(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 8usize;
    while i + 12 <= bytes.len() {
        let len = usize::try_from(u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?)).ok()?;
        let kind = &bytes[i + 4..i + 8];
        let end = i.checked_add(8)?.checked_add(len)?;
        if end + 4 > bytes.len() {
            return None;
        }
        if kind == b"eXIf" {
            return Some(&bytes[i + 8..end]);
        }
        if kind == b"IEND" {
            return None;
        }
        i = end + 4;
    }
    None
}

/// EXIF payload from a WebP's `EXIF` RIFF chunk, if present.
fn webp_exif_bytes(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 12usize;
    while i + 8 <= bytes.len() {
        let kind = &bytes[i..i + 4];
        let len = usize::try_from(u32::from_le_bytes(bytes[i + 4..i + 8].try_into().ok()?)).ok()?;
        let end = i.checked_add(8)?.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        if kind == b"EXIF" {
            return Some(&bytes[i + 8..end]);
        }
        i = end + (len & 1);
    }
    None
}

/// Does the TIFF `ifd` store one plane per channel (PlanarConfiguration 2)?
fn planar(ifd: &craft_tiff::Ifd) -> bool {
    ifd.u16(craft_tiff::tags::PLANAR_CONFIGURATION).is_some_and(|p| p != 1)
}

/// Samples of an image, pixel by pixel, at the depth its file stores them.
#[derive(Clone, Debug, PartialEq)]
enum Samples {
    Eight(Vec<u8>),
    Sixteen(Vec<u16>),
}

impl Samples {
    fn as_slice(&self) -> SampleSlice<'_> {
        match self {
            Self::Eight(v) => SampleSlice::Eight(v),
            Self::Sixteen(v) => SampleSlice::Sixteen(v),
        }
    }
}

/// [`Samples`], borrowed.
#[derive(Clone, Copy, Debug)]
enum SampleSlice<'a> {
    Eight(&'a [u8]),
    Sixteen(&'a [u16]),
}

/// Decode CMYK TIFF `bytes` (8 or 16 bits, chunky or planar) via `craft-image-codecs`.
fn decode_cmyk_tiff_samples(bytes: &[u8], info: &craft_tiff::image::ImageInfo) -> Option<Samples> {
    let _ = is_cmyk_tiff_info(info)?;
    let _ = pixels(info.width, info.height)?;
    let opts = craft_image_codecs::DecodeOptions { limits: CodecLimits::unlimited(), keep_orientation: true };
    let img = craft_image_codecs::decode_as_with(Format::Tiff, bytes, &opts).ok()?;
    if img.layout != ChannelLayout::Cmyk || img.width != info.width || img.height != info.height {
        return None;
    }
    match img.sample_type {
        SampleType::U8 => Some(Samples::Eight(img.into_data())),
        SampleType::U16 => Some(Samples::Sixteen(img.to_u16_samples()?)),
        _ => None,
    }
}

/// The color space an ICC profile's header names (bytes 16 to 19, such as `RGB `, `GRAY` or
/// `CMYK`); `None` when `icc` is shorter than a header.
pub fn icc_color_space(icc: &[u8]) -> Option<[u8; 4]> {
    if icc.len() < 128 {
        return None;
    }
    icc.get(16..20)?.try_into().ok()
}

/// The ICC profile of TIFF tag 34675 (ICCProfile) of `ifd`; `None` without one.
fn tiff_icc(ifd: &craft_tiff::Ifd) -> Option<Vec<u8>> {
    ifd.bytes(craft_tiff::tags::ICC_PROFILE).filter(|icc| !icc.is_empty()).map(|icc| icc.to_vec())
}

/// The ICC profile of TIFF `bytes`' first image (tag 34675); `None` for other images and TIFFs
/// without one.
pub fn tiff_icc_profile(bytes: &[u8]) -> Option<Vec<u8>> {
    tiff_icc(parse_tiff(bytes)?.ifds.first()?)
}

/// The ICC profile of a JPEG, put together from all its numbered `ICC_PROFILE` APP2 segments;
/// `None` without one.
fn jpeg_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut chunks: Vec<(u8, u8, &[u8])> = Vec::new();
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        let Some(&marker) = bytes.get(i) else { break };
        i += 1;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]));
        if len < 2 || i + len > bytes.len() {
            break;
        }
        let payload = &bytes[i + 2..i + len];
        if marker == 0xE2 && payload.starts_with(b"ICC_PROFILE\0") && payload.len() >= 14 {
            let seq = payload[12];
            let total = payload[13];
            if seq >= 1 && total >= seq {
                chunks.push((seq, total, &payload[14..]));
            }
        }
        i += len;
    }
    if chunks.is_empty() {
        return None;
    }
    let total = chunks[0].1;
    chunks.sort_by_key(|c| c.0);
    let mut out = Vec::new();
    for expected in 1..=total {
        let (_, _, data) = chunks.iter().find(|c| c.0 == expected && c.1 == total)?;
        out.extend_from_slice(data);
    }
    (!out.is_empty()).then_some(out)
}

/// `icc` when it is a profile of CMYK colors, which the samples of a CMYK image are in.
fn cmyk_icc(icc: Option<Vec<u8>>) -> Option<Vec<u8>> {
    icc.filter(|icc| icc_color_space(icc) == Some(*b"CMYK"))
}

/// The CMYK samples of a CMYK image at the depth its file stores them, and its ICC profile.
struct CmykSource {
    width: u32,
    height: u32,
    samples: Samples,
    icc: Option<Vec<u8>>,
}

/// The samples of CMYK TIFF `bytes` (8 or 16 bits), their inks together or one plane per ink, with
/// the TIFF's CMYK ICC profile; `None` for other images and ones larger than [`MAX_CMYK_PIXELS`].
fn tiff_source(bytes: &[u8]) -> Option<CmykSource> {
    let tiff = parse_tiff(bytes)?;
    let ifd = tiff.ifds.first()?;
    let info = ifd.image().ok()?;
    let samples = decode_cmyk_tiff_samples(bytes, &info)?;
    let icc = cmyk_icc(tiff_icc(ifd));
    Some(CmykSource { width: info.width, height: info.height, samples, icc })
}

/// The ink amounts of a four-component JPEG, 8 bits each, with its CMYK ICC profile.
fn jpeg_source(bytes: &[u8]) -> Option<CmykSource> {
    let Inks { width, height, data } = jpeg_inks(bytes)?;
    Some(CmykSource { width, height, samples: Samples::Eight(data), icc: cmyk_icc(jpeg_icc(bytes)) })
}

/// A Deflate-compressed CMYK TIFF of `width` × `height` pixels of `samples` (inks together, 8 or 16
/// bits as they are given), declaring the resolution `ppi` when both of its values are finite and
/// above 0, and carrying `icc` as tag 34675 (ICCProfile, of type UNDEFINED); `None` when it can't be
/// written.
fn write_cmyk_tiff(width: u32, height: u32, samples: SampleSlice<'_>, ppi: Option<(f64, f64)>, icc: Option<&[u8]>) -> Option<Vec<u8>> {
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value, tags};
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write as _;

    let (bps, raw): (u16, Vec<u8>) = match samples {
        SampleSlice::Eight(data) => (8, data.to_vec()),
        SampleSlice::Sixteen(data) => (16, data.iter().flat_map(|s| s.to_le_bytes()).collect()),
    };
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&raw).ok()?;
    let compressed = enc.finish().ok()?;

    let mut ifd = IfdBuilder::new();
    ifd.set(tags::IMAGE_WIDTH, Value::Long(vec![width]))
        .set(tags::IMAGE_LENGTH, Value::Long(vec![height]))
        .set(tags::BITS_PER_SAMPLE, Value::Short(vec![bps; 4]))
        .set(tags::COMPRESSION, Value::Short(vec![8]))
        .set(tags::PHOTOMETRIC, Value::Short(vec![5]))
        .set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![4]))
        .set(tags::PLANAR_CONFIGURATION, Value::Short(vec![1]))
        .set(tags::SAMPLE_FORMAT, Value::Short(vec![1; 4]))
        .set_image(ImageData::Strips { rows_per_strip: height.max(1), strips: vec![compressed] });

    if let Some((x, y)) = ppi.filter(|(x, y)| [x, y].iter().all(|v| v.is_finite() && **v > 0.0)) {
        ifd.set(tags::X_RESOLUTION, Value::Rational(vec![((x * 10_000.0).round() as u32, 10_000)]))
            .set(tags::Y_RESOLUTION, Value::Rational(vec![((y * 10_000.0).round() as u32, 10_000)]))
            .set(tags::RESOLUTION_UNIT, Value::Short(vec![2]));
    }
    if let Some(icc) = icc {
        ifd.set(tags::ICC_PROFILE, Value::Undefined(icc.to_vec()));
    }
    TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).ok()
}

impl ImageBlob {
    /// A CMYK TIFF of ink amounts `inks` (Deflate-compressed); `None` when it can't be written.
    pub fn cmyk_tiff(inks: &Inks) -> Option<Self> {
        Self::cmyk_tiff_at(inks, None)
    }

    /// [`ImageBlob::cmyk_tiff`], declaring the resolution `ppi` (horizontal, vertical pixels per
    /// inch) when it is given and both are finite and above 0.
    pub fn cmyk_tiff_at(inks: &Inks, ppi: Option<(f64, f64)>) -> Option<Self> {
        let tiff = write_cmyk_tiff(inks.width, inks.height, SampleSlice::Eight(&inks.data), ppi, None)?;
        Some(Self::new("image/tiff", tiff))
    }

    /// Does the image hold CMYK samples (a four-component JPEG or a CMYK TIFF)? Reads only its
    /// header.
    pub fn is_cmyk(&self) -> bool {
        jpeg_components(&self.bytes) == Some(4) || tiff_image_info(&self.bytes).is_some_and(|info| is_cmyk_tiff_info(&info).is_some())
    }

    /// Does the image hold CMYK samples of 16 bits (a CMYK TIFF), which [`Self::cmyk`] gives at 8
    /// bits? Reads only its header.
    pub fn is_cmyk16(&self) -> bool {
        tiff_image_info(&self.bytes).is_some_and(|info| is_cmyk_tiff_info(&info) == Some(16))
    }

    /// The width and height a JPEG or TIFF says it has, for any color type. Reads only its header.
    pub fn header_size(&self) -> Option<(u32, u32)> {
        let bytes = self.bytes.as_slice();
        if bytes.starts_with(&[0xFF, 0xD8]) {
            return craft_image_codecs::jpeg_info(bytes).map(|info| (info.width, info.height));
        }
        tiff_image_info(bytes).map(|info| (info.width, info.height))
    }

    /// The ink amounts of a CMYK image; `None` for other images, and ones that can't be decoded
    /// or are larger than [`MAX_CMYK_PIXELS`].
    pub fn cmyk(&self) -> Option<Inks> {
        let bytes = self.bytes.as_slice();
        if bytes.starts_with(&[0xFF, 0xD8]) { jpeg_inks(bytes) } else { tiff_inks(bytes) }
    }

    /// The CMYK ICC profile a CMYK image carries: a four-component JPEG's `ICC_PROFILE` APP2
    /// segments put together, or a CMYK TIFF's tag 34675. `None` for other images, and for a
    /// profile that isn't one of CMYK colors. Reads only the file's headers and tags.
    pub fn cmyk_icc_profile(&self) -> Option<Vec<u8>> {
        let bytes = self.bytes.as_slice();
        if jpeg_components(bytes) == Some(4) {
            return cmyk_icc(jpeg_icc(bytes));
        }
        let tiff = parse_tiff(bytes)?;
        let ifd = tiff.ifds.first()?;
        let info = ifd.image().ok()?;
        let _ = is_cmyk_tiff_info(&info)?;
        cmyk_icc(tiff_icc(ifd))
    }

    /// The ink amounts around pixel (`x`, `y`) of a CMYK image, in the pixel space of an image
    /// object of `object` (width, height) pixels: the average of the `size` × `size` pixels centered
    /// on it, clipped to the image, as C, M, Y and K bytes. `None` outside the image and for images
    /// [`Self::cmyk`] doesn't read. The inks of the last CMYK image are kept for the next sample
    /// (the Eyedropper samples one image click after click), as [`Self::sample_object`] keeps its
    /// pixels.
    pub fn sample_cmyk_object(&self, x: f64, y: f64, object: (u32, u32), size: u32) -> Option<[u8; 4]> {
        thread_local! {
            static LAST: std::cell::RefCell<Option<(String, Arc<Inks>)>> = const { std::cell::RefCell::new(None) };
        }
        if !self.is_cmyk() {
            return None;
        }
        let key = self.content_key();
        let inks = LAST.with_borrow_mut(|last| match last {
            Some((k, inks)) if *k == key => Some(inks.clone()),
            _ => {
                let inks = Arc::new(self.cmyk()?);
                *last = Some((key, inks.clone()));
                Some(inks)
            }
        })?;
        let k = |n: u32, of: u32| n as f64 / of.max(1) as f64;
        sample_inks(&inks, x * k(inks.width, object.0), y * k(inks.height, object.1), size)
    }
}

/// [`ImageBlob::sample_cmyk_object`] of `inks` at (`x`, `y`) in their own pixel space.
fn sample_inks(inks: &Inks, x: f64, y: f64, size: u32) -> Option<[u8; 4]> {
    let (w, h) = (i64::from(inks.width), i64::from(inks.height));
    let (cx, cy) = (x.floor() as i64, y.floor() as i64);
    if !(0..w).contains(&cx) || !(0..h).contains(&cy) {
        return None;
    }
    let r = i64::from(size.max(1)) / 2;
    let (mut sum, mut n) = ([0u64; 4], 0u64);
    for py in (cy - r).max(0)..=(cy + r).min(h - 1) {
        for px in (cx - r).max(0)..=(cx + r).min(w - 1) {
            let i = usize::try_from((py * w + px) * 4).ok()?;
            let p = inks.data.get(i..i + 4)?;
            sum.iter_mut().zip(p).for_each(|(s, v)| *s += u64::from(*v));
            n += 1;
        }
    }
    let c = |v: u64| ((v + n / 2) / n) as u8;
    Some(sum.map(c))
}

/// The ink amounts of a four-component JPEG.
fn jpeg_inks(bytes: &[u8]) -> Option<Inks> {
    let limits = CodecLimits { max_pixels: MAX_CMYK_PIXELS, ..CodecLimits::default() };
    let img = craft_image_codecs::decode_as_with(Format::Jpeg, bytes, &limits).ok()?;
    if img.layout() != ChannelLayout::Cmyk || img.sample_type() != SampleType::U8 {
        return None;
    }
    let (w, h) = img.dimensions();
    pixels(w, h)?;
    Inks::new(w, h, img.into_data())
}

/// The ink amounts of a CMYK TIFF (16-bit samples to 8 bits), its inks stored together or one plane
/// per ink.
fn tiff_inks(bytes: &[u8]) -> Option<Inks> {
    let info = tiff_image_info(bytes)?;
    let data = match decode_cmyk_tiff_samples(bytes, &info)? {
        Samples::Eight(v) => v,
        Samples::Sixteen(v) => v.into_iter().map(|s| (s >> 8) as u8).collect(),
    };
    Inks::new(info.width, info.height, data)
}

/// CMYK TIFF `bytes` that no orientation turns ([`turned_cmyk`] stores the others) as Open and
/// Place store them: as they are, or, when they store one plane per ink, which the image decoder
/// doesn't read, their inks together in a CMYK TIFF with the resolution `ppi`, at the 8 or 16 bits
/// the file has, carrying the file's CMYK ICC profile. `None` for other images and for CMYK TIFFs
/// whose inks [`ImageBlob::cmyk`] doesn't read.
pub fn stored_cmyk_tiff(bytes: &[u8], ppi: Option<(f64, f64)>) -> Option<Vec<u8>> {
    let source = tiff_source(bytes)?;
    let tiff = parse_tiff(bytes)?;
    let ifd = tiff.ifds.first()?;
    if !planar(ifd) {
        return Some(bytes.to_vec());
    }
    write_cmyk_tiff(source.width, source.height, source.samples.as_slice(), ppi, source.icc.as_deref())
}

/// CMYK JPEG or TIFF `bytes` that `orientation` turns or flips, as Open and Place store them: their
/// inks turned and written again as a CMYK TIFF with the resolution `ppi` of the turned pixels,
/// with the turned width and height. A TIFF keeps its 8 or 16 bits, a JPEG has 8, and both keep
/// their CMYK ICC profile (a JPEG's put together from all its APP2 segments). `None` for other
/// images and for CMYK images whose inks [`ImageBlob::cmyk`] doesn't read.
pub fn turned_cmyk(bytes: &[u8], orientation: image::metadata::Orientation, ppi: Option<(f64, f64)>) -> Option<(Vec<u8>, (u32, u32))> {
    let CmykSource { width, height, samples, icc } = if bytes.starts_with(&[0xFF, 0xD8]) { jpeg_source(bytes) } else { tiff_source(bytes) }?;
    // Four inks to a pixel, turned as four channels of the same depth are.
    let mut px = match samples {
        Samples::Eight(v) => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(width, height, v)?),
        Samples::Sixteen(v) => image::DynamicImage::ImageRgba16(image::ImageBuffer::from_raw(width, height, v)?),
    };
    px.apply_orientation(orientation);
    let size = (px.width(), px.height());
    let samples = match px {
        image::DynamicImage::ImageRgba16(v) => Samples::Sixteen(v.into_raw()),
        other => Samples::Eight(other.into_rgba8().into_raw()),
    };
    let tiff = write_cmyk_tiff(size.0, size.1, samples.as_slice(), ppi, icc.as_deref())?;
    Some((tiff, size))
}

/// A TIFF whose pixels have samples after their colour, which the image decoder doesn't read at
/// all, as RGBA pixels: CMYK with an alpha channel (5 samples), each colour converted by `rgb` (ink
/// amounts 0..1 → sRGB 0..1; the inks can't be kept with the transparency, so they show as RGB),
/// and grayscale with extra samples, of which the first is the alpha when the file says so (other
/// channels are left out). Associated (premultiplied) alpha is divided back out, and a pixel with
/// no associated alpha has no color. 16-bit samples are converted at 16 bits, and only the result
/// is rounded to 8 bits. The samples may be stored together or one plane per channel. `None` for
/// other images and ones larger than [`MAX_CMYK_PIXELS`].
pub fn extra_samples_tiff_rgba(bytes: &[u8], rgb: impl Fn([f32; 4]) -> [f32; 3]) -> Option<image::RgbaImage> {
    let tiff = parse_tiff(bytes)?;
    let ifd = tiff.ifds.first()?;
    let info = ifd.image().ok()?;
    let bps = *info.bits_per_sample.first()?;
    if !matches!(bps, 8 | 16) || info.bits_per_sample.iter().any(|&b| b != bps) {
        return None;
    }
    let is_cmyka = info.photometric == 5 && info.samples_per_pixel == 5;
    let is_gray_extra = matches!(info.photometric, 0 | 1) && info.samples_per_pixel >= 2;
    if !is_cmyka && !is_gray_extra {
        return None;
    }
    let (w, h) = (info.width, info.height);
    let n = usize::try_from(pixels(w, h)?).ok()?;
    // ExtraSamples 1: associated alpha (premultiplied colour), 2: unassociated alpha, 0: not alpha.
    let extra = ifd.u64s(craft_tiff::tags::EXTRA_SAMPLES).and_then(|v| v.first().copied()).and_then(|v| u16::try_from(v).ok());
    let opts = craft_image_codecs::DecodeOptions { limits: CodecLimits::unlimited(), keep_orientation: true };
    let img = craft_image_codecs::decode_as_with(Format::Tiff, bytes, &opts).ok()?;
    let (samples, colour, alpha) = match img.layout {
        ChannelLayout::CmykA if is_cmyka => (5, 4, true),
        ChannelLayout::GrayA if is_gray_extra => (2, 1, true),
        ChannelLayout::Gray if is_gray_extra => (1, 1, false),
        _ => return None,
    };
    let data = match img.sample_type {
        SampleType::U8 => Samples::Eight(img.into_data()),
        SampleType::U16 => Samples::Sixteen(img.to_u16_samples()?),
        _ => return None,
    };
    // Sample `i` of the image, 0..1.
    let level_at = |i: usize| match &data {
        Samples::Eight(v) => v.get(i).map(|&s| f32::from(s) / 255.0),
        Samples::Sixteen(v) => v.get(i).map(|&s| f32::from(s) / 65535.0),
    };
    // Sample `i` of the image as 8 bits, a 16-bit sample rounded to the nearest.
    let byte_at = |i: usize| match &data {
        Samples::Eight(v) => v.get(i).copied(),
        Samples::Sixteen(v) => v.get(i).map(|&s| u8::try_from((u32::from(s) + 128) / 257).unwrap_or(u8::MAX)),
    };
    let mut out = Vec::with_capacity(n.checked_mul(4)?);
    for first in (0..n).map(|p| p * samples) {
        let (opacity, a) = if alpha { (byte_at(first + colour).unwrap_or(u8::MAX), level_at(first + colour).unwrap_or(1.0)) } else { (u8::MAX, 1.0) };
        let level = |s: usize| {
            let v = level_at(first + s).unwrap_or(0.0);
            match extra {
                Some(1) if a > 0.0 => (v / a).min(1.0),
                // No associated alpha: the color is zero (TIFF 6.0, Associated Alpha Handling).
                Some(1) if alpha => 0.0,
                _ => v,
            }
        };
        let [r, g, b] = match colour {
            4 => rgb([level(0), level(1), level(2), level(3)]),
            _ => [level(0); 3],
        };
        out.extend([r, g, b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        out.push(opacity);
    }
    image::RgbaImage::from_raw(w, h, out)
}

/// Why a TIFF compressed with CCITT Group 3 wasn't read ([`group3_tiff_rgba`]).
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Group3Error {
    /// Its pixels would take more than [`crate::MAX_RASTER_ALLOC`] as RGBA.
    #[error("an image of {w} × {h} pixels is too large to draw")]
    TooLarge { w: u32, h: u32 },
    /// A strip or tile ends before its last row or can't be decoded (it holds a code the decoder
    /// doesn't read, such as T.4's uncompressed mode, or a row of the wrong width), or a tile is
    /// larger than [`crate::MAX_RASTER_ALLOC`] allows as RGBA.
    #[error("could not decode image: {0}")]
    Damaged(craft_stream_codecs::CcittErrorKind),
}

/// The pixels of a 1-bit TIFF compressed with CCITT Group 3, which the image decoder doesn't read,
/// as RGBA. Compression 2 is modified Huffman coding with each row starting at a byte and no EOL
/// codes. Compression 3 is ITU-T T.4 coding with an EOL code before each row: one-dimensional rows,
/// or with T4Options bit 0, rows that a tag bit marks as one-dimensional or as coded against the
/// row above. Fill bits before an EOL are skipped. Rows in T.4's uncompressed mode (T4Options
/// bit 1) can't be decoded. Each strip or tile is coded on its own. White runs are 0 bits and black
/// runs are 1 bits, as libtiff writes them, and PhotometricInterpretation 0 shows 0 bits as white
/// and 1 shows them as black. FillOrder 2 (each byte's lowest bit first) is read too. Together, the
/// strips and tiles are read from at most as many bytes as the file has. `None` for other images;
/// an error when the pixels would take more than [`crate::MAX_RASTER_ALLOC`] as RGBA, a tile is
/// larger than that allowance, or a strip or tile can't be decoded or ends before its last row.
pub fn group3_tiff_rgba(bytes: &[u8]) -> Option<Result<image::RgbaImage, Group3Error>> {
    use craft_stream_codecs::{CcittErrorKind, CcittParams, decode_ccitt_raw};
    use craft_tiff::tags;
    let tiff = parse_tiff(bytes)?;
    let ifd = tiff.ifds.first()?;
    let info = ifd.image().ok()?;
    let compression = info.compression;
    let (w, h) = (info.width, info.height);
    if info.samples_per_pixel != 1 || info.bits_per_sample != [1] || !matches!(compression, 2 | 3) || w == 0 || h == 0 {
        return None;
    }
    // The level of a 0 bit (a white run): white when white is zero (0), black when black is zero (1).
    let zero = match info.photometric {
        0 => u8::MAX,
        1 => 0,
        _ => return None,
    };
    if u64::from(w) * u64::from(h) > crate::MAX_RASTER_ALLOC / 4 {
        return Some(Err(Group3Error::TooLarge { w, h }));
    }
    let t4_options = ifd.u32(292).unwrap_or(0);
    let lsb_first = ifd.u16(266) == Some(2);
    let k = if compression == 3 && t4_options & 1 != 0 { 2 } else { 0 };
    // The width and height of each strip or tile, and the tags of their offsets and byte counts.
    let tiled = ifd.contains(tags::TILE_WIDTH);
    let (cw, ch) = if tiled {
        let tw = ifd.u32(tags::TILE_WIDTH).filter(|&v| v > 0)?;
        let th = ifd.u32(tags::TILE_LENGTH).filter(|&v| v > 0)?;
        (tw, th)
    } else {
        let rows = ifd.u32(tags::ROWS_PER_STRIP).filter(|&v| v > 0).map_or(h, |r| r.min(h));
        (w, rows)
    };
    let damaged = |e: CcittErrorKind| Some(Err(Group3Error::Damaged(e)));
    // A tile is held to the allowance the image is held to; a strip is no larger than the image.
    if u64::from(cw) * u64::from(ch) > crate::MAX_RASTER_ALLOC / 4 {
        return damaged(CcittErrorKind::Overflow);
    }
    let offsets = &info.offsets;
    let counts = &info.byte_counts;
    let across = w.div_ceil(cw);
    let chunks = u64::from(across) * u64::from(h.div_ceil(ch));
    if chunks > offsets.len().min(counts.len()) as u64 {
        return damaged(CcittErrorKind::UnexpectedEof);
    }
    let mut gray = vec![zero; usize::try_from(u64::from(w) * u64::from(h)).ok()?];
    let mut budget = bytes.len();
    let stride = (cw as usize).div_ceil(8);
    for (i, (&offset, &count)) in (0..chunks).zip(offsets.iter().zip(&counts)) {
        let (x0, y0) = (u32::try_from(i % u64::from(across)).ok()? * cw, u32::try_from(i / u64::from(across)).ok()? * ch);
        let Some(start) = usize::try_from(offset).ok().filter(|&s| s <= bytes.len()) else {
            return damaged(CcittErrorKind::UnexpectedEof);
        };
        let len = usize::try_from(count).unwrap_or(usize::MAX).min(bytes.len() - start).min(budget);
        budget -= len;
        let data = bytes.get(start..start + len)?;
        let data: std::borrow::Cow<[u8]> = if lsb_first { data.iter().map(|b| b.reverse_bits()).collect() } else { data.into() };
        // A tile is coded whole; the last strip only has the image's remaining rows.
        let target_rows = if tiled { ch } else { ch.min(h - y0) };
        let params = CcittParams {
            k,
            columns: i64::from(cw),
            rows: i64::from(target_rows),
            end_of_block: compression == 3,
            end_of_line: compression == 3,
            encoded_byte_align: compression == 2,
            black_is_1: true,
        };
        let Ok(out) = decode_ccitt_raw(&data, &params, target_rows) else {
            return damaged(CcittErrorKind::Overflow);
        };
        if let Some(e) = out.error {
            return damaged(e);
        }
        if out.rows_decoded < target_rows {
            return damaged(CcittErrorKind::UnexpectedEof);
        }
        let max_y = target_rows.min(h - y0);
        let max_x = cw.min(w - x0);
        for y in 0..max_y {
            let src_row = &out.data[y as usize * stride..][..stride];
            let dst_off = (y0 + y) as usize * w as usize + x0 as usize;
            for x in 0..max_x as usize {
                let bit = (src_row[x / 8] >> (7 - (x % 8))) & 1;
                gray[dst_off + x] = if bit == 0 { zero } else { u8::MAX - zero };
            }
        }
    }
    let gray = image::GrayImage::from_raw(w, h, gray)?;
    Some(Ok(image::DynamicImage::ImageLuma8(gray).to_rgba8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_extra_tiff_u8(width: u32, height: u32, photometric: u16, spp: u16, extra: u16, data: &[u8]) -> Vec<u8> {
        use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value, tags};
        let mut ifd = IfdBuilder::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Long(vec![width]))
            .set(tags::IMAGE_LENGTH, Value::Long(vec![height]))
            .set(tags::BITS_PER_SAMPLE, Value::Short(vec![8; usize::from(spp)]))
            .set(tags::COMPRESSION, Value::Short(vec![1]))
            .set(tags::PHOTOMETRIC, Value::Short(vec![photometric]))
            .set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![spp]))
            .set(tags::PLANAR_CONFIGURATION, Value::Short(vec![1]))
            .set(tags::SAMPLE_FORMAT, Value::Short(vec![1; usize::from(spp)]))
            .set(tags::EXTRA_SAMPLES, Value::Short(vec![extra]))
            .set_image(ImageData::Strips { rows_per_strip: height, strips: vec![data.to_vec()] });
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap()
    }

    fn test_extra_tiff_u16(width: u32, height: u32, photometric: u16, spp: u16, extra: u16, data: &[u16]) -> Vec<u8> {
        use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value, tags};
        let raw: Vec<u8> = data.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut ifd = IfdBuilder::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Long(vec![width]))
            .set(tags::IMAGE_LENGTH, Value::Long(vec![height]))
            .set(tags::BITS_PER_SAMPLE, Value::Short(vec![16; usize::from(spp)]))
            .set(tags::COMPRESSION, Value::Short(vec![1]))
            .set(tags::PHOTOMETRIC, Value::Short(vec![photometric]))
            .set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![spp]))
            .set(tags::PLANAR_CONFIGURATION, Value::Short(vec![1]))
            .set(tags::SAMPLE_FORMAT, Value::Short(vec![1; usize::from(spp)]))
            .set(tags::EXTRA_SAMPLES, Value::Short(vec![extra]))
            .set_image(ImageData::Strips { rows_per_strip: height, strips: vec![raw] });
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap()
    }

    #[test]
    fn a_cmyk_tiff_with_alpha_reads_as_rgba() {
        // 2 × 1: full cyan opaque, then 100% K at half alpha (premultiplied: K 128, alpha 128).
        let bytes = test_extra_tiff_u8(2, 1, 5, 5, 1, &[255, 0, 0, 0, 255, 0, 0, 0, 128, 128]);
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let px = extra_samples_tiff_rgba(&bytes, naive).expect("CMYKA reads");
        assert_eq!(px.dimensions(), (2, 1));
        assert_eq!(px.get_pixel(0, 0).0, [0, 255, 255, 255], "cyan, opaque");
        assert_eq!(px.get_pixel(1, 0).0, [0, 0, 0, 128], "the premultiplied K divided back out: black at half alpha");
        // Not CMYKA: not this reader's.
        assert!(extra_samples_tiff_rgba(&ImageBlob::cmyk_tiff(&inks()).unwrap().bytes, naive).is_none());
    }

    /// A grayscale TIFF with an alpha channel, or with a channel that isn't one (#1466).
    #[test]
    fn a_grayscale_tiff_with_an_extra_channel_reads_as_rgba() {
        let gray = |extra: u16| {
            // White opaque, a gray of 64 at 80% alpha (premultiplied: 51), black with no alpha.
            let bytes = test_extra_tiff_u8(3, 1, 1, 2, extra, &[255, 255, 51, 204, 0, 0]);
            extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).expect("gray with an extra sample reads").into_raw()
        };
        assert_eq!(gray(1), [255, 255, 255, 255, 64, 64, 64, 204, 0, 0, 0, 0], "associated alpha, divided back out");
        assert_eq!(gray(2), [255, 255, 255, 255, 51, 51, 51, 204, 0, 0, 0, 0], "unassociated alpha");
        assert_eq!(gray(0), [255, 255, 255, 255, 51, 51, 51, 255, 0, 0, 0, 255], "a channel that isn't alpha: opaque");
    }

    /// 16-bit CMYK with alpha converts its 16-bit inks and alpha, and rounds only the result to 8
    /// bits; the high byte of each sample alone would give other pixels.
    #[test]
    fn a_16_bit_cmyk_tiff_with_alpha_converts_at_16_bits() {
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let read = |extra: u16, data: &[u16]| {
            let w = u32::try_from(data.len() / 5).unwrap();
            extra_samples_tiff_rgba(&test_extra_tiff_u16(w, 1, 5, 5, extra, data), naive).expect("CMYKA reads").into_raw()
        };
        // Unassociated: 129 of cyan, opaque; no ink at an alpha of 129; no ink at an alpha of 386.
        let straight = [129, 0, 0, 0, 65535, 0, 0, 0, 0, 129, 0, 0, 0, 0, 386];
        assert_eq!(read(2, &straight), [254, 255, 255, 255, 255, 255, 255, 1, 255, 255, 255, 2]);
        // Associated: cyan 128 at an alpha of 256 (half cyan); K 255 at an alpha of 511 (about half K).
        let premultiplied = [128, 0, 0, 0, 256, 0, 0, 0, 255, 511];
        assert_eq!(read(1, &premultiplied), [128, 255, 255, 1, 128, 128, 128, 2], "divided by the 16-bit alpha");
    }

    /// 16-bit gray with alpha converts at 16 bits as well.
    #[test]
    fn a_16_bit_grayscale_tiff_with_alpha_converts_at_16_bits() {
        let read = |extra: u16| {
            // A gray of 129, opaque, then 128 at an alpha of 256.
            let bytes = test_extra_tiff_u16(2, 1, 1, 2, extra, &[129, 65535, 128, 256]);
            extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).expect("gray with alpha reads").into_raw()
        };
        assert_eq!(read(2), [1, 1, 1, 255, 0, 0, 0, 1], "unassociated");
        assert_eq!(read(1), [1, 1, 1, 255, 128, 128, 128, 1], "associated, divided by the 16-bit alpha");
    }

    /// With associated alpha, a pixel whose alpha is zero has no color (TIFF 6.0, Associated
    /// Alpha Handling), whatever its color samples hold.
    #[test]
    fn a_pixel_with_no_associated_alpha_has_no_color() {
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let cmyk_bytes = test_extra_tiff_u8(1, 1, 5, 5, 1, &[255, 0, 0, 0, 0]);
        let cmyk = extra_samples_tiff_rgba(&cmyk_bytes, naive).expect("CMYKA reads");
        assert_eq!(cmyk.into_raw(), [255, 255, 255, 0], "no ink");
        let gray = extra_samples_tiff_rgba(&test_extra_tiff_u16(1, 1, 1, 2, 1, &[50_000, 0]), |_| [0.0; 3]).expect("gray reads");
        assert_eq!(gray.into_raw(), [0, 0, 0, 0], "a gray of zero");
    }

    /// A header claiming more tiles than can be counted: (2^32 − 1)² pixels in tiles of one pixel,
    /// times 2 planes.
    #[test]
    fn tiffs_with_more_tiles_than_can_be_counted_are_oversized() {
        let header = |planar: u32| {
            let mut f = b"II*\0\x08\0\0\0".to_vec();
            let entries = [(256u16, u32::MAX), (257, u32::MAX), (258, 8), (262, 1), (277, 2), (284, planar), (322, 1), (323, 1), (324, 0), (325, 0)];
            f.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
            for (tag, v) in entries {
                f.extend([tag.to_le_bytes(), 4u16.to_le_bytes()].concat());
                f.extend([1u32.to_le_bytes(), v.to_le_bytes()].concat());
            }
            f.extend([0; 4]);
            f
        };
        assert_eq!(oversized_tiff(&header(2)), Some((u32::MAX, u32::MAX)));
        assert_eq!(oversized_tiff(&header(1)), None, "one plane: (2^32 − 1)² tiles can be counted in 64 bits");
        assert!(tiff(&header(2)).is_none(), "not decoded");
    }

    /// A header claiming more strips than can be counted: 2^31 + 1 rows, a strip each, times 2
    /// planes. The decoder counts strips when the file has strip offsets, tile sizes or not, and
    /// reads a LONG8 value from where the entry points.
    #[test]
    fn tiffs_with_more_strips_than_can_be_counted_are_oversized() {
        // Each entry holds one value of type 3 (SHORT), 4 (LONG) or 16 (LONG8, after the directory).
        let header = |entries: &[(u16, u16, u32)]| {
            let after = 8 + 2 + 12 * entries.len() + 4;
            let mut f = b"II*\0\x08\0\0\0".to_vec();
            f.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
            let mut wide = vec![];
            for &(tag, ty, v) in entries {
                f.extend([tag.to_le_bytes(), ty.to_le_bytes()].concat());
                f.extend(1u32.to_le_bytes());
                if ty == 16 {
                    f.extend(u32::try_from(after + wide.len()).unwrap().to_le_bytes());
                    wide.extend(u64::from(v).to_le_bytes());
                } else {
                    f.extend(v.to_le_bytes());
                }
            }
            f.extend([0; 4]);
            f.extend(wide);
            f
        };
        let tall = 1u32 << 31 | 1;
        let strips = [(256, 4, 1), (257, 4, tall), (258, 3, 8), (262, 3, 1), (273, 4, 0), (277, 3, 2), (278, 4, 1), (279, 4, 0), (284, 3, 2)];
        assert_eq!(oversized_tiff(&header(&strips)), Some((1, tall)));
        let tile_sizes = [&strips[..], &[(322, 4, u32::MAX), (323, 4, u32::MAX)]].concat();
        assert_eq!(oversized_tiff(&header(&tile_sizes)), Some((1, tall)), "tile sizes without tile offsets");
        let mut long8 = strips;
        long8[1] = (257, 16, tall);
        assert_eq!(oversized_tiff(&header(&long8)), Some((1, tall)), "ImageLength as a LONG8");
        for h in [header(&tile_sizes), header(&long8)] {
            assert!(tiff(&h).is_none(), "not decoded");
        }
    }

    fn inks() -> Inks {
        Inks::new(3, 2, vec![0, 40, 100, 0, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 12, 34, 56, 78]).unwrap()
    }

    #[test]
    fn a_cmyk_tiff_keeps_its_ink_amounts_and_shows_as_rgb() {
        let b = ImageBlob::cmyk_tiff(&inks()).unwrap();
        assert_eq!(b.mime, "image/tiff");
        assert!(b.is_cmyk());
        assert_eq!(b.cmyk(), Some(inks()), "lossless");
        // Every image reader sees an RGB image: cyan is (0, 255, 255).
        let rgb = craft_image_codecs::decode(&b.bytes).unwrap().to_rgba8();
        assert_eq!(&rgb[4..7], &[0, 255, 255]);
    }

    #[test]
    fn a_header_gives_the_size_without_the_samples() {
        assert_eq!(ImageBlob::cmyk_tiff(&inks()).unwrap().header_size(), Some((3, 2)));
        // A CMYK TIFF of 6000 × 6000 pixels whose samples aren't there.
        let mut t = b"II*\0\x08\0\0\0\x07\0".to_vec();
        // Bits per sample: four values, after the entries (8 + 2 + 7 × 12 + 4 = 98).
        let entries = [
            (256u16, 4u16, 1u32, 6000u32),
            (257, 4, 1, 6000),
            (258, 3, 4, 98),
            (262, 3, 1, 5),
            (273, 4, 1, 106),
            (277, 3, 1, 4),
            (279, 4, 1, 144_000_000),
        ];
        for (tag, kind, count, value) in entries {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&kind.to_le_bytes());
            t.extend_from_slice(&count.to_le_bytes());
            t.extend_from_slice(&value.to_le_bytes());
        }
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend_from_slice(&[8, 0, 8, 0, 8, 0, 8, 0]);
        let big = ImageBlob::new("image/tiff", t);
        assert!(big.is_cmyk());
        assert_eq!(big.header_size(), Some((6000, 6000)));
        assert_eq!(big.cmyk(), None);
        let png = craft_image_codecs::encode(
            &CodecImage::from_raw(2, Format::Png, 2, ChannelLayout::Rgba, SampleType::U8, vec![0; 16]).unwrap(),
            &EncodeOptions::default(),
        )
        .unwrap();
        assert_eq!(ImageBlob::png(png).header_size(), None, "not a JPEG or TIFF");
    }

    #[test]
    fn a_cmyk_image_samples_its_ink_amounts() {
        let b = ImageBlob::cmyk_tiff(&inks()).unwrap();
        assert_eq!(b.sample_cmyk_object(0.5, 0.5, (3, 2), 1), Some([0, 40, 100, 0]));
        // An object of 6 × 4 pixels showing these 3 × 2: (5, 3) is the last pixel.
        assert_eq!(b.sample_cmyk_object(5.0, 3.0, (6, 4), 1), Some([12, 34, 56, 78]));
        // 3 × 3 around (1, 0), clipped to the image: all six pixels averaged.
        assert_eq!(b.sample_cmyk_object(1.5, 0.5, (3, 2), 3), Some([45, 55, 69, 56]));
        assert_eq!(b.sample_cmyk_object(3.5, 0.5, (3, 2), 1), None, "outside");
        let png = craft_image_codecs::encode(
            &CodecImage::from_raw(2, Format::Png, 2, ChannelLayout::Rgba, SampleType::U8, vec![0; 16]).unwrap(),
            &EncodeOptions::default(),
        )
        .unwrap();
        assert_eq!(ImageBlob::png(png).sample_cmyk_object(0.5, 0.5, (2, 2), 1), None, "an RGB image has no inks");
    }

    #[test]
    fn a_cmyk_jpeg_reads_back_as_ink_amounts() {
        // A flat CMYK JPEG: Adobe's inverted samples with its marker.
        let flat = [10u8, 40, 100, 20];
        let px: Vec<u8> = (0..16 * 16).flat_map(|_| flat).collect();
        let jpg = craft_image_codecs::encode(
            &CodecImage::from_raw(16, Format::Jpeg, 16, ChannelLayout::Cmyk, SampleType::U8, px).unwrap(),
            &EncodeOptions { jpeg_quality: 100, ..EncodeOptions::default() },
        )
        .unwrap();
        let b = ImageBlob::new("image/jpeg", jpg);
        assert_eq!(jpeg_components(&b.bytes), Some(4));
        assert!(b.is_cmyk());
        let back = b.cmyk().unwrap();
        assert_eq!((back.width, back.height), (16, 16));
        for p in back.data.as_chunks::<4>().0 {
            assert!(p.iter().zip(flat).all(|(a, b)| a.abs_diff(b) <= 2), "{p:?}");
        }
    }

    #[test]
    fn other_images_are_not_cmyk() {
        let png = craft_image_codecs::encode(
            &CodecImage::from_raw(2, Format::Png, 2, ChannelLayout::Rgba, SampleType::U8, vec![0; 16]).unwrap(),
            &EncodeOptions::default(),
        )
        .unwrap();
        let jpg = craft_image_codecs::encode(
            &CodecImage::from_raw(8, Format::Jpeg, 8, ChannelLayout::Rgb, SampleType::U8, vec![0; 8 * 8 * 3]).unwrap(),
            &EncodeOptions::default(),
        )
        .unwrap();
        for b in [ImageBlob::png(png), ImageBlob::new("image/jpeg", jpg), ImageBlob::new("image/tiff", b"II*\0junk".to_vec())] {
            assert!(!b.is_cmyk() && b.cmyk().is_none(), "{}", b.mime);
        }
        assert_eq!(Inks::new(2, 2, vec![0; 15]), None, "too few samples");
        assert_eq!(Inks::new(0, 2, vec![]), None, "empty");
        assert_eq!(Inks::new(1 << 14, 1 << 13, vec![]), None, "too large");
    }
}
