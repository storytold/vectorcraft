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

/// An image changed by Perspective Distort is written as the pieces of its mesh, placed so that
/// its pixel rectangle's corners land on its corners: read back, each corner of the quad is a
/// corner of one of the pieces' images.
#[test]
fn a_distorted_image_is_written_with_its_corners_on_its_quad() {
    let mut d = doc(encoded(colour(), ImageFormat::Png), "image/png");
    let id = d.layers[0].children().unwrap()[0].id;
    let q = [(40.0, 10.0), (60.0, 10.0), (90.0, 90.0), (10.0, 90.0)].map(|(x, y)| Point::new(x, y));
    d.node_mut(id).unwrap().set_image_corners(q).unwrap();
    let bytes = export_with_report(&d, &PdfOptions::default()).unwrap().bytes;
    let back = crate::import(&bytes).unwrap();
    let (mut seen, mut pieces) = ([false; 4], 0);
    back.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            pieces += 1;
            let (w, h) = (f64::from(im.width), f64::from(im.height));
            for p in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
                let at = im.xf * Point::new(p.0, p.1);
                for (i, c) in q.iter().enumerate() {
                    seen[i] |= at.distance(*c) < 0.01;
                }
            }
        }
    });
    assert!(pieces > 2, "{pieces}");
    assert_eq!(seen, [true; 4]);
    // The pieces are clipped by the quad: a clipping path with its bounds and its area.
    let mut quad_clips = 0;
    back.walk(|n| {
        if let NodeKind::Group { children, clip: true } = &n.kind
            && let Some(p) = children.first().and_then(|c| c.path_data())
            && let Some(b) = p.bounds()
        {
            let area: f64 = p.subpaths.iter().map(|s| s.area()).sum::<f64>().abs();
            if [b.x0 - 10.0, b.y0 - 10.0, b.x1 - 90.0, b.y1 - 90.0].iter().all(|v| v.abs() < 0.01) && (area - 4000.0).abs() < 1.0 {
                quad_clips += 1;
            }
        }
    });
    assert!(quad_clips >= 1, "no clipping path on the quad");
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
    let r = crate::images::recode(&png, (10.0, 10.0), &c, false, None, crate::images::SourceProfiles::Keep, None);
    assert!(r.image.is_none(), "kept as it is");
}

/// When the Output settings convert colors, a color image beyond the decode allowance can't be
/// kept as it is: it is unconverted, and so is one whose header can't be read. A color image
/// whose pixel data can't be read (tagged with an ICC profile it is converted from or not) can't
/// be written in its own colors either, and is kept as it is, as is a grey one, whose pixels
/// aren't converted, and any image without a conversion.
#[test]
fn a_color_image_beyond_the_allowance_for_a_conversion_is_unconverted() {
    use vectorcraft_color::cms::{DISPLAY_P3, GENERIC_CMYK, icc_bytes};
    let d = doc(encoded(colour(), ImageFormat::Png), "image/png");
    let settings: PdfSettings = serde_json::from_value(json!({"output": {"conversion": "destination", "destination": GENERIC_CMYK}})).unwrap();
    let out = crate::output::ColorOut::new(&d, &settings).unwrap();
    assert!(out.pixels().is_some());
    let then = out.converts_tagged_pixels().then_some(&out);
    let grey = out.converts_grey_images();
    let to_srgb = out.image_srgb_intent().map(|intent| crate::images::ToSrgb { intent, sixteen_bit: false, then, grey });
    assert!(to_srgb.is_some());
    let c: CompressionSettings = serde_json::from_value(json!({})).unwrap();
    let half = |mut bytes: Vec<u8>| {
        bytes.truncate(bytes.len() / 2);
        bytes
    };
    let cut = |img: DynamicImage| half(encoded(img, ImageFormat::Png));
    let mut p3 = icc_bytes(DISPLAY_P3).unwrap().to_vec();
    p3[8..12].copy_from_slice(&[4, 0x20, 0, 0]);
    // `declared_png`'s header declaring RGB pixels (color type 2), with the IHDR chunk's CRC
    // written again.
    let mut rgb_declared = declared_png(60_000, 60_000);
    rgb_declared[25] = 2;
    let mut crc = flate2::Crc::new();
    crc.update(&rgb_declared[12..29]);
    rgb_declared[29..33].copy_from_slice(&crc.sum().to_be_bytes());
    let cases = [
        ("RGB declaring more than the allowance", rgb_declared, true),
        ("RGB", cut(colour()), false),
        ("tagged RGB", half(tagged(&colour(), &p3, ImageFormat::Png)), false),
        ("unreadable", vec![1, 2, 3], true),
        ("grey", cut(colour().grayscale()), false),
        ("grey declaring more than the allowance", declared_png(60_000, 60_000), false),
    ];
    for (what, bytes, unconverted) in cases {
        let r = crate::images::recode(&bytes, (72.0, 72.0), &c, false, out.pixels(), crate::images::SourceProfiles::Keep, to_srgb);
        assert!(r.image.is_none(), "{what}");
        assert_eq!(r.unconverted, unconverted, "{what}");
        assert!(
            !crate::images::recode(&bytes, (72.0, 72.0), &c, false, None, crate::images::SourceProfiles::Keep, None).unconverted,
            "{what}, no conversion"
        );
    }
}

/// A version 2.1 display ICC profile for colour space `space` (`RGB ` or `GRAY`), `len` bytes in
/// all, with an empty tag table, every byte after the tag count different from its neighbors.
fn icc_profile(space: &[u8; 4], len: usize) -> Vec<u8> {
    let mut icc: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    icc[..4].copy_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
    icc[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(space);
    icc[36..40].copy_from_slice(b"acsp");
    icc[128..132].copy_from_slice(&0u32.to_be_bytes());
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
        ImageFormat::Jpeg => {
            let mut enc = image::codecs::jpeg::JpegEncoder::new(&mut out);
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

/// The decoded samples of each image XObject of `pdf` other than soft masks.
fn image_samples(pdf: &[u8]) -> Vec<Vec<u8>> {
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
    images.iter().filter(|s| !masks.contains(&s.raw_data().to_vec())).map(|s| s.decoded().unwrap().into_owned()).collect()
}

/// The raw (still encoded) data of each image XObject of `pdf`.
fn image_data(pdf: &[u8]) -> Vec<Vec<u8>> {
    let file = craft_pdf::syntax::Pdf::new(std::sync::Arc::new(pdf.to_vec())).unwrap();
    file.objects()
        .into_iter()
        .filter_map(|o| match o {
            Object::Stream(s) if s.dict().get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image") => Some(s.raw_data().to_vec()),
            _ => None,
        })
        .collect()
}

/// A PNG or JPEG embedded as it is whose gray or RGB profile is malformed is written without it,
/// the JPEG with its JPEG data unchanged: a profile whose header declares another size, one without
/// the `acsp` signature, one whose tag table runs past its end, and one cut short after 64 bytes.
#[test]
fn a_malformed_profile_is_left_out_of_a_png_or_jpeg_kept_as_it_is() {
    for (space, img, device) in [(b"RGB ", colour(), "DeviceRGB"), (b"GRAY", colour().grayscale(), "DeviceGray")] {
        let mut malformed = [icc_profile(space, 600), icc_profile(space, 600), icc_profile(space, 600), icc_profile(space, 600)];
        malformed[0][..4].copy_from_slice(&601u32.to_be_bytes());
        malformed[1][36..40].copy_from_slice(b"xxxx");
        malformed[2][128..132].copy_from_slice(&100u32.to_be_bytes());
        malformed[3].truncate(64);
        malformed[3][..4].copy_from_slice(&64u32.to_be_bytes());
        for (i, icc) in malformed.iter().enumerate() {
            for (format, mime, filter) in [(ImageFormat::Png, "image/png", "FlateDecode"), (ImageFormat::Jpeg, "image/jpeg", "DCTDecode")] {
                let what = format!("{device}, {format:?}, malformed profile {i}");
                let bytes = tagged(&img, icc, format);
                let mut settings = PdfSettings::default();
                settings.compression.compress_text = false;
                let pdf = export_with_report(&doc(bytes.clone(), mime), &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
                let spaces = image_spaces(&pdf);
                let plain = spaces.len() == 1 && spaces[0].space == device && spaces[0].icc.is_none() && spaces[0].filter == filter;
                assert!(plain, "{what}: {spaces:?}");
                if format == ImageFormat::Jpeg {
                    assert!(image_data(&pdf) == [bytes], "{what}: the JPEG data unchanged");
                }
            }
        }
    }
}

/// In PDF/X-1a a tagged gray JPEG kept as JPEG is written with its JPEG data, without its profile.
#[test]
fn pdf_x1a_keeps_a_tagged_gray_jpeg_as_jpeg() {
    let jpeg = tagged(&colour().grayscale(), &icc_profile(b"GRAY", 300), ImageFormat::Jpeg);
    let d = doc(jpeg.clone(), "image/jpeg");
    for compression in [json!({}), json!({"gray": {"compression": "jpeg"}})] {
        let mut settings: PdfSettings =
            serde_json::from_value(json!({"standard": "pdfX1a", "compatibility": "1.3", "compression": compression.clone()})).unwrap();
        settings.compression.compress_text = false;
        let bytes = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
        crate::pdfx::verify(&bytes, Standard::PdfX1a).unwrap_or_else(|e| panic!("{compression}: {e}"));
        let spaces = image_spaces(&bytes);
        let dct = spaces.len() == 1 && spaces[0].space == "DeviceGray" && spaces[0].icc.is_none() && spaces[0].filter == "DCTDecode";
        assert!(dct, "{compression}: {spaces:?}");
        assert!(image_data(&bytes) == [jpeg.clone()], "{compression}: the JPEG data unchanged");
    }
}

/// With Don't Include Profiles set, a gray or RGB image is written without the ICC profile it
/// carries: a PNG, JPEG or TIFF embedded as it is, resampled or compressed with JPEG, with alpha and
/// at 16 bits. A JPEG kept as it is loses its `ICC_PROFILE` segments, its compressed samples
/// unchanged. Left unset, or `null`, the setting keeps the profiles. PDF/X-4 files keep them
/// whatever the setting says.
#[test]
fn a_set_dont_include_profiles_removes_image_profiles() {
    let (rgb, gray) = (icc_profile(b"RGB ", 600), icc_profile(b"GRAY", 300));
    let mut clear = colour().to_rgba8();
    clear.put_pixel(0, 0, image::Rgba([10, 20, 30, 0]));
    let deep = DynamicImage::ImageRgb16(colour().to_rgb16());
    let export = |d: &Document, settings: Value| {
        let mut settings: PdfSettings = serde_json::from_value(settings).unwrap();
        settings.compression.compress_text = false;
        export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap()
    };
    let resampled =
        json!({"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}, "gray": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}});
    let jpeg = json!({"color": {"compression": "jpeg"}, "gray": {"compression": "jpeg"}});
    let cases = [
        ("RGB PNG", tagged(&colour(), &rgb, ImageFormat::Png), "image/png", json!({}), "DeviceRGB"),
        ("gray PNG", tagged(&colour().grayscale(), &gray, ImageFormat::Png), "image/png", json!({}), "DeviceGray"),
        ("RGB JPEG", tagged(&colour(), &rgb, ImageFormat::Jpeg), "image/jpeg", json!({}), "DeviceRGB"),
        ("gray JPEG", tagged(&colour().grayscale(), &gray, ImageFormat::Jpeg), "image/jpeg", json!({}), "DeviceGray"),
        ("RGB TIFF", tagged(&colour(), &rgb, ImageFormat::Tiff), "image/tiff", json!({}), "DeviceRGB"),
        ("gray TIFF", tagged(&colour().grayscale(), &gray, ImageFormat::Tiff), "image/tiff", json!({}), "DeviceGray"),
        ("RGBA TIFF", tagged(&clear.into(), &rgb, ImageFormat::Tiff), "image/tiff", json!({}), "DeviceRGB"),
        ("16-bit TIFF", tagged(&deep, &rgb, ImageFormat::Tiff), "image/tiff", json!({}), "DeviceRGB"),
        ("RGB PNG, resampled", tagged(&colour(), &rgb, ImageFormat::Png), "image/png", resampled.clone(), "DeviceRGB"),
        ("gray JPEG, resampled", tagged(&colour().grayscale(), &gray, ImageFormat::Jpeg), "image/jpeg", resampled, "DeviceGray"),
        ("RGB TIFF as JPEG", tagged(&colour(), &rgb, ImageFormat::Tiff), "image/tiff", jpeg.clone(), "DeviceRGB"),
        ("gray TIFF as JPEG", tagged(&colour().grayscale(), &gray, ImageFormat::Tiff), "image/tiff", jpeg, "DeviceGray"),
    ];
    for (what, bytes, mime, compression, device) in cases {
        let d = doc(bytes.clone(), mime);
        // Left unset, or null, the image keeps its profile.
        for profiles in [json!({}), json!({"profiles": null})] {
            let r = export(&d, json!({"compression": compression.clone(), "output": profiles}));
            let spaces = image_spaces(&r.bytes);
            assert!(spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.is_some(), "{what}, {profiles}: {spaces:?}");
        }
        let none = export(&d, json!({"compression": compression.clone(), "output": {"profiles": "none"}}));
        assert!(none.warnings.is_empty(), "{what}: {:?}", none.warnings);
        let spaces = image_spaces(&none.bytes);
        assert!(spaces.len() == 1 && spaces[0].space == device && spaces[0].icc.is_none(), "{what}: {spaces:?}");
        let data = image_data(&none.bytes);
        assert!(data.iter().all(|d| !d.windows(12).any(|w| w == b"ICC_PROFILE\0")), "{what}: no ICC_PROFILE segment is left");
        if mime == "image/jpeg" && compression == json!({}) {
            // The JPEG data without its profile segments: the image decoder reads the same pixels
            // from it, and no profile.
            assert!(data.len() == 1 && data[0].len() < bytes.len(), "{what}: the JPEG data kept");
            let decode = |b: &[u8]| image::load_from_memory_with_format(b, ImageFormat::Jpeg).unwrap();
            assert!(decode(&data[0]) == decode(&bytes), "{what}: the same pixels");
            assert!(crate::images::source_icc(&data[0]).is_none() && crate::images::source_icc(&bytes).is_some(), "{what}");
        }
        // PDF/X-4 tags every colour: the image keeps its profile.
        let x4 = export(&d, json!({"standard": "pdfX4", "compatibility": "1.6", "compression": compression, "output": {"profiles": "none"}}));
        let spaces = image_spaces(&x4.bytes);
        assert!(spaces.len() == 1 && spaces[0].space == "ICCBased", "{what}, PDF/X-4: {spaces:?}");
    }
    // A JPEG whose `ICC_PROFILE` segments don't make up a profile (the first of two, the second
    // missing) loses them too.
    let plain = encoded(colour(), ImageFormat::Jpeg);
    let mut part = vec![0xFF, 0xD8, 0xFF, 0xE2, 0, 30];
    part.extend_from_slice(b"ICC_PROFILE\0");
    part.extend_from_slice(&[1, 2]);
    part.extend_from_slice(&rgb[..14]);
    part.extend_from_slice(&plain[2..]);
    assert!(crate::images::source_icc(&part).is_none(), "the decoder reads no profile");
    let none = export(&doc(part, "image/jpeg"), json!({"output": {"profiles": "none"}}));
    let data = image_data(&none.bytes);
    assert!(data == [plain], "an unreadable profile: the JPEG data without its segments");
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
    // 16-bit samples are written big-endian from PDF 1.5 on, and reduced to 8 bits before it.
    // Their high and low bytes differ, so a swapped or truncated stream does not match.
    let deep = DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(600, 600, |x, y| {
        image::Rgb([((x * 109 + y) % 65_521) as u16, ((y * 257 + 3) ^ 0x5a) as u16, ((x * y) % 65_521) as u16])
    }));
    let sixteen: Vec<u8> = deep.to_rgb16().iter().flat_map(|v| v.to_be_bytes()).collect();
    for (compatibility, want) in [("1.7", sixteen), ("1.4", deep.to_rgb8().into_raw())] {
        let mut settings: PdfSettings = serde_json::from_value(json!({ "compatibility": compatibility })).unwrap();
        settings.compression.compress_text = false;
        let r = export_with_report(&tiff(&deep, &rgb), &PdfOptions { settings, ..Default::default() }).unwrap();
        assert!(image_samples(&r.bytes) == [want], "16-bit samples, PDF {compatibility}");
    }
    // A gray image in an RGB profile stays RGB to keep it, resampled too.
    let neutral = tiff(&DynamicImage::ImageRgb8(colour().grayscale().to_rgb8()), &rgb);
    for settings in [json!({}), json!({"compression": {"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}})] {
        assert_eq!(export(&neutral, settings.clone()), icc_based(&rgb, 8, flate, false), "neutral RGB, {settings}");
    }
    // A profile of another colour space is left out.
    let spaces = export(&tiff(&colour(), &gray), json!({}));
    assert!(spaces.len() == 1 && spaces[0].space == "DeviceRGB" && spaces[0].icc.is_none(), "RGB in a gray profile: {spaces:?}");
    // A malformed profile is left out, the image kept as it is, resampled or compressed with JPEG:
    // one whose header declares another size, one without the `acsp` signature, and one whose tag
    // table runs past its end.
    let mut malformed = [rgb.clone(), rgb.clone(), rgb.clone()];
    malformed[0][..4].copy_from_slice(&601u32.to_be_bytes());
    malformed[1][36..40].copy_from_slice(b"xxxx");
    malformed[2][128..132].copy_from_slice(&100u32.to_be_bytes());
    let compressions =
        [json!({}), json!({"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}), json!({"color": {"compression": "jpeg"}})];
    for (i, icc) in malformed.iter().enumerate() {
        for compression in &compressions {
            let spaces = export(&tiff(&colour(), icc), json!({ "compression": compression }));
            assert!(
                spaces.len() == 1 && spaces[0].space == "DeviceRGB" && spaces[0].icc.is_none(),
                "malformed profile {i}, {compression}: {spaces:?}"
            );
        }
    }
    // Converted pixels are in the destination's colours, not the image's profile, which the CMS
    // can't read (it has no tags): they are converted as sRGB ones, with a warning.
    for destination in [vectorcraft_color::cms::DISPLAY_P3, vectorcraft_color::cms::GENERIC_CMYK] {
        let mut settings: PdfSettings = serde_json::from_value(json!({"output": {"conversion": "destination", "destination": destination}})).unwrap();
        settings.compression.compress_text = false;
        let r = export_with_report(&tiff(&colour(), &rgb), &PdfOptions { settings, ..Default::default() }).unwrap();
        assert!(r.warnings.len() == 1 && r.warnings[0].contains("converted to the destination as sRGB"), "{destination}: {:?}", r.warnings);
        let spaces = image_spaces(&r.bytes);
        assert!(spaces.len() == 1 && spaces[0].icc.as_deref() != Some(rgb.as_slice()), "converted to {destination}: {spaces:?}");
    }
    // PDF/X-1a allows no ICC-based color spaces: a tagged gray TIFF (8 or 16 bits), PNG or JPEG is
    // written in DeviceGray, kept as it is, resampled or compressed with JPEG, and the file passes
    // the PDF/X-1a check.
    let compressions = [json!({}), json!({"gray": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}), json!({"gray": {"compression": "jpeg"}})];
    let tiff16 = tiff(&DynamicImage::ImageLuma16(colour().to_luma16()), &gray);
    let png = doc(tagged(&colour().grayscale(), &gray, ImageFormat::Png), "image/png");
    let jpeg = doc(tagged(&colour().grayscale(), &gray, ImageFormat::Jpeg), "image/jpeg");
    for (format, d) in [("TIFF", tiff(&colour().grayscale(), &gray)), ("TIFF, 16 bits", tiff16), ("PNG", png), ("JPEG", jpeg)] {
        for compression in &compressions {
            let mut settings: PdfSettings =
                serde_json::from_value(json!({"standard": "pdfX1a", "compatibility": "1.3", "compression": compression.clone()})).unwrap();
            settings.compression.compress_text = false;
            let r = export_with_report(&d, &PdfOptions { settings, ..Default::default() });
            let bytes = r.unwrap_or_else(|e| panic!("PDF/X-1a, {format}, {compression}: {e}")).bytes;
            crate::pdfx::verify(&bytes, Standard::PdfX1a).unwrap_or_else(|e| panic!("PDF/X-1a, {format}, {compression}: {e}"));
            let spaces = image_spaces(&bytes);
            let gray = spaces.len() == 1 && spaces[0].space == "DeviceGray" && spaces[0].icc.is_none();
            assert!(gray, "PDF/X-1a, {format}, {compression}: {spaces:?}");
        }
    }
}

/// A gray ICC profile (version 2.1, display class) with a D50 white point whose tone curve is `trc`:
/// one entry, a gamma (u8Fixed8), or two, the ends of a table. 192 bytes.
fn gray_profile(trc: &[u16]) -> Vec<u8> {
    let mut icc = vec![0u8; 192];
    icc[..4].copy_from_slice(&192u32.to_be_bytes());
    icc[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(b"GRAY");
    icc[20..24].copy_from_slice(b"XYZ ");
    icc[36..40].copy_from_slice(b"acsp");
    let d50 = [0x0000_F6D6u32, 0x0001_0000, 0x0000_D32D];
    for (i, v) in d50.iter().enumerate() {
        icc[68 + i * 4..72 + i * 4].copy_from_slice(&v.to_be_bytes());
    }
    icc[128..132].copy_from_slice(&2u32.to_be_bytes());
    let curve_len = 12 + 2 * trc.len() as u32;
    for (i, (sig, at, len)) in [(b"kTRC", 156u32, curve_len), (b"wtpt", 172, 20)].into_iter().enumerate() {
        let e = 132 + i * 12;
        icc[e..e + 4].copy_from_slice(sig);
        icc[e + 4..e + 8].copy_from_slice(&at.to_be_bytes());
        icc[e + 8..e + 12].copy_from_slice(&len.to_be_bytes());
    }
    icc[156..160].copy_from_slice(b"curv");
    icc[164..168].copy_from_slice(&(trc.len() as u32).to_be_bytes());
    for (i, v) in trc.iter().enumerate() {
        icc[168 + i * 2..170 + i * 2].copy_from_slice(&v.to_be_bytes());
    }
    icc[172..176].copy_from_slice(b"XYZ ");
    for (i, v) in d50.iter().enumerate() {
        icc[180 + i * 4..184 + i * 4].copy_from_slice(&v.to_be_bytes());
    }
    icc
}

/// The samples of `img` converted from ICC profile `icc` to sRGB with `intent`, as an image
/// XObject holds them: RGB (or grey, the mean of the sRGB values, when `img` is grey) at 8 bits a
/// sample, or at 16 big-endian with `deep`.
fn srgb_samples(img: &DynamicImage, icc: &[u8], intent: vectorcraft_color::cms::Intent, deep: bool) -> Vec<u8> {
    let p = vectorcraft_color::cms::IccProfile::from_bytes(None, icc).unwrap();
    let (max, n) = (if deep { 65_535.0 } else { 255.0 }, if img.color().has_color() { 3 } else { 1 });
    let values: Vec<f32> = match (n, deep) {
        (3, true) => img.to_rgb16().into_raw().into_iter().map(f32::from).collect(),
        (3, false) => img.to_rgb8().into_raw().into_iter().map(f32::from).collect(),
        (_, true) => img.to_luma16().into_raw().into_iter().map(f32::from).collect(),
        _ => img.to_luma8().into_raw().into_iter().map(f32::from).collect(),
    };
    let mut out = vec![];
    for px in values.chunks(n) {
        let v: Vec<f32> = px.iter().map(|v| v / max).collect();
        let rgb = p.to_srgb(&v, intent).unwrap();
        let rgb = if n == 1 { vec![rgb.iter().sum::<f32>() / 3.0] } else { rgb.to_vec() };
        for c in rgb {
            let s = (c * max).round();
            if deep { out.extend_from_slice(&(s as u16).to_be_bytes()) } else { out.push(s as u8) }
        }
    }
    out
}

/// Do samples `a` and `b` (8 bits each, or 16 big-endian with `deep`) differ by at most `tolerance`?
fn samples_close(a: &[u8], b: &[u8], deep: bool, tolerance: i32) -> bool {
    let values = |s: &[u8]| -> Vec<i32> {
        if deep { s.chunks(2).map(|v| i32::from(u16::from_be_bytes([v[0], v[1]]))).collect() } else { s.iter().map(|v| i32::from(*v)).collect() }
    };
    let (a, b) = (values(a), values(b));
    a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| (a - b).abs() <= tolerance)
}

/// The image XObjects of `d` exported with `settings`, which add no warning, and the file.
fn export_images(d: &Document, settings: Value) -> (Vec<Written>, Vec<u8>) {
    let mut settings: PdfSettings = serde_json::from_value(settings).unwrap();
    settings.compression.compress_text = false;
    let r = export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    (image_spaces(&r.bytes), r.bytes)
}

/// With Convert to Destination and an sRGB destination, a gray or RGB image that carries an ICC
/// profile is converted from it to sRGB and written without it: an 8-bit TIFF, a 16-bit one (at
/// 16 bits a sample from PDF 1.5 on, else at 8), a PNG, a gray TIFF, a resampled image and a JPEG
/// kept as JPEG. Without conversion each keeps its profile. An image in an sRGB profile keeps its
/// samples (a JPEG its JPEG data), and a black-and-white image converted to grays stays gray.
#[test]
fn convert_to_srgb_converts_tagged_images_from_their_profile() {
    use vectorcraft_color::cms::{DISPLAY_P3, Intent, SRGB, icc_bytes};
    // Marked version 4.2: PDF 1.7 holds ICC profiles up to that version.
    let mut p3 = icc_bytes(DISPLAY_P3).unwrap().to_vec();
    p3[8..12].copy_from_slice(&[4, 0x20, 0, 0]);
    // Gamma 1.8.
    let gray = gray_profile(&[0x01CD]);
    let to_srgb = |more: Value| {
        let mut s = json!({"output": {"conversion": "destination", "destination": SRGB}});
        s.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        s
    };
    let device = |space: &str, bits: u8, filter: &str| vec![Written { space: space.into(), icc: None, bits, filter: filter.into(), smask: false }];
    let flate = "FlateDecode";
    let deep = DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(600, 600, |x, y| {
        image::Rgb([((x * 109 + y) % 65_521) as u16, ((y * 257 + 3) ^ 0x5a) as u16, ((x * y) % 65_521) as u16])
    }));
    let cases = [
        ("RGB TIFF", colour(), tagged(&colour(), &p3, ImageFormat::Tiff), "image/tiff", &p3, json!({}), device("DeviceRGB", 8, flate), false),
        ("RGB PNG", colour(), tagged(&colour(), &p3, ImageFormat::Png), "image/png", &p3, json!({}), device("DeviceRGB", 8, flate), false),
        (
            "16-bit RGB TIFF",
            deep.clone(),
            tagged(&deep, &p3, ImageFormat::Tiff),
            "image/tiff",
            &p3,
            json!({"compatibility": "1.7"}),
            device("DeviceRGB", 16, flate),
            true,
        ),
        (
            "16-bit RGB TIFF, PDF 1.4",
            deep.clone(),
            tagged(&deep, &p3, ImageFormat::Tiff),
            "image/tiff",
            &p3,
            json!({"compatibility": "1.4"}),
            device("DeviceRGB", 8, flate),
            false,
        ),
        (
            "gray TIFF",
            colour().grayscale(),
            tagged(&colour().grayscale(), &gray, ImageFormat::Tiff),
            "image/tiff",
            &gray,
            json!({}),
            device("DeviceGray", 8, flate),
            false,
        ),
    ];
    for (what, img, bytes, mime, icc, more, want, sixteen) in cases {
        let d = doc(bytes, mime);
        let (spaces, pdf) = export_images(&d, to_srgb(more.clone()));
        assert_eq!(spaces, want, "{what}");
        let samples = image_samples(&pdf);
        // 16-bit samples are converted, then reduced to 8 bits before PDF 1.5.
        let deep_source = matches!(img, DynamicImage::ImageRgb16(_));
        let want = srgb_samples(&img, icc, Intent::RelativeColorimetric, deep_source);
        let want: Vec<u8> = if deep_source && !sixteen {
            want.chunks(2).map(|v| (f32::from(u16::from_be_bytes([v[0], v[1]])) / 257.0).round() as u8).collect()
        } else {
            want
        };
        assert!(samples.len() == 1 && samples_close(&samples[0], &want, sixteen, if sixteen { 2 } else { 1 }), "{what}: converted samples");
        let unconverted: Vec<u8> = match &img {
            DynamicImage::ImageRgb16(s) if sixteen => s.iter().flat_map(|v| v.to_be_bytes()).collect(),
            s if s.color().has_color() => s.to_rgb8().into_raw(),
            s => s.to_luma8().into_raw(),
        };
        assert!(!samples_close(&samples[0], &unconverted, sixteen, 0), "{what}: the samples changed");
        // Without conversion the image keeps its profile (a version 4 profile from PDF 1.5 on).
        let mut plain = more;
        plain["compatibility"] = json!("1.7");
        let (spaces, _) = export_images(&d, plain);
        assert!(
            spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.as_deref() == Some(icc.as_slice()),
            "{what}, no conversion: {spaces:?}"
        );
    }
    // Resampled, or kept as a JPEG: converted and written without the profile.
    let resampled = to_srgb(json!({"compression": {"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 225}}}));
    let (spaces, _) = export_images(&doc(tagged(&colour(), &p3, ImageFormat::Tiff), "image/tiff"), resampled);
    assert_eq!(spaces, device("DeviceRGB", 8, flate), "resampled");
    let (spaces, _) = export_images(&doc(tagged(&colour(), &p3, ImageFormat::Jpeg), "image/jpeg"), to_srgb(json!({})));
    assert_eq!(spaces, device("DeviceRGB", 8, "DCTDecode"), "JPEG");
    // An image in an sRGB profile keeps its samples, and a JPEG its JPEG data, written without the
    // profile; so does an image whose samples the conversion leaves unchanged (black in P3).
    let srgb = icc_bytes(SRGB).unwrap().to_vec();
    let (spaces, pdf) = export_images(&doc(tagged(&colour(), &srgb, ImageFormat::Png), "image/png"), to_srgb(json!({"compatibility": "1.7"})));
    assert_eq!(spaces, device("DeviceRGB", 8, flate), "sRGB PNG");
    assert!(image_samples(&pdf) == [colour().to_rgb8().into_raw()], "sRGB PNG: the samples unchanged");
    let jpeg = tagged(&colour(), &srgb, ImageFormat::Jpeg);
    let (spaces, pdf) = export_images(&doc(jpeg.clone(), "image/jpeg"), to_srgb(json!({"compatibility": "1.7"})));
    assert_eq!(spaces, device("DeviceRGB", 8, "DCTDecode"), "sRGB JPEG");
    assert!(image_data(&pdf) == [jpeg], "sRGB JPEG: the JPEG data unchanged");
    let black = DynamicImage::ImageRgb8(image::RgbImage::new(600, 600));
    let (spaces, _) = export_images(&doc(tagged(&black, &p3, ImageFormat::Png), "image/png"), to_srgb(json!({"compatibility": "1.7"})));
    assert_eq!(spaces, device("DeviceRGB", 8, flate), "black P3 PNG");
    // A black-and-white image whose profile makes black a gray is written in those grays, not
    // thresholded back to black and white when it is resampled.
    let bilevel =
        DynamicImage::ImageLuma8(image::GrayImage::from_fn(600, 600, |x, y| image::Luma([if (x / 10 + y / 10) % 2 == 0 { 0 } else { 255 }])));
    let lifted = gray_profile(&[0x3333, 0xFFFF]);
    let resample = json!({"downsample": "bicubic", "ppi": 150, "abovePpi": 225});
    let (spaces, pdf) = export_images(
        &doc(tagged(&bilevel, &lifted, ImageFormat::Tiff), "image/tiff"),
        to_srgb(json!({"compression": {"gray": resample.clone(), "mono": resample}})),
    );
    assert_eq!(spaces, device("DeviceGray", 8, flate), "lifted black and white");
    let samples = image_samples(&pdf);
    assert!(samples.len() == 1 && samples[0].iter().all(|v| *v > 0) && samples[0].iter().any(|v| *v > 200), "lifted black and white: grays kept");
    // A well-formed profile the CMS can't read (no tags) is kept, with a warning.
    let unread = icc_profile(b"RGB ", 600);
    let mut settings: PdfSettings = serde_json::from_value(to_srgb(json!({}))).unwrap();
    settings.compression.compress_text = false;
    let r = export_with_report(&doc(tagged(&colour(), &unread, ImageFormat::Tiff), "image/tiff"), &PdfOptions { settings, ..Default::default() })
        .unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("couldn't be read")), "{:?}", r.warnings);
    let spaces = image_spaces(&r.bytes);
    assert!(spaces.len() == 1 && spaces[0].icc.as_deref() == Some(unread.as_slice()), "unread profile: {spaces:?}");
    // A tagged image whose header declares more pixels than the decoding allowance is kept as it
    // is, with a warning when it was to be converted.
    let declared = |img: &DynamicImage, icc: &[u8]| {
        let mut png = tagged(img, icc, ImageFormat::Png);
        png[16..24].copy_from_slice(&[0, 0, 0x75, 0x30, 0, 0, 0x75, 0x30]);
        let mut crc = flate2::Crc::new();
        crc.update(&png[12..29]);
        png[29..33].copy_from_slice(&crc.sum().to_be_bytes());
        png
    };
    let c: CompressionSettings = serde_json::from_value(json!({})).unwrap();
    let srgb_out = Some(crate::images::ToSrgb { intent: Intent::RelativeColorimetric, sixteen_bit: true, then: None, grey: true });
    for (what, png) in [("RGB", declared(&colour(), &p3)), ("gray", declared(&colour().grayscale(), &gray))] {
        let keep = crate::images::SourceProfiles::Keep;
        let r = crate::images::recode(&png, (100.0, 100.0), &c, false, None, keep, srgb_out);
        assert!(r.image.is_none() && r.warning.is_some_and(|w| w.contains("not converted to sRGB")), "{what}: {:?}", r.warning);
        let r = crate::images::recode(&png, (100.0, 100.0), &c, false, None, keep, None);
        assert!(r.image.is_none() && r.warning.is_none(), "{what}, no conversion: {:?}", r.warning);
        // Too large to embed as it is too: the export leaves it out, and says only that.
        let mut settings: PdfSettings = serde_json::from_value(to_srgb(json!({}))).unwrap();
        settings.compression.compress_text = false;
        let r = export_with_report(&doc(png, "image/png"), &PdfOptions { settings, ..Default::default() }).unwrap();
        assert!(r.warnings.len() == 1 && r.warnings[0].contains("missing from the picture"), "{what}, exported: {:?}", r.warnings);
    }
}

/// An RGB ICC profile (version 2.1, display class, XYZ connection space) of two lookup tables of
/// two grid points each: the colorimetric one (`A2B1`) maps device RGB linearly onto sRGB's
/// primaries adapted to D50, the perceptual one (`A2B0`) onto half of them. 436 bytes.
fn rgb_two_intents() -> Vec<u8> {
    let mut icc = vec![0u8; 436];
    icc[..4].copy_from_slice(&436u32.to_be_bytes());
    icc[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(b"RGB ");
    icc[20..24].copy_from_slice(b"XYZ ");
    icc[36..40].copy_from_slice(b"acsp");
    let d50 = [0x0000_F6D6u32, 0x0001_0000, 0x0000_D32D];
    for (i, v) in d50.iter().enumerate() {
        icc[68 + i * 4..72 + i * 4].copy_from_slice(&v.to_be_bytes());
    }
    icc[128..132].copy_from_slice(&3u32.to_be_bytes());
    for (i, (sig, at, len)) in [(b"A2B0", 168u32, 124u32), (b"A2B1", 292, 124), (b"wtpt", 416, 20)].into_iter().enumerate() {
        let e = 132 + i * 12;
        icc[e..e + 4].copy_from_slice(sig);
        icc[e + 4..e + 8].copy_from_slice(&at.to_be_bytes());
        icc[e + 8..e + 12].copy_from_slice(&len.to_be_bytes());
    }
    let primaries = [[0.4361f32, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];
    for (at, scale) in [(168usize, 0.5f32), (292, 1.0)] {
        let mut lut = vec![];
        lut.extend_from_slice(b"mft2");
        lut.extend_from_slice(&[0, 0, 0, 0, 3, 3, 2, 0]);
        // The identity matrix, as s15Fixed16 numbers.
        for v in [1u32, 0, 0, 0, 1, 0, 0, 0, 1] {
            lut.extend_from_slice(&(v << 16).to_be_bytes());
        }
        lut.extend_from_slice(&2u16.to_be_bytes());
        lut.extend_from_slice(&2u16.to_be_bytes());
        for _ in 0..3 {
            lut.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
        }
        for corner in 0..8u32 {
            let on = [corner >> 2 & 1, corner >> 1 & 1, corner & 1].map(|b| b as f32);
            let xyz = primaries.iter().zip(on).fold([0.0f32; 3], |sum, (p, o)| [sum[0] + o * p[0], sum[1] + o * p[1], sum[2] + o * p[2]]);
            for v in xyz {
                lut.extend_from_slice(&((v * scale * 32_768.0).round() as u16).to_be_bytes());
            }
        }
        for _ in 0..3 {
            lut.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
        }
        icc[at..at + 124].copy_from_slice(&lut);
    }
    icc[416..420].copy_from_slice(b"XYZ ");
    for (i, v) in d50.iter().enumerate() {
        icc[424 + i * 4..428 + i * 4].copy_from_slice(&v.to_be_bytes());
    }
    icc
}

/// The conversion of a tagged image to an sRGB destination uses the colour settings' intent: an
/// RGB profile with a perceptual table and a colorimetric one converts with the table the intent
/// picks.
#[test]
fn convert_to_srgb_converts_images_with_the_color_settings_intent() {
    use vectorcraft_color::cms::{self, ColorSettings, Intent, SRGB};
    let icc = rgb_two_intents();
    let img = colour();
    let d = doc(tagged(&img, &icc, ImageFormat::Tiff), "image/tiff");
    let settings = json!({"output": {"conversion": "destination", "destination": SRGB}});
    let mut seen = vec![];
    for intent in [Intent::RelativeColorimetric, Intent::Perceptual] {
        let active = std::sync::Arc::new(cms::Cms::new(&ColorSettings { intent, ..ColorSettings::default() }).unwrap());
        let samples = image_samples(&cms::with_active(active, || export_images(&d, settings.clone())).1);
        let want = srgb_samples(&img, &icc, intent, false);
        assert!(samples.len() == 1 && samples_close(&samples[0], &want, false, 1), "{intent:?}: converted samples");
        seen.push(samples[0].clone());
    }
    assert!(seen[0] != seen[1], "the two intents give different samples");
}

/// A 600 × 600 image of sixteen blocks of colour, saturated ones (the primaries and secondaries)
/// among them.
fn colour_blocks() -> DynamicImage {
    const LEVELS: [u8; 4] = [0, 90, 200, 255];
    image::RgbImage::from_fn(600, 600, |x, y| {
        let (i, j) = ((x / 150) as usize, (y / 150) as usize);
        image::Rgb([LEVELS[i], LEVELS[j], LEVELS[(i + 2 * j) % 4]])
    })
    .into()
}

/// The samples (8 bits each) of `img`'s pixels as `convert` gives each one's: each distinct
/// colour converts once.
fn converted_samples(img: &DynamicImage, convert: impl Fn([u8; 3]) -> Vec<u8>) -> Vec<u8> {
    let mut seen: std::collections::HashMap<[u8; 3], Vec<u8>> = std::collections::HashMap::new();
    let mut out = vec![];
    for p in img.to_rgb8().pixels() {
        out.extend_from_slice(seen.entry(p.0).or_insert_with(|| convert(p.0)));
    }
    out
}

/// The largest difference between samples `a` and `b` (8 bits each) and where it is: (difference,
/// sample index, the two samples), or the two lengths when they differ.
fn max_diff(a: &[u8], b: &[u8]) -> String {
    if a.len() != b.len() {
        return format!("lengths {} and {}", a.len(), b.len());
    }
    let worst = a.iter().zip(b).enumerate().max_by_key(|(_, (a, b))| a.abs_diff(**b));
    worst.map_or_else(String::new, |(i, (x, y))| format!("largest difference {} at sample {i}: {x} vs {y}", x.abs_diff(*y)))
}

/// With Convert to Destination and a CMYK destination, or an RGB one other than sRGB, a colour
/// image that carries an ICC profile is converted from that profile (to sRGB, then to the
/// destination) rather than from its numbers read as sRGB ones; an untagged image is converted
/// from sRGB as before. A gray image keeps its profile (gray stays gray), and an RGB profile the
/// CMS can't read leaves the numbers converted as sRGB ones, with a warning. An RGB destination
/// other than sRGB that is the document's own RGB profile leaves RGB images as they are, so the image keeps its
/// profile.
#[test]
fn convert_to_destination_converts_tagged_images_from_their_profile() {
    use vectorcraft_color::cms::{self, ColorSettings, DISPLAY_P3, GENERIC_CMYK, Intent, WIDE_GAMUT_RGB, icc_bytes};
    let intent = Intent::RelativeColorimetric;
    let active = std::sync::Arc::new(cms::Cms::new(&ColorSettings { intent, ..ColorSettings::default() }).unwrap());
    let mut p3 = icc_bytes(DISPLAY_P3).unwrap().to_vec();
    p3[8..12].copy_from_slice(&[4, 0x20, 0, 0]);
    let p3_profile = cms::IccProfile::from_bytes(None, &p3).unwrap();
    // A P3 pixel's sRGB samples.
    let from_p3 = |rgb: [u8; 3]| p3_profile.to_srgb(&rgb.map(|v| f32::from(v) / 255.0), intent).unwrap().map(|v| (v * 255.0).round() as u8);
    let img = colour_blocks();
    let tagged_png = doc(tagged(&img, &p3, ImageFormat::Png), "image/png");
    let untagged_png = doc(encoded(img.clone(), ImageFormat::Png), "image/png");
    let device = |space: &str| vec![Written { space: space.into(), icc: None, bits: 8, filter: "FlateDecode".into(), smask: false }];
    // Each destination: the space images are written in, and how an sRGB pixel's samples convert
    // to it (the tables the export builds from the destination's color settings).
    let cmyk = cms::Cms::new(&ColorSettings { intent, cmyk: GENERIC_CMYK.into(), ..ColorSettings::default() }).unwrap();
    let wide = cms::Cms::new(&ColorSettings { intent, rgb: WIDE_GAMUT_RGB.into(), ..ColorSettings::default() }).unwrap();
    let cmy = cms::ProofLut::build(|rgb| {
        let v = cmyk.srgb_to_cmyk(rgb, intent);
        [v[0], v[1], v[2]]
    });
    let black = cms::ProofLut::build(|rgb| [cmyk.srgb_to_cmyk(rgb, intent)[3], 0.0, 0.0]);
    let wide_lut = cms::ProofLut::build(|rgb| wide.srgb_to_rgb(rgb));
    let to_cmyk = |rgb: [u8; 3]| [cmy.apply8(rgb).as_slice(), &black.apply8(rgb)[..1]].concat();
    let to_wide = |rgb: [u8; 3]| wide_lut.apply8(rgb).to_vec();
    type ToDest<'a> = &'a dyn Fn([u8; 3]) -> Vec<u8>;
    let destinations: [(&str, &str, ToDest<'_>); 2] = [(GENERIC_CMYK, "DeviceCMYK", &to_cmyk), (WIDE_GAMUT_RGB, "DeviceRGB", &to_wide)];
    for (name, space, to_dest) in destinations {
        let settings = json!({"output": {"conversion": "destination", "destination": name}});
        let export = |d: &Document| cms::with_active(active.clone(), || export_images(d, settings.clone()));
        let (spaces, pdf) = export(&tagged_png);
        assert_eq!(spaces, device(space), "{name}: tagged");
        let samples = image_samples(&pdf);
        let want = converted_samples(&img, |rgb| to_dest(from_p3(rgb)));
        let as_srgb = converted_samples(&img, to_dest);
        assert!(
            samples.len() == 1 && samples_close(&samples[0], &want, false, 1),
            "{name}: converted from the profile, {}",
            max_diff(&samples[0], &want)
        );
        assert!(!samples_close(&samples[0], &as_srgb, false, 8), "{name}: not converted as sRGB");
        // An untagged image is converted from sRGB, as before.
        let (spaces, pdf) = export(&untagged_png);
        assert_eq!(spaces, device(space), "{name}: untagged");
        let samples = image_samples(&pdf);
        assert!(
            samples.len() == 1 && samples_close(&samples[0], &as_srgb, false, 1),
            "{name}: untagged converted as sRGB, {}",
            max_diff(&samples[0], &as_srgb)
        );
    }
    // A destination that is the document's own RGB profile leaves RGB images as they are: the
    // image keeps its profile, as untagged images keep their numbers.
    let own = std::sync::Arc::new(cms::Cms::new(&ColorSettings { intent, rgb: WIDE_GAMUT_RGB.into(), ..ColorSettings::default() }).unwrap());
    let settings = json!({"compatibility": "1.7", "output": {"conversion": "destination", "destination": WIDE_GAMUT_RGB}});
    let (spaces, _) = cms::with_active(own, || export_images(&tagged_png, settings));
    assert!(spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.as_deref() == Some(p3.as_slice()), "own RGB profile: {spaces:?}");
    // A gray image keeps its gray profile in a CMYK destination.
    let gray = gray_profile(&[0x01CD]);
    let settings = json!({"output": {"conversion": "destination", "destination": GENERIC_CMYK}});
    let gray_doc = doc(tagged(&colour().grayscale(), &gray, ImageFormat::Tiff), "image/tiff");
    let (spaces, _) = cms::with_active(active.clone(), || export_images(&gray_doc, settings.clone()));
    assert!(spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.as_deref() == Some(gray.as_slice()), "gray: {spaces:?}");
    // A profile the CMS can't read: converted as sRGB, with a warning.
    let unread = icc_profile(b"RGB ", 600);
    let mut s: PdfSettings = serde_json::from_value(settings).unwrap();
    s.compression.compress_text = false;
    let unread_doc = doc(tagged(&img, &unread, ImageFormat::Png), "image/png");
    let r = cms::with_active(active, || export_with_report(&unread_doc, &PdfOptions { settings: s, ..Default::default() })).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("converted to the destination as sRGB")), "{:?}", r.warnings);
    assert_eq!(image_spaces(&r.bytes), device("DeviceCMYK"), "unread profile");
    let as_srgb = converted_samples(&img, to_cmyk);
    assert!(image_samples(&r.bytes).first().is_some_and(|s| samples_close(s, &as_srgb, false, 1)), "unread profile: converted as sRGB");
}

/// With Preserve Numbers, an RGB image that carries an ICC profile is converted from that profile
/// whatever the destination, while an untagged one keeps its numbers in an RGB destination (and
/// converts from sRGB to a CMYK one, as before): to Generic CMYK, sRGB, Wide Gamut RGB, and Wide
/// Gamut RGB that is also the document's RGB profile, and in PDF/X-1a, which converts the same way.
/// A gray image keeps its profile, except in an sRGB destination, where it is converted to sRGB
/// grays. An RGB profile the CMS can't read leaves the image as it is in an RGB destination, and
/// its numbers converted as sRGB ones to a CMYK destination, with a warning.
#[test]
fn preserve_numbers_converts_tagged_images_from_their_profile() {
    use vectorcraft_color::cms::{self, ColorSettings, DISPLAY_P3, GENERIC_CMYK, Intent, SRGB, WIDE_GAMUT_RGB, icc_bytes};
    let intent = Intent::RelativeColorimetric;
    let settings_with =
        |rgb: &str| std::sync::Arc::new(cms::Cms::new(&ColorSettings { intent, rgb: rgb.into(), ..ColorSettings::default() }).unwrap());
    let (active, own) = (settings_with(SRGB), settings_with(WIDE_GAMUT_RGB));
    let mut p3 = icc_bytes(DISPLAY_P3).unwrap().to_vec();
    p3[8..12].copy_from_slice(&[4, 0x20, 0, 0]);
    let p3_profile = cms::IccProfile::from_bytes(None, &p3).unwrap();
    let from_p3 = |rgb: [u8; 3]| p3_profile.to_srgb(&rgb.map(|v| f32::from(v) / 255.0), intent).unwrap().map(|v| (v * 255.0).round() as u8);
    let img = colour_blocks();
    let tagged_png = doc(tagged(&img, &p3, ImageFormat::Png), "image/png");
    let untagged_png = doc(encoded(img.clone(), ImageFormat::Png), "image/png");
    let device = |space: &str| vec![Written { space: space.into(), icc: None, bits: 8, filter: "FlateDecode".into(), smask: false }];
    let cmyk = cms::Cms::new(&ColorSettings { intent, cmyk: GENERIC_CMYK.into(), ..ColorSettings::default() }).unwrap();
    let cmy = cms::ProofLut::build(|rgb| {
        let v = cmyk.srgb_to_cmyk(rgb, intent);
        [v[0], v[1], v[2]]
    });
    let black = cms::ProofLut::build(|rgb| [cmyk.srgb_to_cmyk(rgb, intent)[3], 0.0, 0.0]);
    let wide_lut = cms::ProofLut::build(|rgb| own.srgb_to_rgb(rgb));
    let to_cmyk = |rgb: [u8; 3]| [cmy.apply8(rgb).as_slice(), &black.apply8(rgb)[..1]].concat();
    let to_wide = |rgb: [u8; 3]| wide_lut.apply8(rgb).to_vec();
    let to_srgb = |rgb: [u8; 3]| rgb.to_vec();
    type ToDest<'a> = &'a dyn Fn([u8; 3]) -> Vec<u8>;
    // Each case: what, the color settings, the export settings, the space images are written in,
    // how an sRGB pixel converts to the destination, and whether untagged images keep their numbers.
    type Case<'a> = (&'a str, &'a std::sync::Arc<cms::Cms>, Value, &'a str, ToDest<'a>, bool);
    let cases: [Case<'_>; 5] = [
        ("CMYK", &active, json!({"output": {"conversion": "preserveNumbers", "destination": GENERIC_CMYK}}), "DeviceCMYK", &to_cmyk, false),
        ("sRGB", &active, json!({"output": {"conversion": "preserveNumbers", "destination": SRGB}}), "DeviceRGB", &to_srgb, true),
        ("Wide Gamut", &active, json!({"output": {"conversion": "preserveNumbers", "destination": WIDE_GAMUT_RGB}}), "DeviceRGB", &to_wide, true),
        ("own RGB", &own, json!({"output": {"conversion": "preserveNumbers", "destination": WIDE_GAMUT_RGB}}), "DeviceRGB", &to_wide, true),
        (
            "PDF/X-1a",
            &active,
            json!({"standard": "pdfX1a", "compatibility": "1.3", "output": {"destination": GENERIC_CMYK}}),
            "DeviceCMYK",
            &to_cmyk,
            false,
        ),
    ];
    for (what, color, settings, space, to_dest, keeps) in cases {
        let export = |d: &Document| {
            let mut s: PdfSettings = serde_json::from_value(settings.clone()).unwrap();
            s.compression.compress_text = false;
            let r = cms::with_active((*color).clone(), || export_with_report(d, &PdfOptions { settings: s, ..Default::default() })).unwrap();
            assert!(r.warnings.is_empty(), "{what}: {:?}", r.warnings);
            if what == "PDF/X-1a" {
                crate::pdfx::verify(&r.bytes, Standard::PdfX1a).unwrap_or_else(|e| panic!("{what}: {e}"));
            }
            (image_spaces(&r.bytes), image_samples(&r.bytes))
        };
        let (spaces, samples) = export(&tagged_png);
        assert_eq!(spaces, device(space), "{what}: tagged");
        let want = converted_samples(&img, |rgb| to_dest(from_p3(rgb)));
        let as_numbers = converted_samples(&img, to_dest);
        assert!(
            samples.len() == 1 && samples_close(&samples[0], &want, false, 1),
            "{what}: converted from the profile, {}",
            max_diff(&samples[0], &want)
        );
        assert!(!samples_close(&samples[0], &as_numbers, false, 8), "{what}: not converted as its numbers");
        let (spaces, samples) = export(&untagged_png);
        assert_eq!(spaces, device(space), "{what}: untagged");
        let untagged = if keeps { img.to_rgb8().into_raw() } else { as_numbers };
        assert!(samples.len() == 1 && samples_close(&samples[0], &untagged, false, 1), "{what}: untagged, {}", max_diff(&samples[0], &untagged));
    }
    // With RGB colours ICC-tagged as sRGB, the tagged image is written as its sRGB samples, in an
    // ICC-based colour space with the sRGB profile.
    let settings = json!({"output": {"conversion": "preserveNumbers", "destination": WIDE_GAMUT_RGB, "profiles": "all"}});
    let (spaces, pdf) = cms::with_active(active.clone(), || export_images(&tagged_png, settings));
    let is_srgb = |icc: &[u8]| {
        let p = cms::IccProfile::from_bytes(None, icc).unwrap();
        [[0.2, 0.5, 0.9], [0.9, 0.1, 0.4]].iter().all(|v| p.to_srgb(v, intent).unwrap().iter().zip(v).all(|(a, b)| (a - b).abs() < 0.01))
    };
    assert!(spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.as_deref().is_some_and(is_srgb), "tagged as sRGB: {spaces:?}");
    let samples = image_samples(&pdf);
    let want = converted_samples(&img, |rgb| from_p3(rgb).to_vec());
    assert!(samples.len() == 1 && samples_close(&samples[0], &want, false, 1), "tagged as sRGB, {}", max_diff(&samples[0], &want));
    // A gray image keeps its gray profile, in a CMYK and in an RGB destination, and in an RGB one
    // with RGB colours ICC-tagged as sRGB (`all` profiles, PDF/A-2b).
    let gray = gray_profile(&[0x01CD]);
    let gray_doc = doc(tagged(&colour().grayscale(), &gray, ImageFormat::Tiff), "image/tiff");
    for (what, settings) in [
        (GENERIC_CMYK, json!({"output": {"conversion": "preserveNumbers", "destination": GENERIC_CMYK}})),
        (WIDE_GAMUT_RGB, json!({"output": {"conversion": "preserveNumbers", "destination": WIDE_GAMUT_RGB}})),
        ("all profiles", json!({"output": {"conversion": "preserveNumbers", "destination": WIDE_GAMUT_RGB, "profiles": "all"}})),
        ("PDF/A-2b", json!({"standard": "pdfA2b", "output": {"conversion": "preserveNumbers", "destination": WIDE_GAMUT_RGB}})),
    ] {
        let (spaces, _) = cms::with_active(active.clone(), || export_images(&gray_doc, settings));
        assert!(spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.as_deref() == Some(gray.as_slice()), "gray, {what}: {spaces:?}");
    }
    // In an sRGB destination it is converted to sRGB grays and written without it.
    let settings = json!({"output": {"conversion": "preserveNumbers", "destination": SRGB}});
    let (spaces, _) = cms::with_active(active.clone(), || export_images(&gray_doc, settings));
    assert!(spaces.len() == 1 && spaces[0].space == "DeviceGray" && spaces[0].icc.is_none(), "gray, sRGB: {spaces:?}");
    // A profile the CMS can't read: the image keeps it and its numbers in an RGB destination, and
    // its numbers are converted as sRGB ones to a CMYK destination, with a warning each.
    let unread = icc_profile(b"RGB ", 600);
    let unread_doc = doc(tagged(&img, &unread, ImageFormat::Png), "image/png");
    let unread_export = |destination: &str| {
        let mut s: PdfSettings = serde_json::from_value(json!({"output": {"conversion": "preserveNumbers", "destination": destination}})).unwrap();
        s.compression.compress_text = false;
        cms::with_active(active.clone(), || export_with_report(&unread_doc, &PdfOptions { settings: s, ..Default::default() })).unwrap()
    };
    let r = unread_export(WIDE_GAMUT_RGB);
    assert!(r.warnings.iter().any(|w| w.contains("the image keeps it")), "{:?}", r.warnings);
    let spaces = image_spaces(&r.bytes);
    assert!(spaces.len() == 1 && spaces[0].space == "ICCBased" && spaces[0].icc.as_deref() == Some(unread.as_slice()), "unread profile: {spaces:?}");
    let r = unread_export(GENERIC_CMYK);
    assert!(r.warnings.iter().any(|w| w.contains("converted to the destination as sRGB")), "{:?}", r.warnings);
    let as_srgb = converted_samples(&img, to_cmyk);
    assert!(image_samples(&r.bytes).first().is_some_and(|s| samples_close(s, &as_srgb, false, 1)), "unread profile, CMYK: converted as sRGB");
}
