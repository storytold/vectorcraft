//! CMYK images: the ink amounts of image blobs that hold CMYK samples.
//!
//! Two encodings hold them: a four-component JPEG, whose samples are stored inverted with Adobe's
//! marker (the convention JPEG readers and PDF writers take a CMYK JPEG to follow), and a CMYK
//! TIFF (8 or 16 bits, ink amounts as they are, each pixel's inks together or one plane per ink).
//! [`ImageBlob::cmyk_tiff`] writes the latter, inks together, Deflate-compressed. Every other image
//! decodes to RGB(A) only.

use std::io::Cursor;
use std::sync::Arc;

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
    craft_image_codecs::jpeg_inks::jpeg_components(bytes)
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

/// The decoder of TIFF `bytes`' first image; `None` for other bytes and [`oversized_tiff`] images.
fn tiff_decoder(bytes: &[u8]) -> Option<tiff::decoder::Decoder<Cursor<&[u8]>>> {
    if !is_tiff(bytes) || oversized_tiff(bytes).is_some() {
        return None;
    }
    tiff::decoder::Decoder::new(Cursor::new(bytes)).ok()
}

/// The color type of TIFF `bytes` and its decoder; `None` for [`oversized_tiff`] images too.
pub(crate) fn tiff(bytes: &[u8]) -> Option<(tiff::ColorType, tiff::decoder::Decoder<Cursor<&[u8]>>)> {
    let mut d = tiff_decoder(bytes)?;
    Some((d.colortype().ok()?, d))
}

/// The Orientation tag of TIFF `bytes`' first image, of any unsigned type; `None` for other images
/// and TIFFs without one. Reads only its header, for any color type.
pub fn tiff_orientation(bytes: &[u8]) -> Option<u16> {
    tiff_decoder(bytes)?.find_tag_unsigned::<u16>(tiff::tags::Tag::Orientation).ok().flatten()
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

/// TIFF bytes as a reader sees them with some of them replaced: each patch is an offset and the
/// bytes read there instead, at most four, in order of offset without overlaps. The file is not
/// copied.
struct Patched<'a> {
    bytes: Cursor<&'a [u8]>,
    patches: Vec<(usize, &'static [u8])>,
}

impl std::io::Read for Patched<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = usize::try_from(self.bytes.position()).unwrap_or(usize::MAX);
        let read = std::io::Read::read(&mut self.bytes, buf)?;
        let got = buf.get_mut(..read).unwrap_or_default();
        let end = start.saturating_add(read);
        // The patches are in order and apart, and each is at most four bytes long.
        let first = self.patches.partition_point(|&(at, _)| at.saturating_add(4) <= start);
        for &(at, with) in self.patches.get(first..).unwrap_or_default().iter().take_while(|&&(at, _)| at < end) {
            for (k, &b) in with.iter().enumerate() {
                if let Some(slot) = at.checked_add(k).and_then(|i| i.checked_sub(start)).and_then(|i| got.get_mut(i)) {
                    *slot = b;
                }
            }
        }
        Ok(read)
    }
}

impl std::io::Seek for Patched<'_> {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.bytes, to)
    }
}

/// TIFF `bytes` read with the PhotometricInterpretation (tag 262) of their first image set to 1
/// (black is zero) in each of its entries that holds one SHORT or LONG value. The decoder gives
/// every sample of a grayscale image's pixels, in the order the file stores them, where it gives a
/// CMYK image's inks and at most the first extra sample, and doesn't read a white-is-zero image
/// with extra samples. `None` for bytes that aren't a TIFF and when the first image has no such
/// entry.
fn as_black_is_zero(bytes: &[u8]) -> Option<Patched<'_>> {
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
    let mut patches = Vec::new();
    for i in 0..usize::from(u16_at(ifd)?) {
        let e = ifd.checked_add(i.checked_mul(12)?.checked_add(2)?)?;
        let (Some(tag), Some(ty), Some(count)) = (u16_at(e), u16_at(e.checked_add(2)?), u32_at(e.checked_add(4)?)) else { break };
        if tag != 262 || count != 1 {
            continue;
        }
        // The value sits in the entry, in the file's byte order.
        let one: &'static [u8] = match (ty, le) {
            (3, true) => &[1, 0],
            (3, false) => &[0, 1],
            (4, true) => &[1, 0, 0, 0],
            (4, false) => &[0, 0, 0, 1],
            _ => continue,
        };
        let at = e.checked_add(8)?;
        bytes.get(at..at.checked_add(one.len())?)?;
        patches.push((at, one));
    }
    (!patches.is_empty()).then(|| Patched { bytes: Cursor::new(bytes), patches })
}

/// The ExtraSamples values of the image `d` reads, one for each sample after its color, in order:
/// 0 not alpha, 1 associated alpha, 2 unassociated alpha. Empty without the tag.
fn extra_samples(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>) -> Vec<u16> {
    d.find_tag_unsigned_vec::<u16>(tiff::tags::Tag::ExtraSamples).ok().flatten().unwrap_or_default()
}

/// Does `kind`, the color type of the TIFF `d` reads, hold inks that Open and Place keep: CMYK of 8
/// or 16 bits, with no extra sample that the file names as alpha? The decoder gives a CMYK color
/// type to a file whose alpha follows another extra sample. [`ImageBlob::cmyk`] reads the inks of
/// such a file all the same: a document saved with one as a CMYK image keeps it.
fn cmyk_inks(kind: tiff::ColorType, d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>) -> bool {
    matches!(kind, tiff::ColorType::CMYK(8 | 16)) && !extra_samples(d).iter().any(|e| matches!(e, 1 | 2))
}

/// Most samples a pixel of a TIFF is read with, its color and extra samples included, as the
/// file's SamplesPerPixel counts them: VectorCraft's own limit.
const MAX_EXTRA_SAMPLES_PIXEL: usize = 64;

/// The bytes of the row the decoder of the TIFF `d` reads takes before it reads each strip or tile
/// of a file that stores a pixel's samples together and has more of them (its SamplesPerPixel)
/// than the decoder gives: one row of the strip or tile, every sample of each pixel. 0 for other
/// files. `None` when the file has more than [`MAX_EXTRA_SAMPLES_PIXEL`] samples a pixel, when its
/// samples aren't unsigned integers (a SampleFormat other than 1, which the readers don't take, and
/// which the floating-point predictor needs) and when its color type isn't read. Reads only the
/// file's tags.
fn row_buffer<R: std::io::Read + std::io::Seek>(d: &mut tiff::decoder::Decoder<R>) -> Option<u64> {
    use tiff::tags::Tag;
    let kind = d.colortype().ok()?;
    let all = d.find_tag_unsigned::<u16>(Tag::SamplesPerPixel).ok()?.unwrap_or(1);
    if usize::from(all) > MAX_EXTRA_SAMPLES_PIXEL {
        return None;
    }
    if d.find_tag_unsigned_vec::<u16>(Tag::SampleFormat).ok()?.is_some_and(|f| f.iter().any(|&f| f != 1)) {
        return None;
    }
    if all <= kind.num_samples() || planar(d) {
        return Some(0);
    }
    // A strip is as wide as the image, a tile as its TileWidth.
    let width = match d.get_chunk_type() {
        tiff::decoder::ChunkType::Tile => d.find_tag_unsigned::<u32>(Tag::TileWidth).ok()??,
        tiff::decoder::ChunkType::Strip => d.dimensions().ok()?.0,
    };
    u64::from(width).checked_mul(u64::from(all))?.checked_mul(u64::from(kind.bit_depth().div_ceil(8)))
}

/// `d`, the decoder of a TIFF of `file` bytes, set to decode `n` pixels of `per` samples of `bits`
/// bits each into RGBA. `None` when the file has more than [`MAX_EXTRA_SAMPLES_PIXEL`] samples a
/// pixel, when its samples aren't unsigned integers (a SampleFormat other than 1), and when the
/// decoded samples, the copy that puts a planar file's samples pixel by pixel, the row the decoder
/// reads each strip or tile into ([`row_buffer`]) and the RGBA pixels together take more than
/// [`crate::MAX_RASTER_ALLOC`]. The decoder may then decode no more than those samples, and read no
/// strip or tile of more compressed bytes than the file holds (nor more than its own limit of
/// 128 MiB).
fn with_room<R: std::io::Read + std::io::Seek>(
    mut d: tiff::decoder::Decoder<R>,
    file: usize,
    n: usize,
    per: usize,
    bits: u8,
) -> Option<tiff::decoder::Decoder<R>> {
    if per > MAX_EXTRA_SAMPLES_PIXEL {
        return None;
    }
    let row = row_buffer(&mut d)?;
    let need = n.checked_mul(per)?.checked_mul(usize::from(bits.div_ceil(8)))?;
    let copies = if per > 1 && planar(&mut d) { 2 } else { 1 };
    let peak = u64::try_from(need).ok()?.checked_mul(copies)?.checked_add(u64::try_from(n).ok()?.checked_mul(4)?)?.checked_add(row)?;
    if peak > crate::MAX_RASTER_ALLOC {
        return None;
    }
    let mut limits = tiff::decoder::Limits::default();
    limits.decoding_buffer_size = need;
    limits.intermediate_buffer_size = limits.intermediate_buffer_size.min(file);
    Some(d.with_limits(limits))
}

/// Does the TIFF `d` reads store one plane per channel (PlanarConfiguration 2)?
fn planar<R: std::io::Read + std::io::Seek>(d: &mut tiff::decoder::Decoder<R>) -> bool {
    d.find_tag_unsigned::<u16>(tiff::tags::Tag::PlanarConfiguration).ok().flatten().is_some_and(|p| p != 1)
}

/// The samples of the `n` pixels of the TIFF `d` reads, `per` to a pixel (the number its color
/// type has), pixel by pixel and 8 bits each (16-bit samples rounded to the nearest), whether the
/// file stores each pixel's samples together or one plane per channel. `None` when they don't decode to
/// that many samples, which includes images whose planes together need more than the decoder's
/// limit (256 MiB of samples), and, before any decoding, files with more than
/// [`MAX_EXTRA_SAMPLES_PIXEL`] samples a pixel, files whose samples aren't unsigned integers (a
/// SampleFormat other than 1) and ones whose samples with the row the decoder reads each strip or
/// tile into ([`row_buffer`]) take more than [`crate::MAX_RASTER_ALLOC`].
pub(crate) fn read_samples(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>, n: usize, per: usize) -> Option<Vec<u8>> {
    Some(match read_samples_at_depth(d, n, per)? {
        Samples::Eight(v) => v,
        Samples::Sixteen(v) => v.iter().map(|&s| u8::try_from((u32::from(s) + 128) / 257).unwrap_or(u8::MAX)).collect(),
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
fn read_samples_at_depth<R: std::io::Read + std::io::Seek>(d: &mut tiff::decoder::Decoder<R>, n: usize, per: usize) -> Option<Samples> {
    // Checked from the tags, before anything is decoded.
    let row = row_buffer(d)?;
    let bytes = u64::from(d.colortype().ok()?.bit_depth().div_ceil(8));
    let need = u64::try_from(n).ok()?.checked_mul(u64::try_from(per).ok()?)?.checked_mul(bytes)?;
    if need.checked_add(row)? > crate::MAX_RASTER_ALLOC {
        return None;
    }
    decode_samples(d, n, per)
}

/// [`read_samples_at_depth`] without its checks, for a decoder that [`with_room`] made. Its tags
/// can't be read once its limits are set: the decoder holds a tag's values to its decoding limit.
fn decode_samples<R: std::io::Read + std::io::Seek>(d: &mut tiff::decoder::Decoder<R>, n: usize, per: usize) -> Option<Samples> {
    let mut buf = tiff::decoder::DecodingResult::U8(Vec::new());
    let layout = d.read_image_to_buffer(&mut buf).ok()?;
    match buf {
        tiff::decoder::DecodingResult::U8(v) => chunky(v, layout.planes, n, per).map(Samples::Eight),
        tiff::decoder::DecodingResult::U16(v) => chunky(v, layout.planes, n, per).map(Samples::Sixteen),
        _ => None,
    }
}

/// `data`, the samples of `n` pixels, `per` to a pixel, stored in `planes` planes, as the samples
/// pixel by pixel; `None` when it doesn't hold that many samples or a plane per sample. The copy
/// of a planar file's samples is allocated once, at its length.
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
    let mut out = Vec::with_capacity(data.len());
    for i in 0..n {
        for p in &planes {
            out.push(*p.get(i)?);
        }
    }
    Some(out)
}

/// The color space an ICC profile's header names (bytes 16 to 19, such as `RGB `, `GRAY` or
/// `CMYK`); `None` when `icc` is shorter than a header.
pub fn icc_color_space(icc: &[u8]) -> Option<[u8; 4]> {
    if icc.len() < 128 {
        return None;
    }
    icc.get(16..20)?.try_into().ok()
}

/// The most bytes of an ICC profile a TIFF's tag 34675 is read with (16 MiB).
const MAX_TIFF_ICC: u64 = 16 << 20;

/// The ICC profile of TIFF tag 34675 (ICCProfile, of type UNDEFINED or BYTE) of the image `d`
/// reads, as its bytes; `None` without one and for one over [`MAX_TIFF_ICC`] bytes.
fn tiff_icc(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>) -> Option<Vec<u8>> {
    use tiff::tags::{Tag, Type, ValueBuffer};
    let mut ifd = d.image_ifd();
    let entry = ifd.find_entry(Tag::IccProfile)?;
    if !matches!(entry.field_type(), Type::UNDEFINED | Type::BYTE) || entry.count() == 0 || entry.count() > MAX_TIFF_ICC {
        return None;
    }
    let mut buf = ValueBuffer::empty(Type::UNDEFINED);
    ifd.find_tag_buf(Tag::IccProfile, &mut buf).ok()??;
    Some(buf.as_bytes().to_vec())
}

/// The ICC profile of TIFF `bytes`' first image (tag 34675), up to [`MAX_TIFF_ICC`] bytes whatever
/// the size of the image; `None` for other images, TIFFs without one and a longer one.
pub fn tiff_icc_profile(bytes: &[u8]) -> Option<Vec<u8>> {
    let (_, mut d) = tiff(bytes)?;
    tiff_icc(&mut d)
}

/// The ICC profile of a JPEG, put together from all its numbered `ICC_PROFILE` APP2 segments;
/// `None` without one.
fn jpeg_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    craft_image_codecs::jpeg_inks::jpeg_icc_profile(bytes)
}

#[cfg(test)]
const ICC_SEGMENT: &[u8; 12] = b"ICC_PROFILE\0";

/// JPEG `bytes` carrying `icc` in place of the ICC profile they carry, if any: every
/// `ICC_PROFILE` APP2 segment before the scan taken out, `icc` written as numbered APP2 segments
/// after the leading application segments, and the other segments and the scan as they are. `None`
/// for bytes whose segments before the scan can't be read, an empty `icc`, and one longer than 255
/// segments hold.
pub fn jpeg_with_icc(bytes: &[u8], icc: &[u8]) -> Option<Vec<u8>> {
    craft_image_codecs::jpeg_inks::jpeg_with_icc(bytes, icc)
}

/// JPEG `bytes` without the ICC profile they carry: every `ICC_PROFILE` APP2 segment before the
/// scan taken out, and the other segments and the scan as they are. `None` for bytes whose
/// segments before the scan can't be read.
pub fn jpeg_without_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    craft_image_codecs::jpeg_inks::jpeg_without_icc(bytes)
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
/// the TIFF's CMYK ICC profile; `None` for other images, CMYK ones with alpha, ones larger than
/// [`MAX_CMYK_PIXELS`], and, before any decoding, ones with more than [`MAX_EXTRA_SAMPLES_PIXEL`]
/// samples a pixel (their SamplesPerPixel), ones whose samples aren't unsigned integers (a
/// SampleFormat other than 1) and ones whose samples with the row the decoder reads each strip or
/// tile into ([`row_buffer`]) take more than [`crate::MAX_RASTER_ALLOC`].
fn tiff_source(bytes: &[u8]) -> Option<CmykSource> {
    let (kind, mut d) = tiff(bytes)?;
    if !cmyk_inks(kind, &mut d) {
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

    /// [`ImageBlob::cmyk_tiff`], carrying `icc` as tag 34675 (ICCProfile) when it is a profile of
    /// CMYK colors; any other profile is left out. The caller checks the rest of the profile.
    pub fn cmyk_tiff_with_icc(inks: &Inks, icc: Option<&[u8]>) -> Option<Self> {
        let icc = icc.filter(|icc| icc_color_space(icc) == Some(*b"CMYK"));
        let tiff = write_cmyk_tiff(inks.width, inks.height, SampleSlice::Eight(&inks.data), None, icc)?;
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
            return craft_image_codecs::jpeg_inks::jpeg_dimensions(bytes);
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
    let limits = craft_image_codecs::Limits { max_pixels: MAX_CMYK_PIXELS, ..craft_image_codecs::Limits::default() };
    let image = craft_image_codecs::jpeg_inks::decode_jpeg_inks(bytes, &limits).ok()?;
    let (w, h) = image.dimensions();
    Inks::new(w, h, image.into_data())
}

/// The ink amounts of a CMYK TIFF (16-bit samples to 8 bits), its inks stored together or one plane
/// per ink. `None` for other images, ones larger than [`MAX_CMYK_PIXELS`], and the files
/// [`read_samples`] refuses: more than [`MAX_EXTRA_SAMPLES_PIXEL`] samples a pixel, samples that
/// aren't unsigned integers, or samples that with the decoder's row take more than
/// [`crate::MAX_RASTER_ALLOC`].
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
/// the file has, carrying the file's CMYK ICC profile. `None` for other images, for CMYK TIFFs
/// with an extra sample named as alpha and for ones whose inks [`ImageBlob::cmyk`] doesn't read.
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
/// images, for CMYK TIFFs with an extra sample named as alpha and for CMYK images whose inks
/// [`ImageBlob::cmyk`] doesn't read.
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
/// all, as RGBA pixels: CMYK with an alpha channel, each color converted by `rgb` (ink amounts
/// 0..1 → sRGB 0..1; the inks can't be kept with the transparency, so they show as RGB), and
/// grayscale with extra samples. The alpha is the first extra sample whose ExtraSamples value
/// names it (1 or 2), whatever extra samples come before it; the other extra samples are left out,
/// and without such a value the pixels are opaque. A CMYK file with a fifth sample and no
/// ExtraSamples tag has that sample as unassociated alpha. Associated (premultiplied) alpha is
/// divided back out, and a pixel with no associated alpha has no color. 16-bit samples are
/// converted at 16 bits, and only the result is rounded to 8 bits. The samples may be stored
/// together or one plane per channel. A CMYK file with two or more samples past its inks and its
/// ExtraSamples values (or with no ExtraSamples tag) is read too, opaque when none of those values
/// names an alpha. `None` for other images, CMYK ones without alpha (their inks are read by
/// [`ImageBlob::cmyk`]), ones larger than [`MAX_CMYK_PIXELS`], ones whose SamplesPerPixel is more
/// than [`MAX_EXTRA_SAMPLES_PIXEL`], ones whose samples aren't unsigned integers (a SampleFormat
/// other than 1) and ones whose samples, with the copy a planar file needs, the row the decoder
/// reads each strip or tile into and the RGBA pixels, take more than [`crate::MAX_RASTER_ALLOC`];
/// those are refused from the file's tags, before any decoding.
pub fn extra_samples_tiff_rgba(bytes: &[u8], rgb: impl Fn([f32; 4]) -> [f32; 3]) -> Option<image::RgbaImage> {
    use tiff::tags::Tag;
    let mut d = tiff_decoder(bytes)?;
    // The decoder gives no color type to a CMYK file that has six or more samples besides the
    // ones its ExtraSamples values count.
    let kind = d.colortype().ok();
    let (w, h) = d.dimensions().ok()?;
    // 0: white is zero, 1: black is zero (the two grayscales), 5: CMYK.
    let photometric = d.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten();
    let extras = extra_samples(&mut d);
    // The first extra sample named as alpha: (its place among the extra samples, is it associated?)
    let named = extras.iter().position(|e| matches!(e, 1 | 2)).map(|i| (i, extras.get(i) == Some(&1)));
    // The samples of a pixel that no ExtraSamples value counts.
    let uncounted = d.find_tag_unsigned::<u16>(Tag::SamplesPerPixel).ok().flatten().and_then(|all| usize::from(all).checked_sub(extras.len()));
    // (how many of a pixel's samples are its color, the alpha among the extra samples)
    let (colour, alpha) = match kind {
        Some(tiff::ColorType::CMYKA(8 | 16)) => (4, if extras.is_empty() { Some((0, false)) } else { named }),
        Some(tiff::ColorType::CMYK(8 | 16)) if named.is_some() => (4, named),
        Some(tiff::ColorType::Multiband { bit_depth: 8 | 16, num_samples: 2.. }) if matches!(photometric, Some(0 | 1)) => (1, named),
        None if photometric == Some(5) && uncounted.is_some_and(|c| c > 5) => (4, named),
        _ => return None,
    };
    let n = usize::try_from(pixels(w, h)?).ok()?;
    // The decoder gives a CMYK pixel its inks and at most one extra sample, and doesn't read
    // white-is-zero gray with extra samples: those are read as black-is-zero gray, every sample.
    let given = kind.map_or(0, |k| usize::from(k.num_samples()));
    let (data, samples) = if photometric == Some(0) || kind.is_none() || alpha.is_some_and(|(i, _)| colour + i >= given) {
        let mut d = tiff::decoder::Decoder::new(as_black_is_zero(bytes)?).ok()?;
        let tiff::ColorType::Multiband { bit_depth: bits @ (8 | 16), num_samples } = d.colortype().ok()? else { return None };
        let samples = usize::from(num_samples);
        (decode_samples(&mut with_room(d, bytes.len(), n, samples, bits)?, n, samples)?, samples)
    } else {
        (decode_samples(&mut with_room(d, bytes.len(), n, given, kind?.bit_depth())?, n, given)?, given)
    };
    // (the alpha's sample in a pixel, is it associated?), when the pixel has that sample.
    let alpha = alpha.map(|(i, associated)| (colour + i, associated)).filter(|(at, _)| *at < samples);
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
        let (opacity, a) = match alpha {
            Some((at, _)) => (byte_at(first + at).unwrap_or(u8::MAX), level_at(first + at).unwrap_or(1.0)),
            None => (u8::MAX, 1.0),
        };
        let level = |s: usize| {
            let v = level_at(first + s).unwrap_or(0.0);
            match alpha {
                Some((_, true)) if a > 0.0 => (v / a).min(1.0),
                // No associated alpha: the color is zero (TIFF 6.0, Associated Alpha Handling).
                Some((_, true)) => 0.0,
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
    /// larger than [`crate::MAX_RASTER_ALLOC`] allows as RGBA, or is wider than
    /// [`craft_stream_codecs::MAX_CCITT_COLUMNS`].
    #[error("could not decode image: {}", damage(*.0))]
    Damaged(craft_stream_codecs::CcittErrorKind),
}

/// What went wrong in a Group 3 strip or tile, as [`Group3Error::Damaged`] says it.
fn damage(kind: craft_stream_codecs::CcittErrorKind) -> &'static str {
    use craft_stream_codecs::CcittErrorKind as K;
    match kind {
        K::UnexpectedEof => "unexpected end of input",
        K::Overflow => "arithmetic overflow in position calculation",
        K::InvalidData => "invalid CCITT code sequence",
    }
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
    use craft_stream_codecs::{CcittErrorKind as DecodeError, CcittParams, decode_ccitt_raw};
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
    // Each row's tag bit says how it is coded (K above 0), or every row is one-dimensional (K 0).
    let k = if compression == 3 && t4_options & 1 != 0 { 2 } else { 0 };
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
        // A tile is coded whole; the last strip only has the image's remaining rows.
        let rows = if tiled { ch } else { ch.min(h - y0) };
        let params = CcittParams {
            k,
            columns: i64::from(cw),
            rows: i64::from(rows),
            end_of_line: compression == 3,
            encoded_byte_align: compression == 2,
            end_of_block: compression == 3,
            // 1 bits are black runs, 0 bits white ones.
            black_is_1: true,
        };
        let Ok(out) = decode_ccitt_raw(&data, &params, rows) else {
            // Wider than the decoder reads.
            return damaged(DecodeError::Overflow);
        };
        if let Some(e) = out.error {
            return damaged(e);
        }
        // The data ended, or six EOL codes came, before the strip's or tile's last row.
        if out.rows_decoded < rows {
            return damaged(DecodeError::UnexpectedEof);
        }
        put_rows(&mut gray, (w, h), (x0, y0), cw, &out.data, zero);
    }
    let gray = image::GrayImage::from_raw(w, h, gray)?;
    Some(Ok(image::DynamicImage::ImageLuma8(gray).to_rgba8()))
}

/// The decoded rows `packed` of a strip or tile of a Group 3 TIFF ([`group3_tiff_rgba`]), `columns`
/// wide, one bit a pixel (1 a black run's), put into `out`, the image's levels row by row, `size`
/// (width, height), from (`x0`, `y0`): a 0 bit as `zero` (the level of a white run), a 1 bit as the
/// other level. Pixels past the image's right or bottom edge (a tile's padding) are dropped.
fn put_rows(out: &mut [u8], size: (u32, u32), (x0, y0): (u32, u32), columns: u32, packed: &[u8], zero: u8) {
    let (width, height) = (size.0 as usize, size.1 as usize);
    let (x0, y0, columns) = (x0 as usize, y0 as usize, columns as usize);
    let stride = columns.div_ceil(8);
    if stride == 0 {
        return;
    }
    let across = columns.min(width.saturating_sub(x0));
    for (y, row) in packed.chunks(stride).enumerate() {
        let Some(at) = (y0 + y < height).then(|| (y0 + y) * width + x0) else { break };
        let Some(px) = out.get_mut(at..at + across) else { break };
        for (x, p) in px.iter_mut().enumerate() {
            let black = row.get(x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) != 0);
            *p = if black { u8::MAX - zero } else { zero };
        }
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

    /// A pixel type of `$n` samples of `$inner`, `$bits` bits each, in color space `$space`.
    macro_rules! samples {
        ($name:ident, $inner:ty, $space:ident, $bits:literal, $n:literal) => {
            struct $name;
            impl tiff::encoder::colortype::ColorType for $name {
                type Inner = $inner;
                const TIFF_VALUE: tiff::tags::PhotometricInterpretation = tiff::tags::PhotometricInterpretation::$space;
                const BITS_PER_SAMPLE: &'static [u16] = &[$bits; $n];
                const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] = &[tiff::tags::SampleFormat::Uint; $n];
                fn horizontal_predict(row: &[$inner], result: &mut Vec<$inner>) {
                    result.extend_from_slice(row);
                }
            }
        };
    }
    samples!(Cmyk6x8, u8, CMYK, 8, 6);
    samples!(Cmyk6x16, u16, CMYK, 16, 6);
    samples!(Cmyk7x8, u8, CMYK, 8, 7);
    samples!(Cmyk8x8, u8, CMYK, 8, 8);
    samples!(Gray3x8, u8, BlackIsZero, 8, 3);
    samples!(WhiteGray2x8, u8, WhiteIsZero, 8, 2);

    /// A one-row TIFF of `C`'s pixels `data`, with the ExtraSamples values `extra`.
    fn extras_tiff<C: tiff::encoder::colortype::ColorType>(extra: &[u16], data: &[C::Inner]) -> Vec<u8>
    where
        [C::Inner]: tiff::encoder::TiffValue,
    {
        let samples = C::BITS_PER_SAMPLE.len();
        let mut file = Cursor::new(Vec::new());
        let mut enc = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        let mut img = enc.new_image::<C>(u32::try_from(data.len() / samples).unwrap(), 1).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, extra).unwrap();
        img.write_data(data).unwrap();
        file.into_inner()
    }

    /// Two pixels of four inks with an alpha sample and another extra sample, the alpha first when
    /// `alpha_first`: cyan at an alpha of 128 (or premultiplied, at 128 of cyan when `associated`)
    /// with an extra sample of 9, then black, opaque, with an extra sample of 64.
    fn inks_alpha_and_extra(alpha_first: bool, associated: bool) -> Vec<u16> {
        let cyan = if associated { 128 } else { 255 };
        let px = |inks: [u16; 4], alpha: u16, other: u16| {
            let extras = if alpha_first { [alpha, other] } else { [other, alpha] };
            [&inks[..], &extras[..]].concat()
        };
        [px([cyan, 0, 0, 0], 128, 9), px([0, 0, 0, 255], 255, 64)].concat()
    }

    /// A CMYK TIFF whose pixels have alpha and another extra sample reads the sample its
    /// ExtraSamples names as alpha, first or second, at 8 and 16 bits, and leaves the other out.
    /// Open and Place don't keep its inks. A blob that holds the file, as a document saved with it
    /// as a CMYK image does, is a CMYK image with those inks when the alpha is the second extra
    /// sample.
    #[test]
    fn a_cmyk_tiff_with_alpha_and_another_extra_sample_reads_its_alpha() {
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        for extra in [[1, 0], [2, 0], [0, 1], [0, 2]] {
            let data = inks_alpha_and_extra(extra[0] != 0, extra.contains(&1));
            let eight: Vec<u8> = data.iter().map(|&v| u8::try_from(v).unwrap()).collect();
            let sixteen: Vec<u16> = data.iter().map(|&v| v * 257).collect();
            for (bits, bytes) in [(8, extras_tiff::<Cmyk6x8>(&extra, &eight)), (16, extras_tiff::<Cmyk6x16>(&extra, &sixteen))] {
                let px = extra_samples_tiff_rgba(&bytes, naive).unwrap_or_else(|| panic!("{extra:?}, {bits} bits: reads"));
                assert_eq!(px.into_raw(), [0, 255, 255, 128, 0, 0, 0, 255], "{extra:?}, {bits} bits: cyan at half alpha, then black");
                assert!(stored_cmyk_tiff(&bytes, None).is_none(), "{extra:?}, {bits} bits: the inks can't be kept with the alpha");
                let b = ImageBlob::new("image/tiff", bytes);
                if extra[0] != 0 {
                    // The decoder gives these the color type of CMYK with alpha.
                    assert!(!b.is_cmyk() && b.cmyk().is_none(), "{extra:?}, {bits} bits: never stored as inks");
                    continue;
                }
                assert!(b.is_cmyk(), "{extra:?}, {bits} bits: a stored one stays a CMYK image");
                assert_eq!(b.is_cmyk16(), bits == 16, "{extra:?}, {bits} bits");
                let cyan = u8::try_from(data[0]).unwrap();
                assert_eq!(b.cmyk().map(|i| i.data), Some(vec![cyan, 0, 0, 0, 0, 0, 0, 255]), "{extra:?}, {bits} bits: a stored one keeps its inks");
            }
        }
        // Two extra samples, neither of them alpha: the inks are kept, as with one.
        let bytes = extras_tiff::<Cmyk6x8>(&[0, 0], &[255, 0, 0, 0, 9, 128, 0, 0, 0, 255, 64, 255]);
        assert!(extra_samples_tiff_rgba(&bytes, naive).is_none(), "not alpha: not this reader's");
        assert!(stored_cmyk_tiff(&bytes, None).is_some());
        let b = ImageBlob::new("image/tiff", bytes);
        assert!(b.is_cmyk() && !b.is_cmyk16());
        assert_eq!(b.cmyk().map(|i| i.data), Some(vec![255, 0, 0, 0, 0, 0, 0, 255]));
    }

    /// A CMYK TIFF of six samples a pixel whose ExtraSamples has only one value: the sample after
    /// the inks is alpha only when that value says so, and the sample after it is never alpha. Files
    /// with more samples than the decoder gives a color type read the same way, opaque when no
    /// value names an alpha.
    #[test]
    fn a_cmyk_tiff_with_fewer_extra_sample_values_than_samples_reads_only_a_named_alpha() {
        let naive = |[c, m, y, k]: [f32; 4]| [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
        let data = [255, 0, 0, 0, 128, 9, 0, 0, 0, 255, 255, 64];
        let read = |extra: u16| extra_samples_tiff_rgba(&extras_tiff::<Cmyk6x8>(&[extra], &data), naive).expect("reads").into_raw();
        assert_eq!(read(2), [0, 255, 255, 128, 0, 0, 0, 255], "the sample after the inks is the alpha");
        assert_eq!(read(0), [0, 255, 255, 255, 0, 0, 0, 255], "not alpha: opaque");
        // Eight samples with two values: the decoder gives the file no color type. The second
        // value names the sixth sample as the alpha.
        let wide = [255, 0, 0, 0, 9, 128, 17, 18];
        let px = extra_samples_tiff_rgba(&extras_tiff::<Cmyk8x8>(&[0, 2], &wide), naive).expect("reads");
        assert_eq!(px.into_raw(), [0, 255, 255, 128], "cyan at the alpha the second value names");
        let px = extra_samples_tiff_rgba(&extras_tiff::<Cmyk8x8>(&[0, 0], &wide), naive).expect("no value names an alpha: reads");
        assert_eq!(px.into_raw(), [0, 255, 255, 255], "no value names an alpha: opaque cyan");
        // Six samples and no ExtraSamples tag: the decoder gives the file no color type either.
        let mut file = Cursor::new(Vec::new());
        let mut enc = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        enc.write_image::<Cmyk6x8>(1, 1, &wide[..6]).unwrap();
        let px = extra_samples_tiff_rgba(&file.into_inner(), naive).expect("no ExtraSamples tag: reads");
        assert_eq!(px.into_raw(), [0, 255, 255, 255], "no ExtraSamples tag: opaque cyan");
        // Seven samples with two values: the decoder reads it as CMYK with alpha.
        let px = extra_samples_tiff_rgba(&extras_tiff::<Cmyk7x8>(&[0, 2], &wide[..7]), naive).expect("reads");
        assert_eq!(px.into_raw(), [0, 255, 255, 128], "seven samples: cyan at the alpha the second value names");
        // Its Orientation tag is read too.
        let mut file = Cursor::new(Vec::new());
        let mut enc = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
        let mut img = enc.new_image::<Cmyk8x8>(1, 1).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, &[0u16, 2][..]).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::Orientation, 6u16).unwrap();
        img.write_data(&wide).unwrap();
        let turned = file.into_inner();
        assert_eq!(tiff_orientation(&turned), Some(6));
        assert_eq!(exif_orientation(&turned), Some(image::metadata::Orientation::Rotate90));
    }

    /// A white-is-zero grayscale TIFF `width` pixels wide and `rows` tall (2 or more), each pixel
    /// `samples` samples of 8 bits: a level of 0, an unassociated alpha of 128, then zeros. Each
    /// row is a strip, and every strip is the same bytes of the file.
    fn tall_gray_tiff(width: u32, rows: u32, samples: u16) -> Vec<u8> {
        gray_tiff(width, rows, samples, false)
    }

    /// [`tall_gray_tiff`], or with `planar` one plane per sample, each row of each plane a strip
    /// of zeros, every strip the same bytes of the file.
    fn gray_tiff(width: u32, rows: u32, samples: u16, planar: bool) -> Vec<u8> {
        let at = |f: &Vec<u8>| u32::try_from(f.len()).unwrap();
        let mut f = b"II*\0\0\0\0\0".to_vec();
        let row = at(&f);
        if planar {
            f.resize(f.len() + usize::try_from(width).unwrap(), 0);
        } else {
            for _ in 0..width {
                f.extend([0, 128]);
                f.resize(f.len() + usize::from(samples) - 2, 0);
            }
        }
        let (strips, strip) = if planar { (rows * u32::from(samples), width) } else { (rows, width * u32::from(samples)) };
        let offsets = at(&f);
        f.extend((0..strips).flat_map(|_| row.to_le_bytes()));
        let counts = at(&f);
        f.extend((0..strips).flat_map(|_| strip.to_le_bytes()));
        let ifd = at(&f);
        f[4..8].copy_from_slice(&ifd.to_le_bytes());
        // (tag, type (3 SHORT, 4 LONG), count, value or offset)
        let entries = [
            (256u16, 4u16, 1, width),
            (257, 4, 1, rows),
            (258, 3, 1, 8),
            (259, 3, 1, 1),
            (262, 3, 1, 0),
            (273, 4, strips, offsets),
            (277, 3, 1, u32::from(samples)),
            (278, 4, 1, 1),
            (279, 4, strips, counts),
            (284, 3, 1, if planar { 2 } else { 1 }),
            (338, 3, 1, 2),
        ];
        f.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
        for (tag, ty, count, value) in entries {
            f.extend([tag.to_le_bytes(), ty.to_le_bytes()].concat());
            f.extend([count.to_le_bytes(), value.to_le_bytes()].concat());
        }
        f.extend([0; 4]);
        f
    }

    /// A TIFF with extra samples whose samples take more than the decoder's own limit of 256 MiB
    /// reads: here 256 × 16,385 pixels of 64 samples, 16,384 bytes more than the limit.
    #[test]
    fn a_tiff_with_extra_samples_past_the_decoders_limit_reads() {
        let (width, rows, samples) = (256, 16_385, 64);
        assert!(u64::from(width * rows) * u64::from(samples) > 256 << 20);
        let px = extra_samples_tiff_rgba(&tall_gray_tiff(width, rows, samples), |_| [0.0; 3]).expect("reads");
        assert_eq!((px.width(), px.height()), (width, rows));
        assert!(px.pixels().all(|p| p.0 == [255, 255, 255, 128]), "white at an alpha of 128");
    }

    /// A pixel of more than [`MAX_EXTRA_SAMPLES_PIXEL`] samples is not read.
    #[test]
    fn a_tiff_with_too_many_samples_a_pixel_is_not_read() {
        let samples = u16::try_from(MAX_EXTRA_SAMPLES_PIXEL).unwrap();
        assert!(extra_samples_tiff_rgba(&tall_gray_tiff(2, 2, samples), |_| [0.0; 3]).is_some(), "at the limit: reads");
        assert!(extra_samples_tiff_rgba(&tall_gray_tiff(2, 2, samples + 1), |_| [0.0; 3]).is_none(), "past it: not read");
    }

    /// The header of a one-row CMYK TIFF `width` pixels wide, each pixel `samples` samples of
    /// `bits` bits stored together, with the ExtraSamples values `extras` (more than two). Its one
    /// strip claims every sample of the row, uncompressed, and holds four bytes of zeros.
    fn cmyk_header(width: u32, samples: u16, bits: u16, extras: &[u16]) -> Vec<u8> {
        assert!(extras.len() > 2, "the values sit outside the entry");
        let mut f = b"II*\0\0\0\0\0".to_vec();
        f.extend([0; 4]);
        let at = u32::try_from(f.len()).unwrap();
        f.extend(extras.iter().flat_map(|e| e.to_le_bytes()));
        let ifd = u32::try_from(f.len()).unwrap();
        f[4..8].copy_from_slice(&ifd.to_le_bytes());
        let strip = u32::try_from(u64::from(width) * u64::from(samples) * u64::from(bits / 8)).unwrap();
        // (tag, type (3 SHORT, 4 LONG), count, value or offset)
        let entries = [
            (256u16, 4u16, 1, width),
            (257, 4, 1, 1),
            (258, 3, 1, u32::from(bits)),
            (259, 3, 1, 1),
            (262, 3, 1, 5),
            (273, 4, 1, 8),
            (277, 3, 1, u32::from(samples)),
            (278, 4, 1, 1),
            (279, 4, 1, strip),
            (284, 3, 1, 1),
            (338, 3, u32::try_from(extras.len()).unwrap(), at),
        ];
        f.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
        for (tag, ty, count, value) in entries {
            f.extend([tag.to_le_bytes(), ty.to_le_bytes()].concat());
            f.extend([count.to_le_bytes(), value.to_le_bytes()].concat());
        }
        f.extend([0; 4]);
        f
    }

    /// A CMYK TIFF whose first ExtraSamples value is alpha has the color type of CMYK with alpha,
    /// five samples, however many samples its pixels have; one with more than
    /// [`MAX_EXTRA_SAMPLES_PIXEL`] is refused from its tags, before the decoder takes a row of
    /// every sample (here 8,192 × 65,535 × 2 bytes, 1 GiB). Neither is it read as inks.
    #[test]
    fn a_cmyk_tiff_with_more_samples_a_pixel_than_the_limit_is_refused_before_decoding() {
        let (width, samples) = (8192u32, u16::MAX);
        for first in [1u16, 0] {
            let extras: Vec<u16> = std::iter::once(first).chain(std::iter::repeat_n(0, usize::from(samples) - 5)).collect();
            let bytes = cmyk_header(width, samples, 16, &extras);
            let mut d = tiff::decoder::Decoder::new(Cursor::new(&bytes[..])).unwrap();
            let kind = d.colortype().unwrap();
            assert_eq!(kind, if first == 1 { tiff::ColorType::CMYKA(16) } else { tiff::ColorType::CMYK(16) });
            assert_eq!(row_buffer(&mut d), None, "{kind:?}: refused from the tags");
            let given = usize::from(kind.num_samples());
            assert!(with_room(d, bytes.len(), usize::try_from(width).unwrap(), given, 16).is_none(), "{kind:?}: refused before the decode");
            assert!(extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).is_none(), "{kind:?}");
            assert!(ImageBlob::new("image/tiff", bytes).cmyk().is_none(), "{kind:?}: no inks");
        }
    }

    /// The row the decoder reads each strip of a CMYK TIFF with alpha into, every sample of the
    /// row, counts in the allowance: 2²⁴ pixels of 64 16-bit samples, whose five decoded samples
    /// and RGBA pixels fit but not with that row of 2 GiB.
    #[test]
    fn the_decoders_row_of_every_sample_counts_in_the_allowance() {
        let width = 1u32 << 24;
        let samples = u16::try_from(MAX_EXTRA_SAMPLES_PIXEL).unwrap();
        let extras: Vec<u16> = std::iter::once(2).chain(std::iter::repeat_n(0, usize::from(samples) - 5)).collect();
        let bytes = cmyk_header(width, samples, 16, &extras);
        let n = usize::try_from(width).unwrap();
        let (need, rgba, row) = (u64::from(width) * 5 * 2, u64::from(width) * 4, u64::from(width) * u64::from(samples) * 2);
        assert!(need + rgba <= crate::MAX_RASTER_ALLOC && need + rgba + row > crate::MAX_RASTER_ALLOC);
        let mut d = tiff::decoder::Decoder::new(Cursor::new(&bytes[..])).unwrap();
        assert_eq!(d.colortype().unwrap(), tiff::ColorType::CMYKA(16));
        assert_eq!(row_buffer(&mut d), Some(row));
        assert!(with_room(d, bytes.len(), n, 5, 16).is_none(), "refused before the decode");
        assert!(extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).is_none());
    }

    /// A TIFF whose samples aren't unsigned integers is refused from its tags.
    #[test]
    fn a_tiff_of_float_samples_is_refused_from_its_tags() {
        use tiff::encoder::colortype::{Gray8, Gray32Float};
        let mut file = Cursor::new(Vec::new());
        tiff::encoder::TiffEncoder::new(&mut file).unwrap().write_image::<Gray32Float>(2, 1, &[0.25, 0.5]).unwrap();
        let float = file.into_inner();
        let mut d = tiff::decoder::Decoder::new(Cursor::new(&float[..])).unwrap();
        assert_eq!(row_buffer(&mut d), None, "floats: refused");
        assert!(read_samples_at_depth(&mut d, 2, 1).is_none());
        let mut file = Cursor::new(Vec::new());
        tiff::encoder::TiffEncoder::new(&mut file).unwrap().write_image::<Gray8>(2, 1, &[64, 128]).unwrap();
        let bytes = file.into_inner();
        let mut d = tiff::decoder::Decoder::new(Cursor::new(&bytes[..])).unwrap();
        assert_eq!(row_buffer(&mut d), Some(0), "unsigned: no extra row");
        assert_eq!(read_samples_at_depth(&mut d, 2, 1), Some(Samples::Eight(vec![64, 128])));
    }

    /// The copy that puts a planar file's samples pixel by pixel is allocated at its length, so
    /// the reader holds no more than the allowance counts. 33 samples: a copy that grew as it
    /// was filled would hold room for 64.
    #[test]
    fn the_planar_copy_is_allocated_at_its_length() {
        let (n, per) = (11u8, 3u8);
        let data: Vec<u8> = (0..n * per).collect();
        let len = data.len();
        let out = chunky(data, usize::from(per), usize::from(n), usize::from(per)).unwrap();
        let pixel_by_pixel: Vec<u8> = (0..n).flat_map(|i| (0..per).map(move |p| p * n + i)).collect();
        assert_eq!(out, pixel_by_pixel);
        assert!(out.capacity() >= len && out.capacity() < len.next_power_of_two(), "capacity {} for {len} samples", out.capacity());
    }

    /// A TIFF whose decoded samples fit [`crate::MAX_RASTER_ALLOC`] but not with its RGBA pixels
    /// beside them is refused before its samples are decoded; so is one whose samples fit with
    /// the RGBA pixels but not with the copy a planar file needs. The file holds one row of pixels,
    /// which every strip reuses.
    #[test]
    fn a_tiff_whose_decode_with_its_rgba_takes_more_than_the_allowance_is_refused() {
        let side = 8192u32;
        let n = usize::try_from(u64::from(side) * u64::from(side)).unwrap();
        assert_eq!(u64::try_from(n).unwrap(), MAX_CMYK_PIXELS);
        let max = usize::try_from(crate::MAX_RASTER_ALLOC).unwrap();
        // One sample fewer than fills the allowance: the samples alone fit.
        let per = max / n - 1;
        assert!(n * per <= max && n * per + 4 * n > max);
        let bytes = tall_gray_tiff(side, side, u16::try_from(per).unwrap());
        let decoder = || tiff::decoder::Decoder::new(Cursor::new(&bytes[..])).unwrap();
        assert!(with_room(decoder(), bytes.len(), n, per, 8).is_none(), "refused before the decode");
        assert!(extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).is_none());
        // Half the allowance in samples fits with the RGBA pixels, but not twice over.
        let half = max / n / 2;
        assert!(with_room(decoder(), bytes.len(), n, half, 8).is_some(), "stored together: fits");
        let planar = gray_tiff(side, side, u16::try_from(half).unwrap(), true);
        let decoder = tiff::decoder::Decoder::new(Cursor::new(&planar[..])).unwrap();
        assert!(with_room(decoder, planar.len(), n, half, 8).is_none(), "one plane per sample: refused");
    }

    /// Patched bytes read the same whole, a byte at a time and after a seek into a patch.
    #[test]
    fn patched_bytes_read_with_their_patches() {
        use std::io::{Read, Seek, SeekFrom};
        let bytes: Vec<u8> = (10..30).collect();
        let patched = || Patched { bytes: Cursor::new(&bytes[..]), patches: vec![(4, &[1, 0]), (19, &[7, 7])] };
        let want = [10, 11, 12, 13, 1, 0, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 7];
        let mut whole = Vec::new();
        patched().read_to_end(&mut whole).unwrap();
        assert_eq!(whole, want);
        let mut one = patched();
        let bytewise: Vec<u8> = (0..want.len())
            .map(|_| {
                let mut b = [0];
                assert_eq!(one.read(&mut b).unwrap(), 1);
                b[0]
            })
            .collect();
        assert_eq!(bytewise, want);
        let mut from = patched();
        from.seek(SeekFrom::Start(5)).unwrap();
        let mut b = [9; 3];
        from.read_exact(&mut b).unwrap();
        assert_eq!(b, [0, 16, 17], "the second byte of the first patch, then the file");
    }

    /// A grayscale TIFF whose alpha follows another extra sample reads that alpha.
    #[test]
    fn a_grayscale_tiff_with_alpha_after_another_extra_sample_reads_its_alpha() {
        let read =
            |extra: [u16; 2], data: &[u8]| extra_samples_tiff_rgba(&extras_tiff::<Gray3x8>(&extra, data), |_| [0.0; 3]).expect("reads").into_raw();
        // A gray of 64 at an alpha of 204 (premultiplied: 51), with an extra sample of 9.
        assert_eq!(read([0, 2], &[64, 9, 204]), [64, 64, 64, 204], "unassociated");
        assert_eq!(read([0, 1], &[51, 9, 204]), [64, 64, 64, 204], "associated, divided back out");
        assert_eq!(read([2, 0], &[64, 204, 9]), [64, 64, 64, 204], "the alpha first");
        assert_eq!(read([0, 0], &[64, 9, 204]), [64, 64, 64, 255], "no alpha: opaque");
    }

    /// A grayscale TIFF whose white is zero (PhotometricInterpretation 0), with alpha, reads with
    /// its levels turned around.
    #[test]
    fn a_white_is_zero_grayscale_tiff_with_alpha_reads_as_rgba() {
        // White, opaque; black at an alpha of 128.
        let bytes = extras_tiff::<WhiteGray2x8>(&[2], &[0, 255, 255, 128]);
        let px = extra_samples_tiff_rgba(&bytes, |_| [0.0; 3]).expect("reads");
        assert_eq!(px.into_raw(), [255, 255, 255, 255, 0, 0, 0, 128]);
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

    /// A CMYK TIFF written with a CMYK profile carries it byte for byte as tag 34675; a profile of
    /// other colors, and none, leave the TIFF untagged. The inks are the same either way.
    #[test]
    fn a_cmyk_tiff_carries_the_cmyk_profile_it_is_given() {
        let profile = |space: &[u8; 4]| {
            let mut icc = vec![0u8; 128];
            icc[0..4].copy_from_slice(&128u32.to_be_bytes());
            icc[16..20].copy_from_slice(space);
            icc[36..40].copy_from_slice(b"acsp");
            icc
        };
        let cmyk = profile(b"CMYK");
        let b = ImageBlob::cmyk_tiff_with_icc(&inks(), Some(&cmyk)).unwrap();
        assert_eq!(b.cmyk_icc_profile(), Some(cmyk), "the profile, byte for byte");
        assert_eq!(b.cmyk(), Some(inks()));
        // Read back up to 16 MiB; a longer one isn't read.
        for (len, read) in [(16 << 20, true), ((16 << 20) + 1, false)] {
            let mut big = profile(b"CMYK");
            big.resize(len, 0);
            let b = ImageBlob::cmyk_tiff_with_icc(&inks(), Some(&big)).unwrap();
            assert_eq!(b.cmyk_icc_profile().is_some_and(|p| p == big), read, "{len} bytes");
        }
        for icc in [Some(profile(b"RGB ")), None] {
            let b = ImageBlob::cmyk_tiff_with_icc(&inks(), icc.as_deref()).unwrap();
            assert_eq!(tiff_icc_profile(&b.bytes), None, "{icc:?}");
            assert_eq!(b.cmyk(), Some(inks()));
        }
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

    /// A CMYK JPEG given a profile carries it in place of its own, its samples as they were; a
    /// profile longer than 255 segments hold, an empty one and bytes that aren't a JPEG give none.
    #[test]
    fn a_cmyk_jpeg_carries_the_profile_it_is_given() {
        let profile = |len: usize, mark: u8| {
            let mut icc = vec![mark; len];
            icc[16..20].copy_from_slice(b"CMYK");
            icc
        };
        let px: Vec<u8> = (0..16 * 16).flat_map(|_| [10u8, 40, 100, 20]).collect();
        let mut jpg = Vec::new();
        let mut e = jpeg_encoder::Encoder::new(&mut jpg, 100);
        e.add_icc_profile(&profile(300_000, 1)).unwrap();
        e.encode(&px, 16, 16, jpeg_encoder::ColorType::Cmyk).unwrap();
        let inks = ImageBlob::new("image/jpeg", jpg.clone()).cmyk().unwrap();
        // One segment holds 65,519 bytes of a profile.
        for len in [128, 65_519, 65_520, 300_000, 1 << 20] {
            let icc = profile(len, 2);
            let b = ImageBlob::new("image/jpeg", jpeg_with_icc(&jpg, &icc).unwrap());
            assert!(b.cmyk_icc_profile() == Some(icc), "{len} bytes");
            assert!(b.cmyk() == Some(inks.clone()), "{len} bytes: the samples as they were");
        }
        // A profile segment after the quantization table is taken out too.
        let dqt = jpg.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        let after = dqt + 2 + usize::from(u16::from_be_bytes([jpg[dqt + 2], jpg[dqt + 3]]));
        let mut late = jpg.clone();
        let stale = profile(128, 3);
        let segment = [&[0xFF, 0xE2, 0, 144][..], ICC_SEGMENT, &[1, 1], &stale].concat();
        late.splice(after..after, segment);
        let icc = profile(128, 2);
        let b = jpeg_with_icc(&late, &icc).unwrap();
        assert_eq!(b.windows(ICC_SEGMENT.len()).filter(|w| *w == ICC_SEGMENT).count(), 1, "one profile segment");
        assert!(ImageBlob::new("image/jpeg", b).cmyk_icc_profile() == Some(icc));
        // Fill bytes before the quantization table's marker are passed over.
        let mut filled = jpg.clone();
        filled.splice(dqt..dqt, [0xFF; 3]);
        assert!(ImageBlob::new("image/jpeg", filled.clone()).cmyk() == Some(inks.clone()), "the decoder reads it");
        let icc = profile(128, 2);
        let b = ImageBlob::new("image/jpeg", jpeg_with_icc(&filled, &icc).unwrap());
        assert!(b.cmyk_icc_profile() == Some(icc), "fill bytes");
        assert!(b.cmyk() == Some(inks.clone()), "fill bytes: the samples as they were");
        assert!(jpeg_with_icc(&jpg, &profile(255 * 65_519, 2)).is_some(), "255 segments");
        assert!(jpeg_with_icc(&jpg, &profile(255 * 65_519 + 1, 2)).is_none(), "256 segments");
        assert!(jpeg_with_icc(&jpg, &[]).is_none(), "empty");
        assert!(jpeg_with_icc(&ImageBlob::cmyk_tiff(&inks).unwrap().bytes, &profile(128, 2)).is_none(), "a TIFF");
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

    /// The CMYK JPEG reader before craft-image-codecs read the inks (zune-jpeg, every sample
    /// inverted, libjpeg's truncating YCCK conversion), the reference [`jpeg_inks`] matches.
    fn zune_inks(bytes: &[u8]) -> Option<Inks> {
        use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
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

    /// CMYK and YCCK JPEGs of many sizes, samplings and qualities, baseline and progressive, with
    /// and without Adobe's marker and an ICC profile, read the inks, size, component count and
    /// profile the zune-jpeg reader read; so do gray and RGB JPEGs (no inks) and cut-off files.
    /// The frames it panicked on (progressive 4:2:0 CMYK, whose C, M and Y `jpeg-encoder` writes at
    /// 1×1 and K at 2×2) read as no inks.
    #[test]
    fn cmyk_jpegs_read_as_the_zune_reader_read_them() {
        use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
        let reference_size = |bytes: &[u8]| {
            use zune_jpeg::zune_core::{bytestream::ZCursor, options::DecoderOptions};
            let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
            d.decode_headers().ok()?;
            let (w, h) = d.dimensions()?;
            Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?, usize::from(d.info()?.components), d.icc_profile().filter(|i| !i.is_empty())))
        };
        let (mut files, mut read, mut panicked) = (0, 0, 0);
        for (i, (w, h)) in [(1u16, 1u16), (7, 3), (16, 16), (33, 31), (100, 77)].into_iter().enumerate() {
            let n = usize::from(w) * usize::from(h);
            let px: Vec<u8> = (0..n * 4).map(|k| ((k * 2_654_435_761 + i * 97) >> 7) as u8).collect();
            for ty in [ColorType::Cmyk, ColorType::CmykAsYcck, ColorType::Rgba, ColorType::Luma] {
                for (q, sampling, progressive, icc) in [
                    (100, SamplingFactor::R_4_4_4, false, false),
                    (70, SamplingFactor::R_4_2_0, true, true),
                    (40, SamplingFactor::R_4_2_2, false, false),
                ] {
                    let mut out = Vec::new();
                    let mut e = Encoder::new(&mut out, q);
                    e.set_sampling_factor(sampling);
                    e.set_progressive(progressive);
                    if icc {
                        let mut icc = vec![4; 70_000];
                        icc[16..20].copy_from_slice(b"CMYK");
                        e.add_icc_profile(&icc).unwrap();
                    }
                    let samples = if ty == ColorType::Luma { &px[..n] } else { &px[..] };
                    e.encode(samples, w, h, ty).unwrap();
                    let cut = out[..out.len() * 2 / 3].to_vec();
                    for file in [out, cut] {
                        let b = ImageBlob::new("image/jpeg", file.clone());
                        // The zune-jpeg reader panicked on some subsampled CMYK frames; those
                        // read as no inks now.
                        let inks = std::panic::catch_unwind(|| zune_inks(&file)).unwrap_or_else(|_| {
                            panicked += 1;
                            None
                        });
                        read += usize::from(inks.is_some());
                        assert_eq!(b.cmyk(), inks, "{w}x{h} {ty:?} q{q}");
                        let r = reference_size(&file);
                        assert_eq!(b.header_size(), r.as_ref().map(|r| (r.0, r.1)));
                        assert_eq!(jpeg_components(&file), r.as_ref().map(|r| r.2));
                        assert_eq!(jpeg_icc(&file), r.and_then(|r| r.3));
                        files += 1;
                    }
                }
            }
        }
        assert_eq!(files, 5 * 4 * 3 * 2);
        assert!(read > 20 && panicked > 0, "{read} read as inks, {panicked} panicked");
    }
}
