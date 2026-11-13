//! A PNG writer for raster exports: 8-bit RGBA (RGB when every pixel is opaque) or indexed
//! (PNG-8: a palette of up to 256 colours, 1–8 bits per pixel), adaptive row filters, the
//! resolution as a `pHYs` chunk and optional Adam7 interlacing (the image builds up progressively
//! while it loads). Chunks are written here; flate2 compresses the image data.

use std::borrow::Cow;
use std::io::Write as _;

use flate2::Crc;
use flate2::write::ZlibEncoder;

use super::quantize::Indexed;

/// What goes into the file besides the pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PngOptions {
    /// Pixels per inch, stored as pixels per metre in `pHYs` (`None`: no `pHYs` chunk).
    pub ppi: Option<f64>,
    /// Adam7 interlacing.
    pub interlaced: bool,
}

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// Most image data bytes per `IDAT` chunk.
const IDAT_MAX: usize = 1 << 20;
/// Adam7 passes: (x0, y0, dx, dy).
const ADAM7: [(u32, u32, u32, u32); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

/// Pixels per metre for a resolution in pixels per inch (the unit `pHYs` stores).
pub fn pixels_per_metre(ppi: f64) -> u32 {
    (ppi / 0.0254).round().clamp(1.0, u32::MAX as f64) as u32
}

/// A whole `pHYs` chunk (length, type, data, CRC) declaring `x` × `y` pixels per inch.
pub fn phys_chunk(x: f64, y: f64) -> Vec<u8> {
    let [x, y] = [x, y].map(|v| pixels_per_metre(v).to_be_bytes());
    let mut out = Vec::with_capacity(21);
    // x, y, unit 1 = metre.
    chunk(&mut out, b"pHYs", &[x.as_slice(), y.as_slice(), &[1]].concat());
    out
}

/// Encode straight-alpha RGBA8 pixels (`width`×`height`, row-major) as a PNG file.
pub fn encode(rgba: &[u8], width: u32, height: u32, o: &PngOptions) -> Result<Vec<u8>, String> {
    check_size(width, height)?;
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err("PNG encoding failed: pixel buffer doesn't match the image size".into());
    }
    let px = rgba.as_chunks::<4>().0;
    let opaque = px.iter().all(|p| p[3] == 255);
    let (channels, color_type) = if opaque { (3, 2) } else { (4, 6) };
    let pixels: Cow<[u8]> = if opaque { Cow::Owned(px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()) } else { Cow::Borrowed(rgba) };
    write(&pixels, width, height, Layout { channels, bits: 8, color_type }, &[], o)
}

/// Encode grey levels with alpha (two bytes a pixel, `width`×`height`, row-major) as a greyscale
/// PNG: grey and alpha (colour type 4), or grey alone (colour type 0) when every pixel is opaque.
pub fn encode_gray(gray_alpha: &[u8], width: u32, height: u32, o: &PngOptions) -> Result<Vec<u8>, String> {
    check_size(width, height)?;
    if gray_alpha.len() as u64 != u64::from(width) * u64::from(height) * 2 {
        return Err("PNG encoding failed: pixel buffer doesn't match the image size".into());
    }
    let px = gray_alpha.as_chunks::<2>().0;
    if px.iter().all(|p| p[1] == 255) {
        let levels: Vec<u8> = px.iter().map(|p| p[0]).collect();
        return write(&levels, width, height, Layout { channels: 1, bits: 8, color_type: 0 }, &[], o);
    }
    write(gray_alpha, width, height, Layout { channels: 2, bits: 8, color_type: 4 }, &[], o)
}

/// Encode a palette image as an indexed PNG (colour type 3: `PLTE`, and `tRNS` for its
/// transparent entry) at 1, 2, 4 or 8 bits per pixel, as few as the palette needs.
pub fn encode_indexed(img: &Indexed, o: &PngOptions) -> Result<Vec<u8>, String> {
    check_size(img.width, img.height)?;
    if img.indices.len() as u64 != u64::from(img.width) * u64::from(img.height) || img.palette.is_empty() || img.palette.len() > 256 {
        return Err("PNG encoding failed: the palette image is malformed".into());
    }
    let bits = match img.palette.len() {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    };
    let plte = img.palette.concat();
    // Alpha of the entries up to the transparent one (the rest are opaque).
    let trns: Vec<u8> = img.transparent.map(|t| (0..=t).map(|i| if i == t { 0 } else { 255 }).collect()).unwrap_or_default();
    let mut extra: Vec<(&[u8; 4], &[u8])> = vec![(b"PLTE", &plte)];
    if !trns.is_empty() {
        extra.push((b"tRNS", &trns));
    }
    write(&img.indices, img.width, img.height, Layout { channels: 1, bits, color_type: 3 }, &extra, o)
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(format!("PNG encoding failed: {width} × {height} pixels is not a valid image size"));
    }
    Ok(())
}

/// How pixels are stored: bytes per pixel (one per index for palette images), bits per sample
/// in the file and the PNG colour type.
#[derive(Clone, Copy)]
struct Layout {
    channels: usize,
    bits: u8,
    color_type: u8,
}

/// Write the file: signature, `IHDR`, `pHYs`, the `extra` chunks, the filtered and compressed
/// pixels (`layout.channels` bytes each, packed to `layout.bits` in the file) and `IEND`.
fn write(pixels: &[u8], width: u32, height: u32, layout: Layout, extra: &[(&[u8; 4], &[u8])], o: &PngOptions) -> Result<Vec<u8>, String> {
    let Layout { channels, bits, color_type } = layout;
    let mut out = Vec::with_capacity(pixels.len() / 2 + 64);
    out.extend_from_slice(SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // Bit depth, colour type, deflate compression, adaptive filtering, interlace method.
    ihdr.extend_from_slice(&[bits, color_type, 0, 0, u8::from(o.interlaced)]);
    chunk(&mut out, b"IHDR", &ihdr);
    if let Some(ppi) = o.ppi {
        out.extend_from_slice(&phys_chunk(ppi, ppi));
    }
    for (ty, data) in extra {
        chunk(&mut out, ty, data);
    }
    // Filters work on whole bytes: a sub-byte pixel counts as one.
    let bpp = (channels * bits as usize / 8).max(1);
    let mut raw = Vec::with_capacity(pixels.len() + height as usize * if o.interlaced { 2 } else { 1 });
    let mut rows = |px: &[u8], w: u32, h: u32| {
        let packed = pack(px, w, h, bits);
        filter_rows(&packed, (w as usize * channels * bits as usize).div_ceil(8), h, bpp, &mut raw);
    };
    if o.interlaced {
        for pass in ADAM7 {
            if let Some((w, h, px)) = sub_image(pixels, width, height, channels, pass) {
                rows(&px, w, h);
            }
        }
    } else {
        rows(pixels, width, height);
    }
    let mut z = ZlibEncoder::new(Vec::with_capacity(raw.len() / 2), flate2::Compression::default());
    let data = z.write_all(&raw).and_then(|()| z.finish()).map_err(|e| format!("PNG encoding failed: {e}"))?;
    for part in data.chunks(IDAT_MAX) {
        chunk(&mut out, b"IDAT", part);
    }
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// The signature and the IHDR chunk (length, type, 13 data bytes, CRC), which comes first.
const HEADER: usize = 8 + 12 + 13;

/// `png` (a PNG file) with a text chunk per `(keyword, text)` right after its header: `tEXt`, or
/// uncompressed `iTXt` (UTF-8) for text beyond Latin-1. Keywords are 1–79 printable ASCII
/// characters (others are skipped). Unchanged when it doesn't start with a PNG header.
pub fn with_text(png: Vec<u8>, entries: &[(&str, Cow<str>)]) -> Vec<u8> {
    let chunks: Vec<([u8; 4], Vec<u8>)> = entries
        .iter()
        .filter(|(keyword, _)| !keyword.is_empty() && keyword.len() <= 79 && keyword.bytes().all(|b| (32..=126).contains(&b)))
        .map(|(keyword, text)| {
            let mut data = keyword.as_bytes().to_vec();
            data.push(0);
            let latin1: Option<Vec<u8>> = text.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect();
            match latin1 {
                Some(bytes) => {
                    data.extend(bytes);
                    (*b"tEXt", data)
                }
                None => {
                    // Uncompressed, no language tag, no translated keyword.
                    data.extend_from_slice(&[0, 0, 0, 0]);
                    data.extend_from_slice(text.as_bytes());
                    (*b"iTXt", data)
                }
            }
        })
        .collect();
    with_chunks(png, &chunks)
}

/// `png` marked as sRGB (an `sRGB` chunk, perceptual intent) right after its header, so browsers
/// show its colours as they are. Unchanged when it doesn't start with a PNG header.
pub fn with_srgb(png: Vec<u8>) -> Vec<u8> {
    with_chunks(png, &[(*b"sRGB", vec![0])])
}

/// `png` with `chunks` (type, data) right after its header; chunks too large for a PNG are
/// skipped. Unchanged when there are none or it doesn't start with a PNG header.
fn with_chunks(png: Vec<u8>, chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    if chunks.is_empty() || !png.starts_with(SIGNATURE) || png.get(12..16) != Some(b"IHDR".as_slice()) || png.len() < HEADER {
        return png;
    }
    let mut out = Vec::with_capacity(png.len() + chunks.iter().map(|(_, d)| d.len() + 12).sum::<usize>());
    out.extend_from_slice(&png[..HEADER]);
    for (ty, data) in chunks {
        if u32::try_from(data.len()).is_ok() {
            chunk(&mut out, ty, data);
        }
    }
    out.extend_from_slice(&png[HEADER..]);
    out
}

/// Rows of one-byte pixels packed `bits` per pixel, most significant first (each row starts on a
/// byte); at 8 bits the pixels as they are.
pub(super) fn pack(px: &[u8], w: u32, h: u32, bits: u8) -> Cow<'_, [u8]> {
    if bits >= 8 {
        return Cow::Borrowed(px);
    }
    let per = (8 / bits) as usize;
    let mut out = Vec::with_capacity(h as usize * (w as usize).div_ceil(per));
    for row in px.chunks(w.max(1) as usize) {
        for group in row.chunks(per) {
            let byte = group.iter().enumerate().fold(0u8, |b, (i, v)| b | (v & ((1 << bits) - 1)) << (8 - bits as usize * (i + 1)));
            out.push(byte);
        }
    }
    Cow::Owned(out)
}

/// Append one chunk: length, type, data, CRC of type and data.
fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.update(ty);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

/// The pixels of one Adam7 pass as their own image (`None` when the pass is empty).
fn sub_image(px: &[u8], width: u32, height: u32, channels: usize, (x0, y0, dx, dy): (u32, u32, u32, u32)) -> Option<(u32, u32, Vec<u8>)> {
    let w = width.saturating_sub(x0).div_ceil(dx);
    let h = height.saturating_sub(y0).div_ceil(dy);
    if w == 0 || h == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(w as usize * h as usize * channels);
    for y in (y0..height).step_by(dy as usize) {
        let row = &px[y as usize * width as usize * channels..][..width as usize * channels];
        for x in (x0..width).step_by(dx as usize) {
            out.extend_from_slice(&row[x as usize * channels..][..channels]);
        }
    }
    Some((w, h, out))
}

/// Filter every row (`stride` bytes, `h` rows) into `out` (filter type byte + filtered bytes),
/// picking per row the filter with the smallest sum of absolute values (the usual heuristic).
fn filter_rows(px: &[u8], stride: usize, h: u32, bpp: usize, out: &mut Vec<u8>) {
    let zero = vec![0u8; stride];
    let mut cand: [Vec<u8>; 5] = std::array::from_fn(|_| vec![0u8; stride]);
    for y in 0..h as usize {
        let cur = &px[y * stride..][..stride];
        let prev = if y == 0 { &zero[..] } else { &px[(y - 1) * stride..][..stride] };
        // The first pixel has no left neighbour (a = c = 0).
        let (head, tail) = (bpp.min(stride), stride.saturating_sub(bpp));
        let [none, sub, up, avg, pae] = &mut cand;
        none.copy_from_slice(cur);
        sub[..head].copy_from_slice(&cur[..head]);
        for i in 0..stride {
            up[i] = cur[i].wrapping_sub(prev[i]);
        }
        for i in 0..head {
            avg[i] = cur[i].wrapping_sub(prev[i] / 2);
            pae[i] = cur[i].wrapping_sub(paeth(0, prev[i], 0));
        }
        for j in 0..tail {
            let i = j + bpp;
            let (a, b, c) = (cur[j], prev[i], prev[j]);
            sub[i] = cur[i].wrapping_sub(a);
            avg[i] = cur[i].wrapping_sub(((a as u16 + b as u16) / 2) as u8);
            pae[i] = cur[i].wrapping_sub(paeth(a, b, c));
        }
        let cost = |c: &[u8]| c.iter().map(|&v| (v as i8).unsigned_abs() as u64).sum::<u64>();
        let best = (0..5).min_by_key(|&f| cost(&cand[f])).unwrap_or(0);
        out.push(best as u8);
        out.extend_from_slice(&cand[best]);
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}
