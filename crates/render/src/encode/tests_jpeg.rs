use vectorcraft_color::cms::{self, IccProfile, ProfileKind};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

use super::jpeg::{self, ColorModel, JpegOptions, Method};
use super::*;

/// The segments before the image data: (marker, payload).
fn segments(file: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = vec![];
    let mut i = 2;
    while i + 4 <= file.len() && file[i] == 0xFF {
        let marker = file[i + 1];
        let len = u16::from_be_bytes([file[i + 2], file[i + 3]]) as usize;
        out.push((marker, &file[i + 4..i + 2 + len]));
        if marker == 0xDA {
            break;
        }
        i += 2 + len;
    }
    out
}

/// The frame header (SOF0/SOF2…): (marker, component count).
fn frame(file: &[u8]) -> (u8, u8) {
    let (m, p) = segments(file).into_iter().find(|(m, _)| (0xC0..=0xC2).contains(m)).expect("SOF");
    (m, p[5])
}

/// Every APP2 ICC chunk joined in order.
fn icc(file: &[u8]) -> Option<Vec<u8>> {
    let chunks: Vec<&[u8]> =
        segments(file).into_iter().filter(|(m, p)| *m == 0xE2 && p.starts_with(b"ICC_PROFILE\0")).map(|(_, p)| &p[14..]).collect();
    (!chunks.is_empty()).then(|| chunks.concat())
}

fn noise(w: u32, h: u32, channels: usize) -> Vec<u8> {
    let mut x: u32 = 0x9e37_79b9;
    (0..w as usize * h as usize * channels)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            // Smooth ramps plus a little noise, as photos and gradients are.
            ((i / channels) as u32 % w * 255 / w) as u8 ^ (x >> 29) as u8
        })
        .collect()
}

/// JFIF describes one- and three-component images, so its APP0 segment must not head a CMYK
/// frame: the file would say two different things about its colours and a strict reader rejects
/// it. A CMYK JPEG carries the Adobe APP14 marker instead, as Adobe's own do (#3084).
#[test]
fn a_cmyk_jpeg_is_not_labelled_jfif() {
    let px = noise(16, 16, 4);
    let o = JpegOptions { color_model: ColorModel::Cmyk, ..Default::default() };
    let file = jpeg::encode(&px, 16, 16, 90, Some(300.0), &o).unwrap();
    let markers: Vec<u8> = segments(&file).into_iter().map(|(m, _)| m).collect();
    assert_eq!(frame(&file).1, 4, "four components");
    assert!(!markers.contains(&0xE0), "no APP0/JFIF: {markers:02x?}");
    assert!(markers.contains(&0xEE), "the Adobe APP14 marker names the colours: {markers:02x?}");
    assert!(icc(&file).is_some(), "the CMYK profile is still embedded");
    assert_eq!(&file[..2], &[0xFF, 0xD8], "the file still starts at SOI");
    // The resolution JFIF carried moves to EXIF, where a reader looks when there is no JFIF.
    let exif = segments(&file).into_iter().find(|(m, _)| *m == 0xE1).expect("APP1/EXIF").1;
    assert!(exif.starts_with(b"Exif\0\0"), "{:02x?}", &exif[..8.min(exif.len())]);
    // Big-endian TIFF with XResolution (282), YResolution (283) and ResolutionUnit (296).
    assert_eq!(&exif[6..10], b"MM\0*");
    // "Exif\0\0" then the TIFF header (8 bytes) and the entry count (2), so entries start at 16.
    let tags: Vec<u16> = (0..3).map(|i| u16::from_be_bytes([exif[16 + i * 12], exif[17 + i * 12]])).collect();
    assert_eq!(tags, [282, 283, 296], "{tags:?}");
    // No resolution asked for: nothing stands in for the dropped segment.
    let plain = jpeg::encode(&px, 16, 16, 90, None, &o).unwrap();
    let markers: Vec<u8> = segments(&plain).into_iter().map(|(m, _)| m).collect();
    assert!(!markers.contains(&0xE0) && !markers.contains(&0xE1), "{markers:02x?}");

    // Three- and one-component files keep JFIF, which is what it is for, and their resolution.
    for (model, channels) in [(ColorModel::Rgb, 3), (ColorModel::Gray, 1)] {
        let px = noise(16, 16, channels);
        let o = JpegOptions { color_model: model, ..Default::default() };
        let file = jpeg::encode(&px, 16, 16, 90, Some(300.0), &o).unwrap();
        let markers: Vec<u8> = segments(&file).into_iter().map(|(m, _)| m).collect();
        assert!(markers.contains(&0xE0), "{model:?} keeps JFIF: {markers:02x?}");
        assert_eq!(frame(&file).1, channels as u8);
    }
}

#[test]
fn size_falls_with_quality() {
    let px = noise(64, 48, 3);
    let size = |q| jpeg::encode(&px, 64, 48, q, None, &JpegOptions::default()).unwrap().len();
    assert!(size(10) < size(50) && size(50) < size(95), "{} {} {}", size(10), size(50), size(95));
    assert_eq!(size(0), size(1), "quality 0 is the lowest the coder has");
}

#[test]
fn methods_write_baseline_or_progressive_frames() {
    let px = noise(40, 30, 3);
    let enc = |method, scans| jpeg::encode(&px, 40, 30, 80, None, &JpegOptions { method, scans, ..Default::default() }).unwrap();
    let sos = |f: &[u8]| f.windows(2).filter(|w| w == &[0xFF, 0xDA]).count();
    let base = enc(Method::Baseline, 3);
    assert_eq!((frame(&base).0, sos(&base)), (0xC0, 1));
    let opt = enc(Method::Optimized, 3);
    assert_eq!(frame(&opt).0, 0xC0, "optimized tables are still baseline");
    assert!(opt.len() < base.len(), "optimized tables are smaller");
    let (p3, p5) = (enc(Method::Progressive, 3), enc(Method::Progressive, 5));
    assert_eq!(frame(&p3).0, 0xC2, "SOF2");
    assert!(sos(&p3) > 1 && sos(&p5) > sos(&p3), "more scans: {} {}", sos(&p3), sos(&p5));
    for f in [&base, &p3, &p5] {
        let back = image::load_from_memory(f).unwrap();
        assert_eq!((back.width(), back.height()), (40, 30));
    }
}

#[test]
fn colour_models_write_their_components_and_profiles() {
    for (model, comps, kind) in
        [(ColorModel::Rgb, 3, ProfileKind::Rgb), (ColorModel::Cmyk, 4, ProfileKind::Cmyk), (ColorModel::Gray, 1, ProfileKind::Gray)]
    {
        let o = JpegOptions { color_model: model, ..Default::default() };
        let file = jpeg::encode(&noise(16, 8, model.channels()), 16, 8, 90, Some(150.0), &o).unwrap();
        assert_eq!(frame(&file).1, comps, "{model:?}");
        let p = IccProfile::from_bytes(None, &icc(&file).expect("APP2 ICC")).unwrap();
        assert_eq!(p.kind, kind, "{model:?}");
        // JFIF describes one- and three-component images, so a CMYK file says its resolution in
        // EXIF instead (see `a_cmyk_jpeg_is_not_labelled_jfif`).
        if model == ColorModel::Cmyk {
            let exif = segments(&file).into_iter().find(|(m, _)| *m == 0xE1).expect("APP1/EXIF").1;
            assert!(exif.starts_with(b"Exif\0\0"));
            // "Exif\0\0" (6) + TIFF header (8) + count (2) + three entries (36) + next IFD (4).
            assert_eq!(&exif[56..64], &[0, 0, 0, 150, 0, 0, 0, 1], "150 dots per inch, as a rational");
        } else {
            let app0 = segments(&file).into_iter().find(|(m, _)| *m == 0xE0).unwrap().1;
            assert_eq!(&app0[..5], b"JFIF\0");
            assert_eq!(app0[7..12], [1, 0, 150, 0, 150], "dots per inch");
        }
        let none = jpeg::encode(&noise(16, 8, model.channels()), 16, 8, 90, None, &JpegOptions { embed_icc: false, ..o }).unwrap();
        assert!(icc(&none).is_none());
    }
    // The CMYK file carries the working CMYK space's profile.
    let o = JpegOptions { color_model: ColorModel::Cmyk, ..Default::default() };
    let file = jpeg::encode(&[0; 4], 1, 1, 90, None, &o).unwrap();
    assert_eq!(&icc(&file).unwrap()[..], &cms::icc_bytes(&cms::active_settings().cmyk).unwrap()[..]);
    assert!(jpeg::encode(&[0; 3], 70_000, 1, 90, None, &JpegOptions::default()).is_err(), "too wide");
    assert!(jpeg::encode(&[0; 2], 1, 1, 90, None, &JpegOptions::default()).is_err(), "short buffer");
}

#[test]
fn model_and_method_ids_round_trip() {
    for m in ColorModel::ALL {
        assert_eq!(ColorModel::from_id(m.id()), Some(m));
        assert_eq!(ColorModel::from_id(m.label()), Some(m));
    }
    assert_eq!(ColorModel::from_id("grey"), Some(ColorModel::Gray));
    for m in Method::ALL {
        assert_eq!(Method::from_id(m.id()), Some(m));
    }
    assert_eq!(Method::from_id("bogus"), None);
}

/// A 20×10 pt document: a black-ink rectangle on the left half, an RGB red one on the right.
fn ink_doc() -> Document {
    let mut d = Document::new(20.0, 10.0);
    let l = d.layers[0].id;
    let black = Paint::solid(Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 1.0 });
    let red = Paint::solid(Color::Rgb { r: 1.0, g: 0.0, b: 0.0 });
    for (rect, paint) in [(Rect::new(0.0, 0.0, 10.0, 10.0), black), (Rect::new(10.0, 0.0, 20.0, 10.0), red)] {
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::rectangle(rect), Appearance::basic(paint, Paint::None, 0.0))).unwrap();
    }
    d
}

#[test]
fn cmyk_exports_keep_the_inks_of_cmyk_colours() {
    let d = ink_doc();
    let o = RasterExportOptions { jpeg: JpegOptions { color_model: ColorModel::Cmyk, ..Default::default() }, ..Default::default() };
    let inks = Renderer::new().render_region_inks(&d, d.artboards[0].rect, 1.0, &o.render_options(RasterFormat::Jpeg));
    assert_eq!(inks.len(), 20 * 10 * 4);
    let at = |x: usize, y: usize| &inks[(y * 20 + x) * 4..][..4];
    assert_eq!(at(3, 5), [0, 0, 0, 255], "black ink alone, not a rich black");
    let red = at(15, 5);
    assert!(red[1] > 200 && red[2] > 200 && red[0] < 40, "RGB red separates to magenta and yellow: {red:?}");
    let file = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Jpeg, &o).unwrap();
    assert_eq!(frame(&file).1, 4, "4 components");
    let back = image::load_from_memory(&file).unwrap().to_rgb8();
    assert!(back.get_pixel(3, 5).0.iter().all(|v| *v < 60), "black reads back dark: {:?}", back.get_pixel(3, 5));
    let r = back.get_pixel(15, 5).0;
    assert!(r[0] > 180 && r[1] < 90, "red reads back red: {r:?}");
}

#[test]
fn gray_exports_keep_the_lightness() {
    let d = ink_doc();
    let o = RasterExportOptions { jpeg: JpegOptions { color_model: ColorModel::Gray, ..Default::default() }, ..Default::default() };
    let file = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Jpeg, &o).unwrap();
    assert_eq!(frame(&file).1, 1);
    let back = image::load_from_memory(&file).unwrap().to_luma8();
    let red = back.get_pixel(15, 5)[0];
    // sRGB red's relative luminance 0.2126, encoded: about 127.
    assert!((115..=140).contains(&red), "{red}");
}
