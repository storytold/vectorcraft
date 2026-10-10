//! Image resolution metadata (pixels per inch): PNG `pHYs`, JPEG JFIF density or EXIF, TIFF and
//! WebP EXIF resolution tags, BMP pixels per metre, a Photoshop document's ResolutionInfo. Place
//! sizes an image by it (72 ppi when the file has none); images converted to PNG on import keep it
//! as a `pHYs` chunk.

const INCH_M: f64 = 0.0254;
const INCH_CM: f64 = 2.54;

/// The resolution a file declares as (horizontal, vertical) ppi; `None` when it declares none (or
/// only an aspect ratio).
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
    } else if super::psdread::is_psd(bytes) {
        super::psdread::resolution(bytes)
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
pub(super) fn png_chunks(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
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

/// TIFF (or EXIF) IFD0 XResolution / YResolution / ResolutionUnit.
fn tiff(b: &[u8]) -> Option<(f64, f64)> {
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

/// `png` with a `pHYs` chunk declaring `ppi` right after the header (any existing one is replaced).
pub fn with_png_resolution(png: &[u8], (x, y): (f64, f64)) -> Vec<u8> {
    let Some(head) = be32(png, 8).and_then(|len| slice(png, 0, 20usize.checked_add(len as usize)?)) else { return png.to_vec() };
    let chunk = vectorcraft_render::encode::png::phys_chunk(x, y);
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

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn png_phys_round_trips_and_still_decodes() {
        let png = png_of(4, 3);
        assert_eq!(resolution(&png), None);
        let tagged = with_png_resolution(&png, (300.0, 150.0));
        assert!(close(resolution(&tagged).unwrap(), (300.0, 150.0)), "{:?}", resolution(&tagged));
        let again = with_png_resolution(&tagged, (72.0, 72.0));
        assert!(close(resolution(&again).unwrap(), (72.0, 72.0)), "replaced, not duplicated");
        assert_eq!(again.len(), tagged.len());
        let img = image::load_from_memory(&again).unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
    }

    #[test]
    fn jpeg_jfif_density_in_dpi_and_dpcm() {
        let mut jpg = Vec::new();
        image::RgbImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
        // The encoder writes JFIF with units 0 (aspect only).
        assert_eq!(resolution(&jpg), None);
        let jfif = jpg.windows(5).position(|w| w == b"JFIF\0").unwrap();
        jpg[jfif + 7] = 1;
        jpg[jfif + 8..jfif + 10].copy_from_slice(&240u16.to_be_bytes());
        jpg[jfif + 10..jfif + 12].copy_from_slice(&240u16.to_be_bytes());
        assert_eq!(resolution(&jpg), Some((240.0, 240.0)));
        jpg[jfif + 7] = 2;
        jpg[jfif + 8..jfif + 10].copy_from_slice(&100u16.to_be_bytes());
        assert!(close(resolution(&jpg).unwrap(), (254.0, 240.0 * 2.54)));
    }

    /// A little-endian TIFF/EXIF block declaring `x`/`y` resolution in `unit`.
    fn exif_block(x: u32, y: u32, unit: u16) -> Vec<u8> {
        let mut b = b"II*\0".to_vec();
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        let data = 8 + 2 + 3 * 12 + 4;
        for (tag, ty, val) in [(282u16, 5u16, data as u32), (283, 5, data as u32 + 8), (296, 3, unit as u32)] {
            b.extend_from_slice(&tag.to_le_bytes());
            b.extend_from_slice(&ty.to_le_bytes());
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend_from_slice(&val.to_le_bytes());
        }
        b.extend_from_slice(&0u32.to_le_bytes());
        for v in [x, 1, y, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn tiff_and_exif_resolution_tags() {
        assert_eq!(resolution(&exif_block(300, 200, 2)), Some((300.0, 200.0)));
        assert!(close(resolution(&exif_block(100, 100, 3)).unwrap(), (254.0, 254.0)));
        assert_eq!(resolution(&exif_block(300, 300, 1)), None, "no absolute unit");
        // WebP carries EXIF in its own chunk.
        let exif = exif_block(150, 150, 2);
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend_from_slice(b"EXIF");
        webp.extend_from_slice(&(exif.len() as u32).to_le_bytes());
        webp.extend_from_slice(&exif);
        assert_eq!(resolution(&webp), Some((150.0, 150.0)));
    }

    #[test]
    fn bmp_pixels_per_metre() {
        let mut bmp = Vec::new();
        image::RgbImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp).unwrap();
        bmp[38..42].copy_from_slice(&11811u32.to_le_bytes());
        bmp[42..46].copy_from_slice(&11811u32.to_le_bytes());
        assert!(close(resolution(&bmp).unwrap(), (300.0, 300.0)));
        assert_eq!(resolution(b"GIF89a"), None);
    }
}
