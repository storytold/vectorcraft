//! Image compression and downsampling: images above the threshold resolution are resampled to the
//! target, compressed with JPEG or ZIP as their kind (colour, greyscale, monochrome) asks, and the
//! codecs the writer lacks are written as ZIP with a warning. In files before PDF 1.5, 16-bit
//! images are written at 8 bits.

use std::io::Cursor;

use craft_pdf::syntax::object::{Name, Object};
use image::{DynamicImage, ImageFormat};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Point};

use crate::*;

fn encoded(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

/// A 600 × 600 colour photo-like image.
fn colour() -> DynamicImage {
    image::RgbImage::from_fn(600, 600, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x * y) % 251) as u8])).into()
}

/// A document with image `bytes` (600 × 600 pixels) placed one inch wide: 600 ppi.
fn doc(bytes: Vec<u8>, mime: &str) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("img".into(), ImageBlob::new(mime, bytes));
    let xf = Affine::scale(72.0 / 600.0);
    let n = Node::new(
        d.alloc_id(),
        NodeKind::Image(Box::new(ImageObject {
            key: "img".into(),
            width: 600,
            height: 600,
            xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The PDF of `d` with Compression options `compression` (content streams left readable) and its
/// warnings.
fn pdf(d: &Document, compression: Value) -> (String, Vec<String>) {
    let mut settings: PdfSettings = serde_json::from_value(json!({ "compression": compression })).unwrap();
    settings.compression.compress_text = false;
    let r = export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap();
    (String::from_utf8_lossy(&r.bytes).into_owned(), r.warnings)
}

#[test]
fn images_above_the_threshold_are_resampled_to_the_target_resolution() {
    let d = doc(encoded(colour(), ImageFormat::Png), "image/png");
    for method in ["average", "subsample", "bicubic"] {
        let (text, warnings) = pdf(&d, json!({"color": {"downsample": method, "ppi": 150, "abovePpi": 225}}));
        assert!(text.contains("/Width 150") && text.contains("/Height 150"), "{method}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }
    // Below the threshold, or without downsampling, the image keeps its pixels.
    for c in [json!({"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 700}}), json!({})] {
        assert!(pdf(&d, c.clone()).0.contains("/Width 600"), "{c}");
    }
    // Greyscale images take the greyscale settings.
    let grey = doc(encoded(colour().grayscale(), ImageFormat::Png), "image/png");
    let (text, _) = pdf(&grey, json!({"color": {"downsample": "bicubic", "ppi": 150}, "gray": {"downsample": "bicubic", "ppi": 100}}));
    assert!(text.contains("/Width 100") && text.contains("/DeviceGray"), "grey at 100 ppi");
}

/// An image with clear pixels, resampled, keeps the color of its opaque pixels: the clear black
/// beside them does not darken them.
#[test]
fn resampled_images_keep_their_color_beside_clear_pixels() {
    let striped = image::RgbaImage::from_fn(600, 600, |x, _| if x % 2 == 0 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) });
    let d = doc(encoded(striped.into(), ImageFormat::Png), "image/png");
    for method in ["average", "bicubic"] {
        let mut settings: PdfSettings =
            serde_json::from_value(json!({"compression": {"color": {"downsample": method, "ppi": 150, "abovePpi": 225, "compression": "zip"}}}))
                .unwrap();
        settings.compression.compress_text = false;
        let bytes = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
        let file = craft_pdf::syntax::Pdf::new(std::sync::Arc::new(bytes)).unwrap();
        let rgb = file
            .objects()
            .into_iter()
            .find_map(|o| match o {
                Object::Stream(s)
                    if s.dict().get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image") && s.dict().contains_key(b"SMask") =>
                {
                    Some(s.decoded().unwrap().into_owned())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{method}: a color image with a soft mask"));
        assert_eq!(rgb.len(), 150 * 150 * 3, "{method}");
        for p in rgb.as_chunks::<3>().0 {
            assert!(p[0].abs_diff(200) <= 3 && p[1].abs_diff(30) <= 3 && p[2].abs_diff(30) <= 3, "{method}: {p:?}");
        }
    }
}

/// The width and height of each image XObject in `pdf`.
fn image_sizes(pdf: &[u8]) -> Vec<(u32, u32)> {
    let file = craft_pdf::syntax::Pdf::new(std::sync::Arc::new(pdf.to_vec())).unwrap();
    file.objects()
        .into_iter()
        .filter_map(|o| match o {
            Object::Stream(s) if s.dict().get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image") => {
                Some((s.dict().get::<u32>(b"Width").unwrap_or(0), s.dict().get::<u32>(b"Height").unwrap_or(0)))
            }
            _ => None,
        })
        .collect()
}

/// A mesh warp that stretches the image across more toward its right side cuts it into pieces
/// placed at different widths. With downsampling on, every piece paints one image, resampled for
/// the widest piece; another placement of the image keeps its own resampling.
#[test]
fn a_warped_image_is_written_once_for_all_its_pieces() {
    let compression = json!({"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}});
    for masked in [false, true] {
        let mut d = doc(encoded(colour(), ImageFormat::Png), "image/png");
        let id = d.layers[0].children().unwrap()[0].id;
        let n = d.node(id).unwrap().clone();
        let NodeKind::Image(im) = &n.kind else { panic!("the image") };
        // 0.5 to 1.5 times as wide from left to right, within the page.
        let stretch = |p: Point| Point::new(p.x * (0.5 + p.x / 144.0), p.y);
        *d.node_mut(id).unwrap() = vectorcraft_doc::live::mesh_warp_image(&n, im, &stretch, 4, false, masked);
        let mut settings: PdfSettings = serde_json::from_value(json!({ "compression": compression })).unwrap();
        settings.compression.compress_text = false;
        let bytes = export_with_report(&d, &PdfOptions { settings: settings.clone(), ..Default::default() }).unwrap().bytes;
        let sizes = image_sizes(&bytes);
        assert_eq!(sizes.len(), 1, "masked {masked}: one image for 32 pieces, not {sizes:?}");
        // One inch high (150 pixels at 150 ppi), and wider than one inch at its widest.
        let (w, h) = sizes[0];
        assert!(h == 150 && w > 150 && w < 600, "masked {masked}: {w} × {h}");
        assert!(String::from_utf8_lossy(&bytes).matches(" Do").count() >= 32, "masked {masked}: every piece paints the image");

        // The image placed again, two inches wide and high, is resampled for that placement.
        let plain = Node::new(d.alloc_id(), NodeKind::Image(Box::new(ImageObject { xf: Affine::scale(144.0 / 600.0), ..(**im).clone() })));
        let l = d.layers[0].id;
        d.insert(Some(l), 1, plain).unwrap();
        let bytes = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
        let mut sizes = image_sizes(&bytes);
        sizes.sort_unstable();
        assert_eq!(sizes, [(w, h), (300, 300)], "masked {masked}");
    }
}

#[test]
fn jpeg_zip_and_automatic_compression() {
    let png = doc(encoded(colour(), ImageFormat::Png), "image/png");
    let jpeg = doc(encoded(colour(), ImageFormat::Jpeg), "image/jpeg");
    let dct = |d: &Document, c: Value| pdf(d, c).0.contains("/DCTDecode");
    assert!(dct(&png, json!({"color": {"compression": "jpeg", "quality": "low"}})), "JPEG compression");
    assert!(!dct(&png, json!({"color": {"compression": "zip"}})) && pdf(&png, json!({})).0.contains("/FlateDecode"), "ZIP");
    assert!(!dct(&png, json!({})), "automatic: lossless for a PNG");
    assert!(dct(&jpeg, json!({})) && dct(&jpeg, json!({"color": {"downsample": "average", "ppi": 150}})), "automatic: a JPEG stays JPEG");
    assert!(!dct(&jpeg, json!({"color": {"compression": "zip"}})), "ZIP re-encodes a JPEG losslessly");
    // JPEG has no transparency: an image with some stays lossless.
    let mut clear = colour().to_rgba8();
    clear.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
    let clear = doc(encoded(clear.into(), ImageFormat::Png), "image/png");
    let (text, _) = pdf(&clear, json!({"color": {"compression": "jpeg"}}));
    assert!(!text.contains("/DCTDecode") && text.contains("/SMask"));
}

#[test]
fn sixteen_bit_images_are_written_at_8_bits_before_pdf_1_5() {
    let mut clear = colour().to_rgba8();
    clear.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
    let clear = DynamicImage::from(clear);
    // 16-bit PNGs of RGB, gray and RGBA pixels, and the 8-bit pixels they hold.
    let images = [
        ("RGB", DynamicImage::ImageRgb16(colour().to_rgb16()), colour()),
        ("gray", DynamicImage::ImageLuma16(colour().grayscale().to_luma16()), colour().grayscale()),
        ("RGBA", DynamicImage::ImageRgba16(clear.to_rgba16()), clear),
    ];
    for (kind, deep, pixels) in images {
        let d = doc(encoded(deep, ImageFormat::Png), "image/png");
        // PDF/X-1a and PDF/X-3 files are PDF 1.3. A PDF 1.3 file has no soft masks: the app
        // flattens an image with transparency before it is written.
        let mut cases = vec![(json!({"compatibility": "1.4"}), 8), (json!({"compatibility": "1.5"}), 16), (json!({"compatibility": "1.7"}), 16)];
        if kind != "RGBA" {
            cases.extend([(json!({"compatibility": "1.3"}), 8), (json!({"standard": "pdfX3", "compatibility": "1.3"}), 8)]);
        }
        if kind == "gray" {
            cases.push((json!({"standard": "pdfX1a", "compatibility": "1.3"}), 8));
        }
        for (case, depth) in cases {
            let mut settings: PdfSettings = serde_json::from_value(case.clone()).unwrap();
            settings.compression.compress_text = false;
            let r = export_with_report(&d, &PdfOptions { settings, created: Some(0), ..Default::default() })
                .unwrap_or_else(|e| panic!("{kind}, {case}: {e}"));
            let text = String::from_utf8_lossy(&r.bytes);
            let other = if depth == 8 { 16 } else { 8 };
            assert!(
                text.contains(&format!("/BitsPerComponent {depth}")) && !text.contains(&format!("/BitsPerComponent {other}")),
                "{kind}, {case}: {depth} bits"
            );
            assert!(!r.warnings.iter().any(|w| w.contains("decoded")), "{kind}, {case}: {:?}", r.warnings);
            if depth == 8 {
                // The image reads back with the 8-bit pixels (within 1 for the PDF/X-3 file's RGB
                // image, which is tagged with the sRGB profile).
                let back = import(&r.bytes).unwrap();
                let got = image::load_from_memory(&back.images.values().next().unwrap().bytes).unwrap().to_rgba8();
                let close = got.dimensions() == (600, 600) && got.as_raw().iter().zip(pixels.to_rgba8().as_raw()).all(|(a, b)| a.abs_diff(*b) <= 1);
                assert!(back.images.len() == 1 && close, "{kind}, {case}: the pixels");
            }
        }
    }
    // A 16-bit PNG whose pixel data can't be decoded is left out with the note.
    let mut cut = encoded(DynamicImage::ImageRgb16(colour().to_rgb16()), ImageFormat::Png);
    let at = cut.windows(4).position(|w| w == b"IDAT").unwrap();
    cut.truncate(at + 10);
    let settings = serde_json::from_value(json!({"compatibility": "1.4"})).unwrap();
    let r = export_with_report(&doc(cut, "image/png"), &PdfOptions { settings, ..Default::default() }).unwrap();
    assert_eq!(r.warnings, ["an image couldn't be decoded and is missing from the picture"]);
}

#[test]
fn codecs_the_writer_lacks_are_zip_with_a_warning() {
    let png = doc(encoded(colour(), ImageFormat::Png), "image/png");
    let (text, warnings) = pdf(&png, json!({"color": {"compression": "jpeg2000"}}));
    assert!(!text.contains("/JPXDecode") && text.contains("/FlateDecode"));
    assert!(warnings.len() == 1 && warnings[0].contains("JPEG 2000"), "{warnings:?}");
    // Black-and-white images are monochrome: their settings apply, CCITT becomes ZIP.
    let bw = image::GrayImage::from_fn(600, 600, |x, y| image::Luma([if (x / 7 + y / 5) % 2 == 0 { 0 } else { 255 }]));
    let bw = doc(encoded(bw.into(), ImageFormat::Png), "image/png");
    let (text, warnings) = pdf(&bw, json!({"mono": {"downsample": "bicubic", "ppi": 300, "abovePpi": 450, "compression": "ccittG4"}}));
    assert!(text.contains("/Width 300") && !text.contains("/CCITTFaxDecode"));
    assert!(warnings.len() == 1 && warnings[0].contains("CCITT"), "{warnings:?}");
    // The codecs of other kinds of images say nothing.
    assert!(pdf(&png, json!({"mono": {"compression": "ccittG4"}})).1.is_empty());
}

/// A PNG whose header declares `w` × `h` grey pixels and whose data holds a few: what a file can
/// declare, not what it holds.
fn declared_png(w: u32, h: u32) -> Vec<u8> {
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        [&(data.len() as u32).to_be_bytes()[..], kind, data, &crc.sum().to_be_bytes()].concat()
    };
    let ihdr = [&w.to_be_bytes()[..], &h.to_be_bytes(), &[8, 0, 0, 0, 0]].concat();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut z, &[0; 64]).unwrap();
    let idat = z.finish().unwrap();
    [&b"\x89PNG\r\n\x1a\n"[..], &chunk(b"IHDR", &ihdr), &chunk(b"IDAT", &idat), &chunk(b"IEND", &[])].concat()
}

/// Images a document holds are decoded within the decode allowance: a header declaring billions of
/// pixels is refused before they are allocated, as an opacity mask and when recoded.
#[test]
fn images_declaring_more_than_the_allowance_are_not_decoded() {
    let png = declared_png(60_000, 60_000);
    let blob = ImageBlob::new("image/png", png.clone());
    assert_eq!(crate::images::mask_alpha(&blob, 60_000, 60_000, false), None);
    assert_eq!(crate::images::mask_alpha(&blob, 2, 2, false), None, "another size is not decoded at all");
    let c: CompressionSettings = serde_json::from_value(json!({"gray": {"downsample": "bicubic", "ppi": 72, "abovePpi": 100}})).unwrap();
    let r = crate::images::recode(&png, (10.0, 10.0), &c, false, None);
    assert!(r.image.is_none(), "kept as it is");
}
