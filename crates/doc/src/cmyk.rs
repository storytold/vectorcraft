//! CMYK images: the ink amounts of image blobs that hold CMYK samples.
//!
//! Two encodings hold them: a four-component JPEG, whose samples are stored inverted with Adobe's
//! marker (the convention JPEG readers and PDF writers take a CMYK JPEG to follow), and a CMYK
//! TIFF (8 or 16 bits, ink amounts as they are, each pixel's inks together or one plane per ink).
//! [`ImageBlob::cmyk_tiff`] writes the latter, inks together, Deflate-compressed. Every other image
//! decodes to RGB(A) only.

use std::io::Cursor;
use std::sync::Arc;

use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

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
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
    d.decode_headers().ok()?;
    Some(d.info()?.components as usize)
}

fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")
}

/// What [`jpeg_frames`] found in a JPEG's headers.
enum Frames {
    /// Every frame header declares a size the check accepts, or there is none.
    Fit,
    /// The width and height of the first frame header the check refuses.
    Misfit(u16, u16),
    /// The steps ran past [`MAX_JPEG_HEADER_STEPS`].
    OverBudget,
}

/// Checks the width and height each 8-bit baseline, extended or progressive frame header in the
/// headers of a JPEG declares with `fits`. The JPEG is the bytes after its start-of-image marker:
/// `parts[0]` followed by `parts[1]`. The headers are read as the decoder reads them, up to the
/// first start-of-scan or end-of-image marker: every other marker is followed by its segment's
/// length, and bytes between segments are passed over, as are fill bytes (0xFF, and 0x00 after
/// one) before a marker. `steps` counts each byte looked at and each segment passed over.
fn jpeg_frames(parts: [&[u8]; 2], steps: &mut u64, fits: impl Fn(u16, u16) -> bool) -> Frames {
    let byte = |i: usize| match i.checked_sub(parts[0].len()) {
        None => parts[0].get(i).copied(),
        Some(j) => parts[1].get(j).copied(),
    };
    let u16_at = |i: usize| Some(u16::from_be_bytes([byte(i)?, byte(i.checked_add(1)?)?]));
    let mut p = 0usize;
    loop {
        *steps += 1;
        if *steps > MAX_JPEG_HEADER_STEPS {
            return Frames::OverBudget;
        }
        let Some(b) = byte(p) else { return Frames::Fit };
        p += 1;
        if b != 0xFF {
            continue;
        }
        while matches!(byte(p), Some(0xFF | 0)) {
            p += 1;
            *steps += 1;
            if *steps > MAX_JPEG_HEADER_STEPS {
                return Frames::OverBudget;
            }
        }
        let Some(marker) = byte(p) else { return Frames::Fit };
        p += 1;
        if matches!(marker, 0xDA | 0xD9) {
            return Frames::Fit;
        }
        // Length (2 bytes), then for a frame header its precision, height and width.
        let Some(len) = u16_at(p) else { return Frames::Fit };
        if matches!(marker, 0xC0..=0xC2)
            && byte(p.saturating_add(2)) == Some(8)
            && let (Some(h), Some(w)) = (u16_at(p.saturating_add(3)), u16_at(p.saturating_add(5)))
            && !fits(w, h)
        {
            return Frames::Misfit(w, h);
        }
        if len < 2 {
            return Frames::Fit;
        }
        p = p.saturating_add(usize::from(len));
    }
}

/// Most strips or tiles the first image of a TIFF may have: VectorCraft's own limit, 1/1024 of
/// [`crate::MAX_RASTER_ALLOC`] (524,288 in the web app, 2,097,152 on the desktop). The decoder
/// reads an offset and a byte count of 8 bytes each for every one into memory before anything else.
pub const MAX_TIFF_CHUNKS: u64 = crate::MAX_RASTER_ALLOC / 1024;

/// Most pixels the JPEG of a JPEG-compressed TIFF tile may declare when it declares more pixels
/// than the whole image has: VectorCraft's own limit (2048 × 2048, 16 MB as CMYK).
pub const MAX_JPEG_TILE_PIXELS: u64 = 1 << 22;

/// Most steps reading the JPEG headers of a TIFF's strips or tiles may take (a byte looked at, a
/// segment passed over or a strip offset read): VectorCraft's own limit.
const MAX_JPEG_HEADER_STEPS: u64 = 1 << 24;

/// The width and height of the first image of TIFF or BigTIFF `bytes` when it has more strips or
/// tiles than the `tiff` decoder can count: strips per plane times planes past `u32::MAX`, or tiles
/// per plane times planes past `usize::MAX`. A file that stores one plane per channel
/// (PlanarConfiguration 2) has a plane for each channel; other files have one. The decoder
/// multiplies these counts without a check, which panics where overflow checks are on. Such an
/// image, when it is at least a pixel wide, claims at least 2^32 samples.
/// It also gives them for an image with more than [`MAX_TIFF_CHUNKS`] strips or tiles, counted from
/// its size or from the number of values its StripOffsets and StripByteCounts tags hold, or its
/// TileOffsets and TileByteCounts tags for a tiled image. An image the decoder reads as strips (it
/// has both strip tables and neither tile table) counts strips, and one it reads as tiles counts
/// tiles; the decoder refuses an image with any other mix of these tags before it reads a table.
/// For a JPEG-compressed image (Compression 7) it gives the width and height a strip's or tile's
/// JPEG declares in a frame header (in the JPEGTables, or in the strip or tile before its first
/// scan) when they are wider or taller than the strip or tile, or, for a tile, when they make more
/// than [`MAX_JPEG_TILE_PIXELS`] pixels and more than the image has. The decoder allocates the
/// pixels the JPEG declares. It gives the image's width and height when reading those headers
/// takes more than 16,777,216 steps (a byte looked at, a segment passed over or a strip offset
/// read), when the strip or tile offsets are of a type other than BYTE, UNDEFINED, SHORT, LONG,
/// IFD, LONG8 or IFD8, and when the image's width and height, a striped image's RowsPerStrip or a
/// tiled image's tile size isn't a single value of one of those types.
/// Where a tag repeats, the value giving the most strips or tiles counts, and the JPEG check uses
/// the last entry of a field type the decoder knows, as the decoder does. `None` for other images
/// and for bytes that aren't a TIFF.
pub fn oversized_tiff(bytes: &[u8]) -> Option<(u32, u32)> {
    let le = bytes.starts_with(b"II");
    let head = bytes.get(..4)?;
    // A BigTIFF counts and points with 8 bytes where a TIFF uses 4, so its entries are 20 bytes.
    let big = if head == b"II*\0" || head == b"MM\0*" {
        false
    } else if head == b"II+\0" || head == b"MM\0+" {
        true
    } else {
        return None;
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = bytes.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let u64_at = |at: usize| -> Option<u64> {
        let b: [u8; 8] = bytes.get(at..at.checked_add(8)?)?.try_into().ok()?;
        Some(if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) })
    };
    let word_len: usize = if big { 8 } else { 4 };
    let word = |at: usize| if big { u64_at(at) } else { u32_at(at).map(u64::from) };
    // The bytes one value of each field type takes; 0 for types the decoder doesn't know.
    let width_of = |ty: u16| -> usize {
        match ty {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 | 16..=18 => 8,
            _ => 0,
        }
    };
    let ifd = usize::try_from(word(if big { 8 } else { 4 })?).ok()?;
    let entries = if big { u64_at(ifd)? } else { u64::from(u16_at(ifd)?) };
    let first = ifd.checked_add(if big { 8 } else { 2 })?;
    // (tag, field type, number of values, where the values are: in the entry when they fit in its
    // last word, otherwise where that word points)
    let entry = |i: usize| -> Option<(u16, u16, u64, Option<usize>)> {
        let e = first.checked_add(i.checked_mul(4 + 2 * word_len)?)?;
        let (tag, ty, count) = (u16_at(e)?, u16_at(e.checked_add(2)?)?, word(e.checked_add(4)?)?);
        let at = e.checked_add(4 + word_len)?;
        let fits = u64::try_from(width_of(ty)).ok()?.checked_mul(count).is_some_and(|n| n <= word_len as u64);
        let values = if fits { Some(at) } else { word(at).and_then(|p| usize::try_from(p).ok()) };
        Some((tag, ty, count, values))
    };
    // Value `i` of an entry's BYTE, UNDEFINED, SHORT, LONG, IFD, LONG8 or IFD8 values at `at`.
    let nth = |ty: u16, at: usize, i: u64| -> Option<u64> {
        let p = at.checked_add(usize::try_from(i).ok()?.checked_mul(width_of(ty))?)?;
        match ty {
            1 | 7 => bytes.get(p).map(|&b| u64::from(b)),
            3 => u16_at(p).map(u64::from),
            4 | 13 => u32_at(p).map(u64::from),
            16 | 18 => u64_at(p),
            _ => None,
        }
    };
    let (mut width, mut height, mut planes, mut planar) = (0u32, 0u32, 1u32, false);
    let (mut rows, mut tile_w, mut tile_h) = (None::<u32>, None::<u32>, None::<u32>);
    // The most values a strip table and a tile table hold.
    let (mut strip_chunks, mut tile_chunks) = (0u64, 0u64);
    // The decoder keeps the last entry of a tag whose field type it knows. These are the last such
    // entries (type, count, where) of the tags `LAST` lists.
    const LAST: [u16; 11] = [256, 257, 259, 278, 322, 323, 273, 279, 324, 325, 347];
    let mut last = [None::<(u16, u64, Option<usize>)>; LAST.len()];
    let least = |was: Option<u32>, v: u32| Some(was.map_or(v, |w| w.min(v)));
    // The entry's single BYTE, UNDEFINED, SHORT, LONG or IFD value, or its single LONG8 or IFD8
    // value when that fits in 32 bits.
    let single = |ty: u16, count: u64, at: Option<usize>| at.filter(|_| count == 1).and_then(|at| nth(ty, at, 0)).and_then(|v| u32::try_from(v).ok());
    for i in 0..usize::try_from(entries).unwrap_or(usize::MAX) {
        let Some((tag, ty, count, at)) = entry(i) else { break };
        match tag {
            273 | 279 => strip_chunks = strip_chunks.max(count),
            324 | 325 => tile_chunks = tile_chunks.max(count),
            _ => {}
        }
        if width_of(ty) > 0
            && let Some(slot) = LAST.iter().position(|&t| t == tag).and_then(|k| last.get_mut(k))
        {
            *slot = Some((ty, count, at));
        }
        let Some(v) = single(ty, count, at) else { continue };
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
    let tiled = match (tile_w, tile_h) {
        (Some(tw @ 1..), Some(th @ 1..)) => u128::from(width.div_ceil(tw)) * u128::from(height.div_ceil(th)) * planes,
        _ => 0,
    };
    let tiles = tiled > usize::MAX as u128;
    let rows = rows.unwrap_or(height);
    let striped = height.saturating_sub(1).checked_div(rows).map_or(0, |n| u128::from(n + 1) * planes);
    let strips = striped > u128::from(u32::MAX);
    if tiles || strips {
        return Some((width, height));
    }
    let last_of = |tag: u16| LAST.iter().position(|&t| t == tag).and_then(|k| last.get(k).copied().flatten());
    // The single value the decoder reads from the last entry of `tag`: `None` without one, and
    // `Some(None)` when that entry doesn't hold a single value read here.
    let value_of = |tag: u16| last_of(tag).map(|(ty, count, at)| single(ty, count, at));
    // The decoder reads strips when the image has both strip tables and neither tile table, tiles
    // in the opposite case, and refuses every other image before it reads a table.
    let layout = match [273, 279, 324, 325].map(|tag| last_of(tag).is_some()) {
        [true, true, false, false] => Some((273, striped.max(u128::from(strip_chunks)))),
        [false, false, true, true] => Some((324, tiled.max(u128::from(tile_chunks)))),
        _ => None,
    };
    let (offsets_tag, chunks) = layout?;
    if chunks > u128::from(MAX_TIFF_CHUNKS) {
        return Some((width, height));
    }
    if value_of(259) != Some(Some(7)) {
        return None;
    }
    // The JPEG check uses the sizes the decoder reads. A size it needs that isn't a single value
    // read here refuses the image.
    let (Some(Some(w_img)), Some(Some(h_img))) = (value_of(256), value_of(257)) else { return Some((width, height)) };
    let tile = offsets_tag == 324;
    // RowsPerStrip, which the decoder reads only for strips.
    let rows = if tile { Some(h_img) } else { value_of(278).unwrap_or(Some(h_img)) };
    let Some(rows) = rows else { return Some((width, height)) };
    let (tw, th) = match (value_of(322).flatten(), value_of(323).flatten()) {
        (Some(tw), Some(th)) => (tw, th),
        _ if tile => return Some((width, height)),
        _ => (0, 0),
    };
    // A strip is as wide as the image and RowsPerStrip tall (at most the image's height); a tile's
    // JPEG may be as large as the tile, which the edge tiles of the image overhang.
    let fits = |w: u16, h: u16| {
        let (w, h) = (u32::from(w), u32::from(h));
        if tile {
            let pixels = u64::from(w) * u64::from(h);
            w <= tw && h <= th && (pixels <= MAX_JPEG_TILE_PIXELS || pixels <= u64::from(w_img) * u64::from(h_img))
        } else {
            w <= w_img && h <= rows.min(h_img)
        }
    };
    // The decoder reads the JPEGTables after their start-of-image marker and without their last two
    // bytes, followed by each strip or tile after its first two bytes; without JPEGTables, each
    // strip or tile after its start-of-image marker.
    let tables = last_of(347)
        .and_then(|(_, count, at)| {
            let at = at?;
            let end = at.checked_add(usize::try_from(count).ok()?)?.min(bytes.len());
            bytes.get(at.checked_add(2)?..end.checked_sub(2)?)
        })
        .unwrap_or_default();
    let (ty, count, at) = last_of(offsets_tag)?;
    let at = at?;
    // The decoder also takes offsets of other types (a RATIONAL or ASCII value), which aren't read
    // here: such an image is refused.
    if !matches!(ty, 1 | 3 | 4 | 7 | 13 | 16 | 18) {
        return Some((width, height));
    }
    let mut steps = 0u64;
    for i in 0..count {
        steps += 1;
        let Some(at) = nth(ty, at, i).and_then(|o| usize::try_from(o).ok()) else { break };
        let part = bytes.get(at.saturating_add(2)..).unwrap_or_default();
        match jpeg_frames([tables, part], &mut steps, fits) {
            Frames::Fit => {}
            Frames::Misfit(w, h) => return Some((u32::from(w), u32::from(h))),
            Frames::OverBudget => return Some((width, height)),
        }
    }
    None
}

/// The color type of TIFF `bytes` and its decoder; `None` for [`oversized_tiff`] images too.
pub(crate) fn tiff(bytes: &[u8]) -> Option<(tiff::ColorType, tiff::decoder::Decoder<Cursor<&[u8]>>)> {
    if !is_tiff(bytes) || oversized_tiff(bytes).is_some() {
        return None;
    }
    let mut d = tiff::decoder::Decoder::new(Cursor::new(bytes)).ok()?;
    Some((d.colortype().ok()?, d))
}

/// The Orientation tag of TIFF `bytes`' first image, of any unsigned type; `None` for other images
/// and TIFFs without one. Reads only its header.
pub fn tiff_orientation(bytes: &[u8]) -> Option<u16> {
    let (_, mut d) = tiff(bytes)?;
    d.find_tag_unsigned::<u16>(tiff::tags::Tag::Orientation).ok().flatten()
}

/// The EXIF orientation of encoded image `bytes` when it turns or flips the pixels; `None` when
/// it leaves them as stored, or the image has none. A TIFF the image decoder can't open (one with
/// extra samples) has its Orientation tag read from its first image ([`tiff_orientation`]). Reads
/// only the image's header and metadata; `None` for an [`oversized_tiff`], which no decoder opens.
pub fn exif_orientation(bytes: &[u8]) -> Option<image::metadata::Orientation> {
    use image::ImageDecoder as _;
    use image::metadata::Orientation;
    if oversized_tiff(bytes).is_some() {
        return None;
    }
    let decoded = || image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_decoder().ok()?.orientation().ok();
    let tagged = || tiff_orientation(bytes).and_then(|v| Orientation::from_exif(u8::try_from(v).ok()?));
    decoded().or_else(tagged).filter(|o| *o != Orientation::NoTransforms)
}

/// Does the TIFF `d` reads store one plane per channel (PlanarConfiguration 2)?
fn planar(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>) -> bool {
    d.find_tag_unsigned::<u16>(tiff::tags::Tag::PlanarConfiguration).ok().flatten().is_some_and(|p| p != 1)
}

/// The samples of the `n` pixels of the TIFF `d` reads, `per` to a pixel (the number its color
/// type has), pixel by pixel and 8 bits each (16-bit samples to their high byte), whether the file
/// stores each pixel's samples together or one plane per channel. `None` when they don't decode to
/// that many samples, which includes images whose planes together need more than the decoder's
/// limit (256 MB of samples).
pub(crate) fn read_samples(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>, n: usize, per: usize) -> Option<Vec<u8>> {
    Some(match read_samples_at_depth(d, n, per)? {
        Samples::Eight(v) => v,
        Samples::Sixteen(v) => v.iter().map(|s| (s >> 8) as u8).collect(),
    })
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

/// [`read_samples`], keeping 16-bit samples at 16 bits.
fn read_samples_at_depth(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>, n: usize, per: usize) -> Option<Samples> {
    let mut buf = tiff::decoder::DecodingResult::U8(Vec::new());
    let layout = d.read_image_to_buffer(&mut buf).ok()?;
    match buf {
        tiff::decoder::DecodingResult::U8(v) => chunky(v, layout.planes, n, per).map(Samples::Eight),
        tiff::decoder::DecodingResult::U16(v) => chunky(v, layout.planes, n, per).map(Samples::Sixteen),
        _ => None,
    }
}

/// `data`, the samples of `n` pixels, `per` to a pixel, stored in `planes` planes, as the samples
/// pixel by pixel; `None` when it doesn't hold that many samples or a plane per sample.
fn chunky<T: Copy>(data: Vec<T>, planes: usize, n: usize, per: usize) -> Option<Vec<T>> {
    if Some(data.len()) != n.checked_mul(per) {
        return None;
    }
    if planes <= 1 {
        return Some(data);
    }
    if planes != per {
        return None;
    }
    let planes: Vec<&[T]> = data.chunks_exact(n).collect();
    Some((0..n).flat_map(|i| planes.iter().filter_map(move |p| p.get(i).copied())).collect())
}

/// The color space an ICC profile's header names (bytes 16 to 19, such as `RGB `, `GRAY` or
/// `CMYK`); `None` when `icc` is shorter than a header.
pub fn icc_color_space(icc: &[u8]) -> Option<[u8; 4]> {
    if icc.len() < 128 {
        return None;
    }
    icc.get(16..20)?.try_into().ok()
}

/// The ICC profile of TIFF tag 34675 (ICCProfile) of the image `d` reads; `None` without one.
fn tiff_icc(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>) -> Option<Vec<u8>> {
    d.get_tag_u8_vec(tiff::tags::Tag::IccProfile).ok().filter(|icc| !icc.is_empty())
}

/// The ICC profile of TIFF `bytes`' first image (tag 34675), read with the `tiff` decoder's own
/// limits, whatever the size of the image; `None` for other images and TIFFs without one.
pub fn tiff_icc_profile(bytes: &[u8]) -> Option<Vec<u8>> {
    let (_, mut d) = tiff(bytes)?;
    tiff_icc(&mut d)
}

/// The ICC profile of a JPEG, put together from all its numbered `ICC_PROFILE` APP2 segments;
/// `None` without one.
fn jpeg_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
    d.decode_headers().ok()?;
    d.icc_profile().filter(|icc| !icc.is_empty())
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
    let (kind, mut d) = tiff(bytes)?;
    if !matches!(kind, tiff::ColorType::CMYK(8 | 16)) {
        return None;
    }
    let (width, height) = d.dimensions().ok()?;
    let n = usize::try_from(pixels(width, height)?).ok()?;
    let icc = cmyk_icc(tiff_icc(&mut d));
    let samples = read_samples_at_depth(&mut d, n, 4)?;
    Some(CmykSource { width, height, samples, icc })
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
    use tiff::encoder::colortype::{CMYK8, CMYK16, ColorType};
    use tiff::encoder::{Compression, ImageEncoder, Rational, TiffEncoder, TiffKind, compression::DeflateLevel};
    use tiff::tags::{ResolutionUnit, Tag};
    fn tags<W: std::io::Write + std::io::Seek, C: ColorType, K: TiffKind>(
        img: &mut ImageEncoder<'_, W, C, K>,
        ppi: Option<(f64, f64)>,
        icc: Option<&[u8]>,
    ) -> Option<()> {
        if let Some((x, y)) = ppi.filter(|(x, y)| [x, y].iter().all(|v| v.is_finite() && **v > 0.0)) {
            // Ten-thousandths of a pixel per inch.
            let r = |v: f64| Rational { n: (v * 10_000.0).round() as u32, d: 10_000 };
            img.encoder().write_tag(Tag::XResolution, r(x)).ok()?;
            img.encoder().write_tag(Tag::YResolution, r(y)).ok()?;
            img.encoder().write_tag(Tag::ResolutionUnit, ResolutionUnit::Inch).ok()?;
        }
        if let Some(icc) = icc {
            img.encoder().write_tag(Tag::IccProfile, Undefined(icc)).ok()?;
        }
        Some(())
    }
    let mut out = Cursor::new(Vec::new());
    let mut enc = TiffEncoder::new(&mut out).ok()?.with_compression(Compression::Deflate(DeflateLevel::Balanced));
    match samples {
        SampleSlice::Eight(data) => {
            let mut img = enc.new_image::<CMYK8>(width, height).ok()?;
            tags(&mut img, ppi, icc)?;
            img.write_data(data).ok()?;
        }
        SampleSlice::Sixteen(data) => {
            let mut img = enc.new_image::<CMYK16>(width, height).ok()?;
            tags(&mut img, ppi, icc)?;
            img.write_data(data).ok()?;
        }
    }
    Some(out.into_inner())
}

/// Bytes written as a TIFF field of type UNDEFINED, the type of an embedded ICC profile.
struct Undefined<'a>(&'a [u8]);

impl tiff::encoder::TiffValue for Undefined<'_> {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: tiff::tags::Type = tiff::tags::Type::UNDEFINED;

    fn count(&self) -> usize {
        self.0.len()
    }

    fn data(&self) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(self.0)
    }
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
        jpeg_components(&self.bytes) == Some(4) || tiff(&self.bytes).is_some_and(|(t, _)| matches!(t, tiff::ColorType::CMYK(8 | 16)))
    }

    /// Does the image hold CMYK samples of 16 bits (a CMYK TIFF), which [`Self::cmyk`] gives at 8
    /// bits? Reads only its header.
    pub fn is_cmyk16(&self) -> bool {
        tiff(&self.bytes).is_some_and(|(t, _)| matches!(t, tiff::ColorType::CMYK(16)))
    }

    /// The width and height a JPEG or TIFF says it has, for any color type. Reads only its header.
    pub fn header_size(&self) -> Option<(u32, u32)> {
        let bytes = self.bytes.as_slice();
        if bytes.starts_with(&[0xFF, 0xD8]) {
            let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
            d.decode_headers().ok()?;
            let (w, h) = d.dimensions()?;
            return Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?));
        }
        tiff(bytes).and_then(|(_, mut d)| d.dimensions().ok())
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
        let (kind, mut d) = tiff(bytes)?;
        if !matches!(kind, tiff::ColorType::CMYK(8 | 16)) {
            return None;
        }
        cmyk_icc(tiff_icc(&mut d))
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

/// The ink amounts of a four-component JPEG (its samples inverted back; YCCK converted to CMYK
/// first, as JPEG readers do).
fn jpeg_inks(bytes: &[u8]) -> Option<Inks> {
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
    d.decode_headers().ok()?;
    let space = d.input_colorspace()?;
    if d.info()?.components != 4 || !matches!(space, ColorSpace::CMYK | ColorSpace::YCCK) {
        return None;
    }
    let (w, h) = d.dimensions()?;
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    pixels(w, h)?;
    d.set_options(DecoderOptions::default().jpeg_set_out_colorspace(space));
    let mut data = d.decode().ok()?;
    for p in data.as_chunks_mut::<4>().0 {
        if space == ColorSpace::YCCK {
            let [y, cb, cr] = [p[0], p[1], p[2]].map(f32::from);
            // libjpeg's YCCK → CMYK: the complement of the YCbCr colour's RGB.
            p[0] = (434.456 - y - 1.402 * cr).clamp(0.0, 255.0) as u8;
            p[1] = (119.541 - y + 0.344 * cb + 0.714 * cr).clamp(0.0, 255.0) as u8;
            p[2] = (481.816 - y - 1.772 * cb).clamp(0.0, 255.0) as u8;
        }
        for v in p.iter_mut() {
            *v = 255 - *v;
        }
    }
    Inks::new(w, h, data)
}

/// The ink amounts of a CMYK TIFF (16-bit samples to 8 bits), its inks stored together or one plane
/// per ink.
fn tiff_inks(bytes: &[u8]) -> Option<Inks> {
    let (kind, mut d) = tiff(bytes)?;
    let (w, h) = d.dimensions().ok()?;
    if !matches!(kind, tiff::ColorType::CMYK(8 | 16)) {
        return None;
    }
    let n = usize::try_from(pixels(w, h)?).ok()?;
    Inks::new(w, h, read_samples(&mut d, n, 4)?)
}

/// CMYK TIFF `bytes` that no orientation turns ([`turned_cmyk`] stores the others) as Open and
/// Place store them: as they are, or, when they store one plane per ink, which the image decoder
/// doesn't read, their inks together in a CMYK TIFF with the resolution `ppi`, at the 8 or 16 bits
/// the file has, carrying the file's CMYK ICC profile. `None` for other images and for CMYK TIFFs
/// whose inks [`ImageBlob::cmyk`] doesn't read.
pub fn stored_cmyk_tiff(bytes: &[u8], ppi: Option<(f64, f64)>) -> Option<Vec<u8>> {
    let source = tiff_source(bytes)?;
    let (_, mut d) = tiff(bytes)?;
    if !planar(&mut d) {
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
    use tiff::tags::Tag;
    let (kind, mut d) = tiff(bytes)?;
    let (w, h) = d.dimensions().ok()?;
    // 0: white is zero, 1: black is zero (the two grayscales).
    let photometric = d.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten();
    // ExtraSamples 1: associated alpha (premultiplied colour), 2: unassociated alpha, 0: not alpha.
    let extra = d.find_tag_unsigned_vec::<u16>(Tag::ExtraSamples).ok().flatten().and_then(|v| v.first().copied());
    // (samples in a pixel, how many of them are its colour, is the next one alpha?)
    let (samples, colour, alpha) = match kind {
        tiff::ColorType::CMYKA(8 | 16) => (5, 4, true),
        tiff::ColorType::Multiband { bit_depth: 8 | 16, num_samples: n @ 2.. } if matches!(photometric, Some(0 | 1)) => {
            (usize::from(n), 1, matches!(extra, Some(1 | 2)))
        }
        _ => return None,
    };
    let n = usize::try_from(pixels(w, h)?).ok()?;
    let data = read_samples_at_depth(&mut d, n, samples)?;
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
            _ if photometric == Some(0) => [1.0 - level(0); 3],
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
    Damaged(hayro_ccitt::DecodeError),
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
    use hayro_ccitt::{DecodeError, DecodeSettings, DecoderContext, EncodingMode};
    use tiff::tags::Tag;
    let (kind, mut d) = tiff(bytes)?;
    let compression = d.find_tag_unsigned::<u16>(Tag::Compression).ok().flatten()?;
    let (w, h) = d.dimensions().ok()?;
    if kind != tiff::ColorType::Gray(1) || !matches!(compression, 2 | 3) || w == 0 || h == 0 {
        return None;
    }
    // The level of a 0 bit (a white run): white when white is zero (0), black when black is zero (1).
    let zero = match d.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten() {
        Some(0) => u8::MAX,
        Some(1) => 0,
        _ => return None,
    };
    if u64::from(w) * u64::from(h) > crate::MAX_RASTER_ALLOC / 4 {
        return Some(Err(Group3Error::TooLarge { w, h }));
    }
    let t4_options = d.find_tag_unsigned::<u32>(Tag::Unknown(292)).ok().flatten().unwrap_or(0);
    let lsb_first = d.find_tag_unsigned::<u16>(Tag::FillOrder).ok().flatten() == Some(2);
    let encoding = if compression == 3 && t4_options & 1 != 0 {
        // Each row's tag bit says how it is coded.
        EncodingMode::Group3_2D { k: 2 }
    } else {
        EncodingMode::Group3_1D
    };
    // The width and height of each strip or tile, and the tags of their offsets and byte counts.
    let tiled = d.get_chunk_type() == tiff::decoder::ChunkType::Tile;
    let (cw, ch, offsets, counts) = if tiled {
        let tw = d.find_tag_unsigned::<u32>(Tag::TileWidth).ok().flatten().filter(|&v| v > 0)?;
        let th = d.find_tag_unsigned::<u32>(Tag::TileLength).ok().flatten().filter(|&v| v > 0)?;
        (tw, th, Tag::TileOffsets, Tag::TileByteCounts)
    } else {
        let rows = d.find_tag_unsigned::<u32>(Tag::RowsPerStrip).ok().flatten().filter(|&v| v > 0).map_or(h, |r| r.min(h));
        (w, rows, Tag::StripOffsets, Tag::StripByteCounts)
    };
    let damaged = |e: DecodeError| Some(Err(Group3Error::Damaged(e)));
    // A tile is held to the allowance the image is held to; a strip is no larger than the image.
    if u64::from(cw) * u64::from(ch) > crate::MAX_RASTER_ALLOC / 4 {
        return damaged(DecodeError::Overflow);
    }
    let offsets = d.find_tag_unsigned_vec::<u64>(offsets).ok().flatten().unwrap_or_default();
    let counts = d.find_tag_unsigned_vec::<u64>(counts).ok().flatten().unwrap_or_default();
    let across = w.div_ceil(cw);
    let chunks = u64::from(across) * u64::from(h.div_ceil(ch));
    if chunks > offsets.len().min(counts.len()) as u64 {
        return damaged(DecodeError::UnexpectedEof);
    }
    let mut gray = vec![zero; usize::try_from(u64::from(w) * u64::from(h)).ok()?];
    let mut budget = bytes.len();
    for (i, (&offset, &count)) in (0..chunks).zip(offsets.iter().zip(&counts)) {
        let (x0, y0) = (u32::try_from(i % u64::from(across)).ok()? * cw, u32::try_from(i / u64::from(across)).ok()? * ch);
        let Some(start) = usize::try_from(offset).ok().filter(|&s| s <= bytes.len()) else {
            return damaged(DecodeError::UnexpectedEof);
        };
        let len = usize::try_from(count).unwrap_or(usize::MAX).min(bytes.len() - start).min(budget);
        budget -= len;
        let data = bytes.get(start..start + len)?;
        let data: std::borrow::Cow<[u8]> = if lsb_first { data.iter().map(|b| b.reverse_bits()).collect() } else { data.into() };
        let settings = DecodeSettings {
            columns: cw,
            // A tile is coded whole; the last strip only has the image's remaining rows.
            rows: if tiled { ch } else { ch.min(h - y0) },
            end_of_block: compression == 3,
            end_of_line: compression == 3,
            rows_are_byte_aligned: compression == 2,
            encoding,
            invert_black: false,
        };
        let mut rows = Rows { out: &mut gray, width: w, height: h, x0, y0, x: 0, y: 0, zero };
        if let Err(e) = hayro_ccitt::decode(&data, &mut rows, &mut DecoderContext::new(settings)) {
            return damaged(e);
        }
        // The data ended, or six EOL codes came, before the strip's or tile's last row.
        if rows.y < settings.rows {
            return damaged(DecodeError::UnexpectedEof);
        }
    }
    let gray = image::GrayImage::from_raw(w, h, gray)?;
    Some(Ok(image::DynamicImage::ImageLuma8(gray).to_rgba8()))
}

/// Where a strip or tile of a Group 3 TIFF ([`group3_tiff_rgba`]) puts its decoded pixels: `out`
/// holds the image's levels row by row, `width` × `height`, and the strip or tile starts at (`x0`,
/// `y0`). Pixels past the image's right or bottom edge (a tile's padding) are dropped.
struct Rows<'a> {
    out: &'a mut [u8],
    width: u32,
    height: u32,
    x0: u32,
    y0: u32,
    /// The next pixel in the strip or tile.
    x: u64,
    y: u32,
    /// The level of a white run.
    zero: u8,
}

impl Rows<'_> {
    /// The next `n` pixels of the row, a white run's or a black run's.
    fn run(&mut self, white: bool, n: u64) {
        let level = if white { self.zero } else { u8::MAX - self.zero };
        let row = u64::from(self.y0) + u64::from(self.y);
        let left = u64::from(self.x0) + self.x;
        let right = left.saturating_add(n).min(u64::from(self.width));
        if row < u64::from(self.height) && left < right {
            let at = row * u64::from(self.width);
            if let (Ok(a), Ok(b)) = (usize::try_from(at + left), usize::try_from(at + right))
                && let Some(px) = self.out.get_mut(a..b)
            {
                px.fill(level);
            }
        }
        self.x = self.x.saturating_add(n);
    }
}

impl hayro_ccitt::Decoder for Rows<'_> {
    fn push_pixel(&mut self, white: bool) {
        self.run(white, 1);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.run(white, u64::from(chunk_count) * 8);
    }

    fn next_line(&mut self) {
        self.y = self.y.saturating_add(1);
        self.x = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cmyk_tiff_with_alpha_reads_as_rgba() {
        use tiff::encoder::{TiffEncoder, colortype::CMYKA8};
        // 2 × 1: full cyan opaque, then 100% K at half alpha (premultiplied: K 128, alpha 128).
        let mut file = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut file).unwrap();
        let mut img = enc.new_image::<CMYKA8>(2, 1).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, &[1u16][..]).unwrap();
        img.write_data(&[255, 0, 0, 0, 255, 0, 0, 0, 128, 128]).unwrap();
        let bytes = file.into_inner();
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let px = extra_samples_tiff_rgba(&bytes, naive).expect("CMYKA reads");
        assert_eq!(px.dimensions(), (2, 1));
        assert_eq!(px.get_pixel(0, 0).0, [0, 255, 255, 255], "cyan, opaque");
        assert_eq!(px.get_pixel(1, 0).0, [0, 0, 0, 128], "the premultiplied K divided back out: black at half alpha");
        // Not CMYKA: not this reader's.
        assert!(extra_samples_tiff_rgba(&ImageBlob::cmyk_tiff(&inks()).unwrap().bytes, naive).is_none());
    }

    /// Two 8-bit samples a pixel: gray and one extra sample, which the file's `ExtraSamples` names.
    struct GrayExtra8;
    impl tiff::encoder::colortype::ColorType for GrayExtra8 {
        type Inner = u8;
        const TIFF_VALUE: tiff::tags::PhotometricInterpretation = tiff::tags::PhotometricInterpretation::BlackIsZero;
        const BITS_PER_SAMPLE: &'static [u16] = &[8, 8];
        const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] = &[tiff::tags::SampleFormat::Uint; 2];
        fn horizontal_predict(row: &[u8], result: &mut Vec<u8>) {
            result.extend_from_slice(row);
        }
    }

    /// A grayscale TIFF with an alpha channel, or with a channel that isn't one (#1466): the image
    /// decoder reads neither.
    #[test]
    fn a_grayscale_tiff_with_an_extra_channel_reads_as_rgba() {
        let gray = |extra: u16| {
            let mut file = Cursor::new(Vec::new());
            let mut enc = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
            let mut img = enc.new_image::<GrayExtra8>(3, 1).unwrap();
            img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, &[extra][..]).unwrap();
            // White opaque, a gray of 64 at 80% alpha (premultiplied: 51), black with no alpha.
            img.write_data(&[255, 255, 51, 204, 0, 0]).unwrap();
            let bytes = file.into_inner();
            assert!(image::load_from_memory(&bytes).is_err(), "the image decoder reads it now: this reader may go");
            extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).expect("gray with an extra sample reads").into_raw()
        };
        assert_eq!(gray(1), [255, 255, 255, 255, 64, 64, 64, 204, 0, 0, 0, 0], "associated alpha, divided back out");
        assert_eq!(gray(2), [255, 255, 255, 255, 51, 51, 51, 204, 0, 0, 0, 0], "unassociated alpha");
        assert_eq!(gray(0), [255, 255, 255, 255, 51, 51, 51, 255, 0, 0, 0, 255], "a channel that isn't alpha: opaque");
    }

    /// Five 16-bit samples a pixel: the four inks and one extra sample, which the file's
    /// `ExtraSamples` names.
    struct Cmyka16;
    impl tiff::encoder::colortype::ColorType for Cmyka16 {
        type Inner = u16;
        const TIFF_VALUE: tiff::tags::PhotometricInterpretation = tiff::tags::PhotometricInterpretation::CMYK;
        const BITS_PER_SAMPLE: &'static [u16] = &[16; 5];
        const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] = &[tiff::tags::SampleFormat::Uint; 5];
        fn horizontal_predict(row: &[u16], result: &mut Vec<u16>) {
            result.extend_from_slice(row);
        }
    }

    /// Two 16-bit samples a pixel: gray and one extra sample.
    struct GrayExtra16;
    impl tiff::encoder::colortype::ColorType for GrayExtra16 {
        type Inner = u16;
        const TIFF_VALUE: tiff::tags::PhotometricInterpretation = tiff::tags::PhotometricInterpretation::BlackIsZero;
        const BITS_PER_SAMPLE: &'static [u16] = &[16, 16];
        const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] = &[tiff::tags::SampleFormat::Uint; 2];
        fn horizontal_predict(row: &[u16], result: &mut Vec<u16>) {
            result.extend_from_slice(row);
        }
    }

    /// A one-row TIFF of `C`'s pixels `data`, with the ExtraSamples value `extra`.
    fn extra_tiff<C: tiff::encoder::colortype::ColorType<Inner = u16>>(extra: u16, data: &[u16]) -> Vec<u8> {
        let samples = C::BITS_PER_SAMPLE.len();
        let mut file = Cursor::new(Vec::new());
        let mut enc = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        let mut img = enc.new_image::<C>(u32::try_from(data.len() / samples).unwrap(), 1).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, &[extra][..]).unwrap();
        img.write_data(data).unwrap();
        file.into_inner()
    }

    /// 16-bit CMYK with alpha converts its 16-bit inks and alpha, and rounds only the result to 8
    /// bits; the high byte of each sample alone would give other pixels.
    #[test]
    fn a_16_bit_cmyk_tiff_with_alpha_converts_at_16_bits() {
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let read = |extra: u16, data: &[u16]| extra_samples_tiff_rgba(&extra_tiff::<Cmyka16>(extra, data), naive).expect("CMYKA reads").into_raw();
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
            let bytes = extra_tiff::<GrayExtra16>(extra, &[129, 65535, 128, 256]);
            extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).expect("gray with alpha reads").into_raw()
        };
        assert_eq!(read(2), [1, 1, 1, 255, 0, 0, 0, 1], "unassociated");
        assert_eq!(read(1), [1, 1, 1, 255, 128, 128, 128, 1], "associated, divided by the 16-bit alpha");
    }

    /// With associated alpha, a pixel whose alpha is zero has no color (TIFF 6.0, Associated
    /// Alpha Handling), whatever its color samples hold.
    #[test]
    fn a_pixel_with_no_associated_alpha_has_no_color() {
        use tiff::encoder::{TiffEncoder, colortype::CMYKA8};
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let mut file = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut file).unwrap();
        let mut img = enc.new_image::<CMYKA8>(1, 1).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, &[1u16][..]).unwrap();
        img.write_data(&[255, 0, 0, 0, 0]).unwrap();
        let cmyk = extra_samples_tiff_rgba(&file.into_inner(), naive).expect("CMYKA reads");
        assert_eq!(cmyk.into_raw(), [255, 255, 255, 0], "no ink");
        let gray = extra_samples_tiff_rgba(&extra_tiff::<GrayExtra16>(1, &[50_000, 0]), |_| [0.0; 3]).expect("gray reads");
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
        assert_eq!(oversized_tiff(&header(1)), Some((u32::MAX, u32::MAX)), "one plane: countable, but past MAX_TIFF_CHUNKS");
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

    /// A little-endian TIFF, or BigTIFF when `big`, holding `data` right after its header (from
    /// byte 8, or 16 in a BigTIFF), then one directory of `entries` (tag, field type, values). An
    /// entry's values sit in the entry when they fit, otherwise after the directory.
    fn tiff_file(big: bool, entries: &[(u16, u16, Vec<u64>)], data: &[u8]) -> Vec<u8> {
        let (word, head) = if big { (8, 16) } else { (4, 8) };
        let put = |f: &mut Vec<u8>, v: u64, n: usize| f.extend(&v.to_le_bytes()[..n]);
        let mut f = if big { b"II+\0\x08\0\0\0".to_vec() } else { b"II*\0".to_vec() };
        let ifd = head + data.len();
        put(&mut f, ifd as u64, word);
        f.extend(data);
        put(&mut f, entries.len() as u64, if big { 8 } else { 2 });
        let after = f.len() + entries.len() * (4 + 2 * word) + word;
        let mut tail = vec![];
        for (tag, ty, values) in entries {
            let width = match ty {
                1 | 7 => 1,
                3 => 2,
                4 => 4,
                _ => 8,
            };
            f.extend([tag.to_le_bytes(), ty.to_le_bytes()].concat());
            put(&mut f, values.len() as u64, word);
            let mut v = vec![];
            for &x in values {
                put(&mut v, x, width);
            }
            if v.len() <= word {
                v.resize(word, 0);
                f.extend(v);
            } else {
                put(&mut f, (after + tail.len()) as u64, word);
                tail.extend(v);
            }
        }
        put(&mut f, 0, word);
        f.extend(tail);
        f
    }

    /// A JPEG of four components whose progressive frame header declares `w` × `h` pixels, with a
    /// scan header and no scan data.
    fn jpeg_header(w: u16, h: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xC2, 0, 20, 8];
        j.extend(h.to_be_bytes());
        j.extend(w.to_be_bytes());
        j.push(4);
        for id in 1..=4 {
            j.extend([id, 0x11, 0]);
        }
        j.extend([0xFF, 0xDA, 0, 14, 4, 1, 0, 2, 0, 3, 0, 4, 0, 0, 63, 0, 0xFF, 0xD9]);
        j
    }

    /// The directory of a 1 × 1 CMYK image with alpha, JPEG-compressed in one strip of `len` bytes
    /// at byte `at`.
    fn jpeg_strip_entries(at: u64, len: u64) -> Vec<(u16, u16, Vec<u64>)> {
        vec![
            (256, 4, vec![1]),
            (257, 4, vec![1]),
            (258, 3, vec![8; 5]),
            (259, 3, vec![7]),
            (262, 3, vec![5]),
            (273, 4, vec![at]),
            (277, 3, vec![5]),
            (278, 4, vec![1]),
            (279, 4, vec![len]),
            (338, 3, vec![2]),
        ]
    }

    /// A strip's JPEG that declares more pixels than the strip holds would have the decoder
    /// allocate them: a 1 × 1 image whose JPEG claims 16384 × 16384 is refused before any decoder
    /// opens it, in a TIFF or a BigTIFF, with fill bytes or other bytes before the frame header,
    /// and with the frame header in the JPEGTables.
    #[test]
    fn a_jpeg_strip_declaring_more_than_its_strip_is_refused() {
        let big = jpeg_header(16384, 16384);
        let file = tiff_file(false, &jpeg_strip_entries(8, big.len() as u64), &big);
        assert_eq!(oversized_tiff(&file), Some((16384, 16384)));
        assert!(tiff(&file).is_none(), "not decoded");
        let bigtiff = tiff_file(true, &jpeg_strip_entries(16, big.len() as u64), &big);
        assert_eq!(oversized_tiff(&bigtiff), Some((16384, 16384)), "BigTIFF");
        // The decoder reads the last ImageWidth and ImageLength: 1 × 1 after 16384 × 16384, in one
        // strip without RowsPerStrip.
        let mut entries = jpeg_strip_entries(8, big.len() as u64);
        entries.retain(|e| e.0 != 278);
        entries.splice(0..0, [(256, 4, vec![16384]), (257, 4, vec![16384])]);
        assert_eq!(oversized_tiff(&tiff_file(false, &entries, &big)), Some((16384, 16384)), "repeated sizes");
        // StripOffsets as a BYTE, which the decoder reads, and as a RATIONAL, which it reads as two
        // offsets.
        for (ty, expected) in [(1, (16384, 16384)), (5, (1, 1))] {
            let mut entries = jpeg_strip_entries(8, big.len() as u64);
            for e in &mut entries {
                if e.0 == 273 {
                    e.1 = ty;
                }
            }
            assert_eq!(oversized_tiff(&tiff_file(false, &entries, &big)), Some(expected), "StripOffsets of type {ty}");
        }
        let one = jpeg_header(1, 1);
        assert_eq!(oversized_tiff(&tiff_file(false, &jpeg_strip_entries(8, one.len() as u64), &one)), None, "a JPEG as large as its strip");
        let wide = jpeg_header(2, 1);
        assert_eq!(oversized_tiff(&tiff_file(false, &jpeg_strip_entries(8, wide.len() as u64), &wide)), Some((2, 1)), "one pixel too wide");
        for (what, prefix) in [("fill bytes", &[0xFF, 0x00, 0xFF][..]), ("other bytes", &[0x12, 0x34][..])] {
            let mut j = big[..2].to_vec();
            j.extend(prefix);
            j.extend(&big[2..]);
            let file = tiff_file(false, &jpeg_strip_entries(8, j.len() as u64), &j);
            assert_eq!(oversized_tiff(&file), Some((16384, 16384)), "{what} before the frame header");
        }
        // The JPEGTables hold the frame header; the strip holds only the scan.
        let tables = [&big[..24], &[0xFF, 0xD9][..]].concat();
        let scan = [&[0xFF, 0xD8][..], &big[24..]].concat();
        let mut entries = jpeg_strip_entries(8 + tables.len() as u64, scan.len() as u64);
        entries.push((347, 7, tables.iter().map(|&b| u64::from(b)).collect()));
        let file = tiff_file(false, &entries, &[tables, scan].concat());
        assert_eq!(oversized_tiff(&file), Some((16384, 16384)), "frame header in the JPEGTables");
        // An encoder's 8 × 8 RGB JPEG, with its own segments before the frame header, in an
        // 8 × 8 image of one strip, and of strips 4 rows tall.
        let mut real = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut real).encode(&[90; 8 * 8 * 3], 8, 8, image::ExtendedColorType::Rgb8).unwrap();
        let rgb = |rows: u64| {
            let entries = [
                (256, 4, vec![8]),
                (257, 4, vec![8]),
                (258, 3, vec![8; 3]),
                (259, 3, vec![7]),
                (262, 3, vec![2]),
                (273, 4, vec![8]),
                (277, 3, vec![3]),
                (278, 4, vec![rows]),
                (279, 4, vec![real.len() as u64]),
            ];
            oversized_tiff(&tiff_file(false, &entries, &real))
        };
        assert_eq!(rgb(8), None, "an encoder's JPEG as large as its strip");
        assert_eq!(rgb(4), Some((8, 8)), "an encoder's JPEG taller than its strip");
    }

    /// A tile's JPEG may be as large as the tile, which overhangs the image, and when it is more
    /// than MAX_JPEG_TILE_PIXELS, no larger than the image.
    #[test]
    fn a_jpeg_tile_declaring_more_than_its_tile_is_refused() {
        let tiled_with = |tile: u64, j: &[u8], rows: Vec<u64>| {
            let mut entries: Vec<_> = jpeg_strip_entries(0, 0).into_iter().filter(|e| !matches!(e.0, 273 | 278 | 279)).collect();
            if !rows.is_empty() {
                entries.push((278, 4, rows));
            }
            entries.extend([(322, 4, vec![tile]), (323, 4, vec![tile]), (324, 4, vec![8]), (325, 4, vec![j.len() as u64])]);
            oversized_tiff(&tiff_file(false, &entries, j))
        };
        let tiled = |tile: u64, j: &[u8]| tiled_with(tile, j, vec![]);
        assert_eq!(tiled_with(16, &jpeg_header(16, 16), vec![1, 1]), None, "RowsPerStrip, unread in a tiled image");
        assert_eq!(tiled(16, &jpeg_header(16, 16)), None, "as large as the tile");
        assert_eq!(tiled(16, &jpeg_header(17, 16)), Some((17, 16)), "wider than the tile");
        assert_eq!(tiled(16, &jpeg_header(16, 17)), Some((16, 17)), "taller than the tile");
        assert_eq!(tiled(4096, &jpeg_header(2048, 2048)), None, "MAX_JPEG_TILE_PIXELS");
        assert_eq!(tiled(4096, &jpeg_header(2048, 2049)), Some((2048, 2049)), "past MAX_JPEG_TILE_PIXELS and the image");
    }

    /// The decoder reads every strip's offset and byte count before anything else: an image with
    /// more than MAX_TIFF_CHUNKS strips, counted from its size or from its tables, is refused.
    #[test]
    fn tiffs_with_more_strips_than_their_tables_may_hold_are_refused() {
        let file = |height: u64, count: u32| {
            let entries = [
                (256, 4, vec![1]),
                (257, 4, vec![height]),
                (258, 3, vec![8]),
                (262, 3, vec![1]),
                (273, 4, vec![8]),
                (278, 4, vec![1]),
                (279, 4, vec![1]),
            ];
            let mut f = tiff_file(false, &entries, &[0]);
            // StripOffsets' count, its values left out of the file.
            let at = 8 + 1 + 2 + 4 * 12 + 4;
            f[at..at + 4].copy_from_slice(&count.to_le_bytes());
            f
        };
        let most = MAX_TIFF_CHUNKS;
        assert_eq!(oversized_tiff(&file(most, 1)), None, "MAX_TIFF_CHUNKS rows of a strip each");
        assert_eq!(oversized_tiff(&file(most + 1, 1)), Some((1, u32::try_from(most + 1).unwrap())), "one strip more");
        assert_eq!(oversized_tiff(&file(1, u32::try_from(most).unwrap())), None, "MAX_TIFF_CHUNKS offsets");
        assert_eq!(oversized_tiff(&file(1, u32::try_from(most + 1).unwrap())), Some((1, 1)), "one offset more");
        assert!(tiff(&file(1, 1 << 25)).is_none(), "not decoded");
        // Tile sizes without tile tables: the decoder reads the strips and never counts the
        // 4,194,304 tiles of a pixel the sizes give.
        let strips_with_tile_sizes = |offsets: Option<u16>| {
            let mut entries = vec![
                (256, 4, vec![2048]),
                (257, 4, vec![2048]),
                (258, 3, vec![8]),
                (262, 3, vec![1]),
                (273, 4, vec![8]),
                (279, 4, vec![2048 * 2048]),
                (322, 4, vec![1]),
                (323, 4, vec![1]),
            ];
            entries.extend(offsets.map(|tag| (tag, 4, vec![8])));
            entries.sort_by_key(|e| e.0);
            oversized_tiff(&tiff_file(false, &entries, &[0]))
        };
        assert_eq!(strips_with_tile_sizes(None), None, "a strip image with tile sizes of one pixel");
        assert_eq!(strips_with_tile_sizes(Some(324)), None, "strip and tile tables: the decoder refuses it unread");
    }

    /// Strips whose JPEG headers take more than MAX_JPEG_HEADER_STEPS steps to read refuse the
    /// file: here every strip points at the same 1 MB without a marker.
    #[test]
    fn jpeg_strips_that_take_too_long_to_read_are_refused() {
        let file = |strips: u64| {
            let mut entries = jpeg_strip_entries(0, 0);
            entries.retain(|e| !matches!(e.0, 257 | 273 | 279));
            entries.extend([(257, 4, vec![strips]), (273, 4, vec![8; strips as usize]), (279, 4, vec![1 << 20; strips as usize])]);
            tiff_file(false, &entries, &std::iter::repeat_n(0, 1 << 20).collect::<Vec<u8>>())
        };
        assert_eq!(oversized_tiff(&file(8)), None, "about 8 million steps");
        assert_eq!(oversized_tiff(&file(17)), Some((1, 17)), "about 17 million steps");
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
        let rgb = image::load_from_memory(&b.bytes).unwrap().to_rgb8();
        assert_eq!(rgb.get_pixel(1, 0).0, [0, 255, 255]);
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
        let mut png = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
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
        let mut png = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(ImageBlob::png(png).sample_cmyk_object(0.5, 0.5, (2, 2), 1), None, "an RGB image has no inks");
    }

    #[test]
    fn a_cmyk_jpeg_reads_back_as_ink_amounts() {
        // A flat CMYK JPEG: Adobe's inverted samples with its marker.
        let flat = [10u8, 40, 100, 20];
        let px: Vec<u8> = (0..16 * 16).flat_map(|_| flat).collect();
        let mut jpg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpg, 100).encode(&px, 16, 16, jpeg_encoder::ColorType::Cmyk).unwrap();
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
        let mut png = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut jpg = Vec::new();
        image::RgbImage::new(8, 8).write_to(&mut Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
        for b in [ImageBlob::png(png), ImageBlob::new("image/jpeg", jpg), ImageBlob::new("image/tiff", b"II*\0junk".to_vec())] {
            assert!(!b.is_cmyk() && b.cmyk().is_none(), "{}", b.mime);
        }
        assert_eq!(Inks::new(2, 2, vec![0; 15]), None, "too few samples");
        assert_eq!(Inks::new(0, 2, vec![]), None, "empty");
        assert_eq!(Inks::new(1 << 14, 1 << 13, vec![]), None, "too large");
    }
}
