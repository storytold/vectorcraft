//! Image resolution metadata (pixels per inch): PNG `pHYs`, JPEG JFIF density or EXIF, TIFF and
//! WebP and JPEG XL EXIF resolution tags, BMP pixels per metre, a Photoshop document's ResolutionInfo, a PCX
//! header's HDpi and VDpi, JP2 capture and display resolution boxes, or a HEIF image's EXIF
//! resolution tags. Place
//! sizes an image by it (72 ppi when the file has none); images converted to PNG on import keep it
//! as a `pHYs` chunk. The formats the image decoder reads are read by [`vectorcraft_doc::ppi`].

pub(super) use vectorcraft_doc::ppi::png_chunks;
use vectorcraft_doc::ppi::tiff;
pub use vectorcraft_doc::ppi::with_png_resolution;

const INCH_M: f64 = 0.0254;

/// The resolution a file declares as (horizontal, vertical) ppi; `None` when it declares none (or
/// only an aspect ratio).
pub fn resolution(bytes: &[u8]) -> Option<(f64, f64)> {
    if vectorcraft_doc::ppi::reads(bytes) {
        return vectorcraft_doc::ppi::resolution(bytes);
    }
    let r = if super::jxlread::is_jxl(bytes) {
        jxl(bytes)
    } else if super::psdread::is_psd(bytes) {
        super::psdread::resolution(bytes)
    } else if super::pcxread::is_pcx(bytes) {
        super::pcxread::resolution(bytes)
    } else if super::jp2read::is_jpeg2000(bytes) {
        jp2(bytes)
    } else if super::heifread::is_heif(bytes) {
        super::heifread::with_exif(bytes, tiff)
    } else {
        None
    };
    r.filter(|(x, y)| [*x, *y].iter().all(|v| v.is_finite() && (1.0..=100_000.0).contains(v)))
}

fn jp2_axis(n: u16, d: u16, e: i8) -> Option<f64> {
    if d == 0 {
        return None;
    }
    let ppi = f64::from(n) / f64::from(d) * 10f64.powi(i32::from(e)) * INCH_M;
    (ppi.is_finite() && (1.0..=100_000.0).contains(&ppi)).then_some(ppi)
}

/// JP2 resolution is independent on each axis. A valid display (`resd`) value, the resolution the
/// image is meant to be reproduced at, wins over a capture (`resc`) value whichever box comes first;
/// the capture value is used for an axis with no valid display value. Within one box type, later
/// boxes replace earlier valid values for that axis; an invalid later axis leaves the earlier value intact.
fn jp2(b: &[u8]) -> Option<(f64, f64)> {
    if !b.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n") {
        return Some((72.0, 72.0));
    }
    // [capture, display], each (x, y).
    let mut found = [(None, None); 2];
    for (_, header) in super::jp2read::boxes(b).filter(|(ty, _)| *ty == b"jp2h") {
        for (_, res) in super::jp2read::boxes(header).filter(|(ty, _)| *ty == b"res ") {
            for (ty, data) in super::jp2read::boxes(res).filter(|(ty, _)| *ty == b"resc" || *ty == b"resd") {
                let Some([vn0, vn1, vd0, vd1, hn0, hn1, hd0, hd1, ve, he]): Option<[u8; 10]> = data.get(..10).and_then(|v| v.try_into().ok()) else {
                    continue;
                };
                let (x, y) = &mut found[usize::from(ty == b"resd")];
                if let Some(value) = jp2_axis(u16::from_be_bytes([hn0, hn1]), u16::from_be_bytes([hd0, hd1]), he as i8) {
                    *x = Some(value);
                }
                if let Some(value) = jp2_axis(u16::from_be_bytes([vn0, vn1]), u16::from_be_bytes([vd0, vd1]), ve as i8) {
                    *y = Some(value);
                }
            }
        }
    }
    let [(cx, cy), (dx, dy)] = found;
    Some((dx.or(cx).unwrap_or(72.0), dy.or(cy).unwrap_or(72.0)))
}

/// `n` bytes at `at` (`None` past the end; offsets come from the file, so no overflow either).
fn slice(b: &[u8], at: usize, n: usize) -> Option<&[u8]> {
    b.get(at..at.checked_add(n)?)
}
fn bytes<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    slice(b, at, N)?.try_into().ok()
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    bytes(b, at).map(u32::from_be_bytes)
}

fn jxl(b: &[u8]) -> Option<(f64, f64)> {
    if !b.starts_with(&[0, 0, 0, 12, b'J', b'X', b'L', b' ', 13, 10, 135, 10]) {
        return None;
    }
    let mut at = 12;
    while at < b.len() {
        let size = be32(b, at)?;
        let ty = slice(b, at.checked_add(4)?, 4)?;
        let (header, len) = match size {
            0 => (8, b.len().checked_sub(at)?),
            1 => (16, usize::try_from(u64::from_be_bytes(bytes(b, at.checked_add(8)?)?)).ok()?),
            _ => (8, size as usize),
        };
        if len < header {
            return None;
        }
        let payload = slice(b, at.checked_add(header)?, len - header)?;
        if ty == b"Exif" {
            let offset = usize::try_from(be32(payload, 0)?).ok()?;
            return tiff(payload.get(4usize.checked_add(offset)?..)?);
        }
        at = at.checked_add(len)?;
    }
    None
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

    /// A CMYK JPEG carries no JFIF (it describes one- and three-component images), so its
    /// resolution is written as EXIF instead and this reader still finds it (#3084).
    #[test]
    fn a_cmyk_jpeg_keeps_its_resolution_without_jfif() {
        use vectorcraft_render::encode::jpeg::{self, ColorModel, JpegOptions};
        let o = JpegOptions { color_model: ColorModel::Cmyk, ..Default::default() };
        let jpg = jpeg::encode(&[0; 4 * 4], 2, 2, 90, Some(240.0), &o).unwrap();
        assert!(!jpg.windows(5).any(|w| w == b"JFIF\0"), "no JFIF on a CMYK file");
        assert_eq!(resolution(&jpg), Some((240.0, 240.0)));
        // An RGB file still says it in JFIF, as it always did.
        let rgb = jpeg::encode(&[0; 3 * 4], 2, 2, 90, Some(240.0), &JpegOptions::default()).unwrap();
        assert!(rgb.windows(5).any(|w| w == b"JFIF\0"));
        assert_eq!(resolution(&rgb), Some((240.0, 240.0)));
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
