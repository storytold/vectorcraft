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

/// The color type of TIFF `bytes` and its decoder; `None` for [`oversized_tiff`] images too.
fn tiff(bytes: &[u8]) -> Option<(tiff::ColorType, tiff::decoder::Decoder<Cursor<&[u8]>>)> {
    if !is_tiff(bytes) || oversized_tiff(bytes).is_some() {
        return None;
    }
    let mut d = tiff::decoder::Decoder::new(Cursor::new(bytes)).ok()?;
    Some((d.colortype().ok()?, d))
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
fn read_samples(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>, n: usize, per: usize) -> Option<Vec<u8>> {
    let mut buf = tiff::decoder::DecodingResult::U8(Vec::new());
    let layout = d.read_image_to_buffer(&mut buf).ok()?;
    let data = match buf {
        tiff::decoder::DecodingResult::U8(v) => v,
        tiff::decoder::DecodingResult::U16(v) => v.iter().map(|s| (s >> 8) as u8).collect(),
        _ => return None,
    };
    if Some(data.len()) != n.checked_mul(per) {
        return None;
    }
    if layout.planes <= 1 {
        return Some(data);
    }
    if layout.planes != per {
        return None;
    }
    let planes: Vec<&[u8]> = data.chunks_exact(n).collect();
    Some((0..n).flat_map(|i| planes.iter().map(move |p| p.get(i).copied().unwrap_or(0))).collect())
}

impl ImageBlob {
    /// A CMYK TIFF of ink amounts `inks` (Deflate-compressed); `None` when it can't be written.
    pub fn cmyk_tiff(inks: &Inks) -> Option<Self> {
        Self::cmyk_tiff_at(inks, None)
    }

    /// [`ImageBlob::cmyk_tiff`], declaring the resolution `ppi` (horizontal, vertical pixels per
    /// inch) when it is given and both are finite and above 0.
    pub fn cmyk_tiff_at(inks: &Inks, ppi: Option<(f64, f64)>) -> Option<Self> {
        use tiff::encoder::{Compression, Rational, TiffEncoder, colortype::CMYK8, compression::DeflateLevel};
        use tiff::tags::{ResolutionUnit, Tag};
        let mut out = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut out).ok()?.with_compression(Compression::Deflate(DeflateLevel::Balanced));
        let mut img = enc.new_image::<CMYK8>(inks.width, inks.height).ok()?;
        if let Some((x, y)) = ppi.filter(|(x, y)| [x, y].iter().all(|v| v.is_finite() && **v > 0.0)) {
            // Ten-thousandths of a pixel per inch.
            let r = |v: f64| Rational { n: (v * 10_000.0).round() as u32, d: 10_000 };
            img.encoder().write_tag(Tag::XResolution, r(x)).ok()?;
            img.encoder().write_tag(Tag::YResolution, r(y)).ok()?;
            img.encoder().write_tag(Tag::ResolutionUnit, ResolutionUnit::Inch).ok()?;
        }
        img.write_data(&inks.data).ok()?;
        Some(Self::new("image/tiff", out.into_inner()))
    }

    /// Does the image hold CMYK samples (a four-component JPEG or a CMYK TIFF)? Reads only its
    /// header.
    pub fn is_cmyk(&self) -> bool {
        jpeg_components(&self.bytes) == Some(4) || tiff(&self.bytes).is_some_and(|(t, _)| matches!(t, tiff::ColorType::CMYK(8 | 16)))
    }

    /// The ink amounts of a CMYK image; `None` for other images, and ones that can't be decoded
    /// or are larger than [`MAX_CMYK_PIXELS`].
    pub fn cmyk(&self) -> Option<Inks> {
        let bytes = self.bytes.as_slice();
        if bytes.starts_with(&[0xFF, 0xD8]) { jpeg_inks(bytes) } else { tiff_inks(bytes) }
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

/// CMYK TIFF `bytes` as Open and Place store them: as they are, or, when they store one plane per
/// ink, which the image decoder doesn't read, their ink amounts written again by
/// [`ImageBlob::cmyk_tiff_at`] with the resolution `ppi`. `None` for other images and for CMYK
/// TIFFs whose inks [`ImageBlob::cmyk`] doesn't read.
pub fn stored_cmyk_tiff(bytes: &[u8], ppi: Option<(f64, f64)>) -> Option<Vec<u8>> {
    let inks = tiff_inks(bytes)?;
    let (_, mut d) = tiff(bytes)?;
    if !planar(&mut d) {
        return Some(bytes.to_vec());
    }
    ImageBlob::cmyk_tiff_at(&inks, ppi).map(|b| Arc::unwrap_or_clone(b.bytes))
}

/// A TIFF whose pixels have samples after their colour, which the image decoder doesn't read at
/// all, as RGBA pixels: CMYK with an alpha channel (5 samples), each colour converted by `rgb` (ink
/// amounts 0..1 → sRGB 0..1; the inks can't be kept with the transparency, so they show as RGB),
/// and grayscale with extra samples, of which the first is the alpha when the file says so (other
/// channels are left out). Associated (premultiplied) alpha is divided back out. The samples may be
/// stored together or one plane per channel. `None` for other images and ones larger than
/// [`MAX_CMYK_PIXELS`].
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
    let data = read_samples(&mut d, n, samples)?;
    let mut out = Vec::with_capacity(n * 4);
    for p in data.chunks_exact(samples) {
        let opacity = if alpha { p.get(colour).copied().unwrap_or(u8::MAX) } else { u8::MAX };
        let a = f32::from(opacity) / 255.0;
        let level = |v: u8| {
            let v = f32::from(v) / 255.0;
            if extra == Some(1) && a > 0.0 { (v / a).min(1.0) } else { v }
        };
        let [r, g, b] = match *p {
            [c, m, y, k, ..] if colour == 4 => rgb([level(c), level(m), level(y), level(k)]),
            [v, ..] if photometric == Some(0) => [1.0 - level(v); 3],
            [v, ..] => [level(v); 3],
            [] => continue,
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
        let rgb = image::load_from_memory(&b.bytes).unwrap().to_rgb8();
        assert_eq!(rgb.get_pixel(1, 0).0, [0, 255, 255]);
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
