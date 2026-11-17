//! The TIFF preview (little-endian, PackBits strips, written by the raster exports' TIFF writer):
//! 1-bit black and white, or 8-bit RGB with or without an alpha channel. Also the PNG thumbnail,
//! and a reader for the palette previews other apps write that the `image` crate can't decode.

use std::borrow::Cow;

use vectorcraft_render::encode::on_white;
use vectorcraft_render::encode::tiff::{self, ByteOrder, Compression, Image, Layout, Photometric};

use crate::Raster;

/// `img` as a TIFF: one bit a pixel (`bw`: black where darker than mid-grey on white), else RGB
/// over white or, with `alpha`, RGBA (unassociated alpha).
pub(crate) fn encode(img: &Raster, bw: bool, alpha: bool) -> Result<Vec<u8>, String> {
    let w = img.width as usize;
    if w == 0 || img.height == 0 || w.checked_mul(img.height as usize).and_then(|n| n.checked_mul(4)) != Some(img.rgba.len()) {
        return Err("the preview image is empty".into());
    }
    let px = img.rgba.as_chunks::<4>().0;
    let (data, photometric, samples, bits): (Cow<[u8]>, _, u16, u16) = if bw {
        let mut data = Vec::with_capacity(w.div_ceil(8) * img.height as usize);
        for line in px.chunks(w) {
            let at = data.len();
            data.resize(at + w.div_ceil(8), 0);
            for (x, p) in line.iter().enumerate() {
                let [r, g, b] = on_white(p);
                let luma = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b);
                // WhiteIsZero: a set bit is black.
                if luma < 128_000
                    && let Some(byte) = data.get_mut(at + x / 8)
                {
                    *byte |= 0x80 >> (x % 8);
                }
            }
        }
        (Cow::Owned(data), Photometric::WhiteIsZero, 1, 1)
    } else if alpha {
        (Cow::Borrowed(&img.rgba), Photometric::Rgb, 4, 8)
    } else {
        (px.iter().flat_map(on_white).collect(), Photometric::Rgb, 3, 8)
    };
    let image = Image { width: img.width, height: img.height, data: &data, photometric, samples, bits, alpha: alpha && !bw };
    tiff::write(&image, &Layout { byte_order: ByteOrder::Little, compression: Compression::PackBits, ppi: 72.0, icc: None })
        .map_err(|e| format!("the preview: {e}"))
}

/// `img` as a PNG.
pub(crate) fn png(img: &Raster) -> Result<Vec<u8>, String> {
    let raw = craft_image_codecs::Image::from_raw(
        img.width,
        img.height,
        craft_image_codecs::ChannelLayout::Rgba,
        craft_image_codecs::SampleType::U8,
        img.rgba.clone(),
    )
    .map_err(|_| "the thumbnail image is malformed".to_string())?;
    craft_image_codecs::encode(&raw, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).map_err(|e| e.to_string())
}

/// Most pixels a palette preview may have.
const MAX_PIXELS: usize = 1 << 26;
/// Most values a TIFF field may have.
const MAX_VALUES: usize = 1 << 20;

/// A TIFF's bytes in its byte order.
struct Tiff<'a> {
    b: &'a [u8],
    big: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.b.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if self.big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.b.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if self.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }

    /// The values of the directory entry at `at` (bytes, shorts or longs; in the entry when they
    /// fit in four bytes).
    fn values(&self, at: usize) -> Option<Vec<u32>> {
        let (kind, n) = (self.u16(at.checked_add(2)?)?, self.u32(at.checked_add(4)?)? as usize);
        let size = match kind {
            1 => 1,
            3 => 2,
            4 => 4,
            _ => return None,
        };
        if n > MAX_VALUES {
            return None;
        }
        let field = at.checked_add(8)?;
        let start = if n * size <= 4 { field } else { self.u32(field)? as usize };
        (0..n)
            .map(|i| {
                let at = start.checked_add(i * size)?;
                match size {
                    1 => self.b.get(at).map(|v| u32::from(*v)),
                    2 => self.u16(at).map(u32::from),
                    _ => self.u32(at),
                }
            })
            .collect()
    }
}

/// An 8-bit palette TIFF, with or without an alpha sample (`ExtraSamples`), uncompressed and
/// interleaved, as RGBA: the layout of other apps' EPS previews. `None` for anything else.
pub(crate) fn palette_rgba(bytes: &[u8]) -> Option<image::RgbaImage> {
    let big = match bytes.get(..4)? {
        b"II*\0" => false,
        b"MM\0*" => true,
        _ => return None,
    };
    let t = Tiff { b: bytes, big };
    let ifd = t.u32(4)? as usize;
    let entries = usize::from(t.u16(ifd)?);
    let mut fields = std::collections::HashMap::new();
    for i in 0..entries {
        let at = ifd.checked_add(2 + i * 12)?;
        fields.insert(t.u16(at)?, at);
    }
    let get = |tag: u16| fields.get(&tag).and_then(|at| t.values(*at));
    let one = |tag: u16| get(tag).and_then(|v| v.first().copied());
    let (w, h) = (one(256)? as usize, one(257)? as usize);
    let samples = one(277).unwrap_or(1) as usize;
    let alpha = samples == 2 && matches!(get(338)?.as_slice(), [0..=2]);
    let plain = one(259).unwrap_or(1) == 1 && one(262)? == 3 && one(284).unwrap_or(1) == 1;
    if !plain || !(samples == 1 || alpha) || get(258)?.iter().any(|b| *b != 8) || w == 0 || h == 0 || w.checked_mul(h)? > MAX_PIXELS {
        return None;
    }
    let map = get(320)?;
    if map.len() != 3 * 256 {
        return None;
    }
    // Strips of `per` rows each, in order.
    let (offsets, counts) = (get(273)?, get(279)?);
    let per = (one(278).unwrap_or(u32::MAX) as usize).min(h);
    let row = w * samples;
    // The strips are in the file: so is all that's allocated.
    if per == 0 || offsets.len() != counts.len() || offsets.len() < h.div_ceil(per) || row * h > bytes.len() {
        return None;
    }
    let mut data = Vec::with_capacity(row * h);
    for (k, (at, n)) in offsets.iter().zip(&counts).take(h.div_ceil(per)).enumerate() {
        let want = per.min(h - k * per) * row;
        if (*n as usize) < want {
            return None;
        }
        let at = *at as usize;
        data.extend_from_slice(bytes.get(at..at.checked_add(want)?)?);
    }
    // Map entries are 16-bit.
    let channel = |c: usize, i: usize| map.get(c * 256 + i).map_or(0, |v| ((u64::from(*v).min(65_535) * 255 + 32_767) / 65_535) as u8);
    let rgba = data
        .chunks_exact(samples)
        .flat_map(|px| {
            let i = usize::from(px.first().copied().unwrap_or(0));
            [channel(0, i), channel(1, i), channel(2, i), px.get(1).copied().unwrap_or(255)]
        })
        .collect();
    image::RgbaImage::from_raw(w as u32, h as u32, rgba)
}
