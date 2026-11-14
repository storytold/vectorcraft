//! Image compression and downsampling: images above the threshold resolution are resampled to the
//! target, compressed with JPEG or ZIP as their kind (colour, greyscale, monochrome) asks, and the
//! codecs the writer lacks are written as ZIP with a warning. In files before PDF 1.5, 16-bit
//! images are written at 8 bits.

use std::io::Cursor;

use craft_pdf::syntax::object::{Array, Name, Object, Stream};
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

/// A version 2.1 display ICC profile header for colour space `space` (`RGB ` or `GRAY`), `len`
/// bytes in all, every byte after the header different from its neighbors.
fn icc_profile(space: &[u8; 4], len: usize) -> Vec<u8> {
    let mut icc: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    icc[..4].copy_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
    icc[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(space);
    icc[36..40].copy_from_slice(b"acsp");
    icc
}

/// `img` encoded as `format` carrying ICC profile `icc`.
fn tagged(img: &DynamicImage, icc: &[u8], format: ImageFormat) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut out = Vec::new();
    match format {
        ImageFormat::Tiff => {
            let mut enc = image::codecs::tiff::TiffEncoder::new(Cursor::new(&mut out));
            enc.set_icc_profile(icc.to_vec()).unwrap();
            img.write_with_encoder(enc).unwrap();
        }
        _ => {
            let mut enc = image::codecs::png::PngEncoder::new(&mut out);
            enc.set_icc_profile(icc.to_vec()).unwrap();
            img.write_with_encoder(enc).unwrap();
        }
    }
    out
}

/// An image XObject as it is written.
#[derive(Debug, PartialEq)]
struct Written {
    /// Its colour space's name: `ICCBased` for an ICC-based one.
    space: String,
    /// The profile an ICC-based colour space holds.
    icc: Option<Vec<u8>>,
    bits: u8,
    filter: String,
    smask: bool,
}

/// Each image XObject of `pdf` other than soft masks.
fn image_spaces(pdf: &[u8]) -> Vec<Written> {
    let file = craft_pdf::syntax::Pdf::new(std::sync::Arc::new(pdf.to_vec())).unwrap();
    let images: Vec<Stream<'_>> = file
        .objects()
        .into_iter()
        .filter_map(|o| match o {
            Object::Stream(s) if s.dict().get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image") => Some(s),
            _ => None,
        })
        .collect();
    let masks: Vec<Vec<u8>> = images.iter().filter_map(|s| s.dict().get::<Stream<'_>>(b"SMask")).map(|m| m.raw_data().to_vec()).collect();
    let mut out = vec![];
    for s in images.iter().filter(|s| !masks.contains(&s.raw_data().to_vec())) {
        let d = s.dict();
        let (space, icc) = match (d.get::<Name<'_>>(b"ColorSpace"), d.get::<Array<'_>>(b"ColorSpace")) {
            (Some(n), _) => (n.as_str().to_string(), None),
            (_, Some(a)) => {
                let mut it = a.flex_iter();
                let name = it.next::<Name<'_>>().unwrap().as_str().to_string();
                (name, it.next::<Stream<'_>>().map(|s| s.decoded().unwrap().into_owned()))
            }
            _ => (String::new(), None),
        };
        let filter = d.get::<Name<'_>>(b"Filter").map(|n| n.as_str().to_string()).unwrap_or_default();
        let (bits, smask) = (d.get::<u8>(b"BitsPerComponent").unwrap_or(0), d.get::<Stream<'_>>(b"SMask").is_some());
        out.push(Written { space, icc, bits, filter, smask });
    }
    out
}

/// A gray or RGB TIFF that carries an ICC profile of its colour space is written in an ICC-based
/// colour space holding the same profile, at 16 bits a sample from PDF 1.5 on, with its alpha as
/// a soft mask; and so is a tagged TIFF or PNG that is resampled or compressed with JPEG. Pixels
/// the Output settings convert, and a profile of another colour space, are written without it.
#[test]
fn tagged_tiffs_keep_their_icc_profile() {
    let (rgb, gray) = (icc_profile(b"RGB ", 600), icc_profile(b"GRAY", 300));
    let mut clear = colour().to_rgba8();
    clear.put_pixel(0, 0, image::Rgba([10, 20, 30, 0]));
    let tiff = |img: &DynamicImage, icc: &[u8]| doc(tagged(img, icc, ImageFormat::Tiff), "image/tiff");
    let export = |d: &Document, settings: Value| {
        let mut settings: PdfSettings = serde_json::from_value(settings).unwrap();
        settings.compression.compress_text = false;
        let r = export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        image_spaces(&r.bytes)
    };
    let icc_based = |icc: &[u8], bits: u8, filter: &str, smask: bool| {
        vec![Written { space: "ICCBased".into(), icc: Some(icc.to_vec()), bits, filter: filter.into(), smask }]
    };
    let flate = "FlateDecode";
    let cases = [
        ("RGB", tiff(&colour(), &rgb), json!({}), icc_based(&rgb, 8, flate, false)),
        ("gray", tiff(&colour().grayscale(), &gray), json!({}), icc_based(&gray, 8, flate, false)),
        (
            "RGB, 16 bits",
            tiff(&DynamicImage::ImageRgb16(colour().to_rgb16()), &rgb),
            json!({"compatibility": "1.7"}),
            icc_based(&rgb, 16, flate, false),
        ),
        (
            "RGB, 16 bits, PDF 1.4",
            tiff(&DynamicImage::ImageRgb16(colour().to_rgb16()), &rgb),
            json!({"compatibility": "1.4"}),
            icc_based(&rgb, 8, flate, false),
        ),
        ("RGBA", tiff(&clear.clone().into(), &rgb), json!({}), icc_based(&rgb, 8, flate, true)),
        (
            "RGB, resampled",
            tiff(&colour(), &rgb),
            json!({"compression": {"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}}),
            icc_based(&rgb, 8, flate, false),
        ),
        (
            "gray, resampled",
            tiff(&colour().grayscale(), &gray),
            json!({"compression": {"gray": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}}),
            icc_based(&gray, 8, flate, false),
        ),
        ("RGB, JPEG", tiff(&colour(), &rgb), json!({"compression": {"color": {"compression": "jpeg"}}}), icc_based(&rgb, 8, "DCTDecode", false)),
        (
            "gray, JPEG",
            tiff(&colour().grayscale(), &gray),
            json!({"compression": {"gray": {"compression": "jpeg"}}}),
            icc_based(&gray, 8, "DCTDecode", false),
        ),
        (
            "PNG, resampled",
            doc(tagged(&colour(), &rgb, ImageFormat::Png), "image/png"),
            json!({"compression": {"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}}),
            icc_based(&rgb, 8, flate, false),
        ),
    ];
    for (what, d, settings, want) in cases {
        assert_eq!(export(&d, settings), want, "{what}");
    }
    // A gray image in an RGB profile stays RGB to keep it, resampled too.
    let neutral = tiff(&DynamicImage::ImageRgb8(colour().grayscale().to_rgb8()), &rgb);
    for settings in [json!({}), json!({"compression": {"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}})] {
        assert_eq!(export(&neutral, settings.clone()), icc_based(&rgb, 8, flate, false), "neutral RGB, {settings}");
    }
    // A profile of another colour space is left out.
    let spaces = export(&tiff(&colour(), &gray), json!({}));
    assert!(spaces.len() == 1 && spaces[0].space == "DeviceRGB" && spaces[0].icc.is_none(), "RGB in a gray profile: {spaces:?}");
    // Converted pixels are in the destination's colours, not the image's profile.
    for destination in [vectorcraft_color::cms::DISPLAY_P3, vectorcraft_color::cms::GENERIC_CMYK] {
        let spaces = export(&tiff(&colour(), &rgb), json!({"output": {"conversion": "destination", "destination": destination}}));
        assert!(spaces.len() == 1 && spaces[0].icc.as_deref() != Some(rgb.as_slice()), "converted to {destination}: {spaces:?}");
    }
}
