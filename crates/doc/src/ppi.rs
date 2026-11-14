//! Image resolution metadata (pixels per inch) in the formats the image decoder reads: PNG `pHYs`,
//! JPEG JFIF density or EXIF, TIFF and WebP EXIF resolution tags, and BMP pixels per metre. An image
//! converted to PNG keeps it as a `pHYs` chunk ([`with_png_resolution`]).

const INCH_M: f64 = 0.0254;
const INCH_CM: f64 = 2.54;

/// Is `bytes` a PNG, JPEG, TIFF, WebP or BMP file, the formats [`resolution`] reads?
pub fn reads(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8])
        || bytes.starts_with(b"II*\0")
        || bytes.starts_with(b"MM\0*")
        || (bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
        || bytes.starts_with(b"BM")
}

/// The resolution a PNG, JPEG, TIFF, WebP or BMP file declares as (horizontal, vertical) ppi, each
/// from 1 to 100,000; `None` when it declares none (or only an aspect ratio), and for other files.
pub fn resolution(bytes: &[u8]) -> Option<(f64, f64)> {
    let r = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg(bytes)
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        tiff(bytes)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp(bytes)
    } else if bytes.starts_with(b"BM") {
        bmp(bytes)
    } else {
        None
    };
    r.filter(|(x, y)| [*x, *y].iter().all(|v| v.is_finite() && (1.0..=100_000.0).contains(v)))
}

/// `n` bytes at `at` (`None` past the end; offsets come from the file, so no overflow either).
fn slice(b: &[u8], at: usize, n: usize) -> Option<&[u8]> {
    b.get(at..at.checked_add(n)?)
}
fn bytes<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    slice(b, at, N)?.try_into().ok()
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    bytes(b, at).map(u16::from_be_bytes)
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    bytes(b, at).map(u32::from_be_bytes)
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    bytes(b, at).map(u32::from_le_bytes)
}

/// Pixels per inch from the whole pixels per metre PNG and BMP store: a whole number of ppi when
/// the metre value is that rounded (300 ppi is stored as 11811 per metre, 299.9994 ppi).
fn per_metre(ppm: u32) -> f64 {
    let v = ppm as f64 * INCH_M;
    let whole = v.round();
    if (v - whole).abs() <= INCH_M / 2.0 + 1e-9 { whole } else { v }
}

/// PNG chunks after the signature: (type, data).
pub fn png_chunks(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = 8;
    std::iter::from_fn(move || {
        let len = be32(b, at)? as usize;
        let ty = slice(b, at + 4, 4)?;
        let data = slice(b, at + 8, len)?;
        at = at.checked_add(12 + len)?;
        Some((ty, data))
    })
}

fn png(b: &[u8]) -> Option<(f64, f64)> {
    let (_, d) = png_chunks(b).take_while(|(ty, _)| *ty != b"IDAT").find(|(ty, _)| *ty == b"pHYs")?;
    // Unit 1 = metre; 0 only gives the pixel aspect ratio.
    (d.get(8) == Some(&1)).then(|| (per_metre(be32(d, 0).unwrap_or(0)), per_metre(be32(d, 4).unwrap_or(0))))
}

fn jpeg(b: &[u8]) -> Option<(f64, f64)> {
    let (mut jfif, mut exif) = (None, None);
    let mut at = 2;
    while at + 4 <= b.len() && b[at] == 0xff {
        let marker = b[at + 1];
        // 0xff fill bytes pad between segments.
        if marker == 0xff {
            at += 1;
            continue;
        }
        if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            at += 2;
            continue;
        }
        // Start of scan or end of image: no more metadata.
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let len = be16(b, at + 2)? as usize;
        let seg = slice(b, at + 4, len.checked_sub(2)?)?;
        match marker {
            0xe0 if seg.starts_with(b"JFIF\0") && seg.len() >= 12 => {
                let (x, y) = (be16(seg, 8)? as f64, be16(seg, 10)? as f64);
                jfif = match seg[7] {
                    1 => Some((x, y)),
                    2 => Some((x * INCH_CM, y * INCH_CM)),
                    _ => None,
                };
            }
            0xe1 if seg.starts_with(b"Exif\0\0") => exif = tiff(&seg[6..]),
            _ => {}
        }
        at += 2 + len;
    }
    jfif.or(exif)
}

/// TIFF (or EXIF) IFD0 XResolution / YResolution / ResolutionUnit, as (horizontal, vertical) ppi
/// in the stored pixels' axes, unchecked; `None` without an absolute unit.
pub fn tiff(b: &[u8]) -> Option<(f64, f64)> {
    let le = b.starts_with(b"II");
    let u16_at = |at: usize| bytes(b, at).map(if le { u16::from_le_bytes } else { u16::from_be_bytes });
    let u32_at = |at: usize| bytes(b, at).map(if le { u32::from_le_bytes } else { u32::from_be_bytes });
    let ifd = u32_at(4)? as usize;
    let (mut x, mut y, mut unit) = (None, None, 2);
    for i in 0..u16_at(ifd)? as usize {
        let e = ifd.checked_add(2 + i * 12)?;
        let rational = || -> Option<f64> {
            let at = u32_at(e + 8)? as usize;
            let (n, d) = (u32_at(at)?, u32_at(at.checked_add(4)?)?);
            (d != 0).then(|| n as f64 / d as f64)
        };
        match u16_at(e)? {
            282 => x = rational(),
            283 => y = rational(),
            296 => unit = u16_at(e + 8)?,
            _ => {}
        }
    }
    let k = match unit {
        2 => 1.0,
        3 => INCH_CM,
        _ => return None,
    };
    let x = x?;
    Some((x * k, y.unwrap_or(x) * k))
}

fn webp(b: &[u8]) -> Option<(f64, f64)> {
    let mut at = 12;
    while at + 8 <= b.len() {
        let len = le32(b, at + 4)? as usize;
        let data = b.get(at + 8..at + 8 + len)?;
        if &b[at..at + 4] == b"EXIF" {
            return tiff(data.strip_prefix(b"Exif\0\0").unwrap_or(data));
        }
        at += 8 + len + (len & 1);
    }
    None
}

fn bmp(b: &[u8]) -> Option<(f64, f64)> {
    // BITMAPINFOHEADER (40 bytes or longer) at 14: pixels per metre at +24 and +28.
    if le32(b, 14)? < 40 {
        return None;
    }
    let ppm = |at| le32(b, at).filter(|v| (1..=i32::MAX as u32).contains(v)).map(per_metre);
    Some((ppm(38)?, ppm(42)?))
}

/// Pixels per metre for a resolution in pixels per inch (the unit `pHYs` stores).
pub fn pixels_per_metre(ppi: f64) -> u32 {
    (ppi / INCH_M).round().clamp(1.0, u32::MAX as f64) as u32
}

/// PNG CRC-32 (the polynomial ZIP uses) of `parts` read one after another.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for &b in parts.iter().flat_map(|p| p.iter()) {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & u32::from(crc & 1 != 0).wrapping_neg());
        }
    }
    !crc
}

/// A whole `pHYs` chunk (length, type, data, CRC) declaring `x` × `y` pixels per inch.
pub fn phys_chunk(x: f64, y: f64) -> Vec<u8> {
    let [x, y] = [x, y].map(|v| pixels_per_metre(v).to_be_bytes());
    // x, y, unit 1 = metre.
    let data = [x.as_slice(), y.as_slice(), &[1]].concat();
    let mut out = Vec::with_capacity(21);
    out.extend_from_slice(&9u32.to_be_bytes());
    out.extend_from_slice(b"pHYs");
    out.extend_from_slice(&data);
    out.extend_from_slice(&crc32(&[b"pHYs", &data]).to_be_bytes());
    out
}

/// `png` with a `pHYs` chunk declaring `ppi` right after the header (any existing one is replaced).
pub fn with_png_resolution(png: &[u8], (x, y): (f64, f64)) -> Vec<u8> {
    let Some(head) = be32(png, 8).and_then(|len| slice(png, 0, 20usize.checked_add(len as usize)?)) else { return png.to_vec() };
    let chunk = phys_chunk(x, y);
    let mut out = Vec::with_capacity(png.len() + chunk.len());
    out.extend_from_slice(head);
    out.extend_from_slice(&chunk);
    let mut at = head.len();
    while let Some(chunk) = be32(png, at).and_then(|len| slice(png, at, 12usize.checked_add(len as usize)?)) {
        if chunk.get(4..8) != Some(b"pHYs") {
            out.extend_from_slice(chunk);
        }
        at += chunk.len();
    }
    out.extend_from_slice(png.get(at..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::new(w, h).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// The `pHYs` chunk carries the CRC a PNG reader checks, and replaces the one already there.
    #[test]
    fn a_png_takes_its_resolution_and_still_decodes() {
        let png = png_of(4, 3);
        assert_eq!(resolution(&png), None);
        let tagged = with_png_resolution(&png, (300.0, 150.0));
        assert_eq!(resolution(&tagged), Some((300.0, 150.0)));
        let again = with_png_resolution(&tagged, (72.0, 72.0));
        assert_eq!(resolution(&again), Some((72.0, 72.0)), "replaced, not duplicated");
        assert_eq!(again.len(), tagged.len());
        let img = image::load_from_memory(&again).unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
        // Every PNG ends with this IEND chunk CRC.
        assert_eq!(crc32(&[b"IEND"]), 0xAE42_6082);
    }
}
