//! Grayscale and 1-bit images colorized by a fill color are written in that color, a spot color
//! as its ink, with the image's coverage as the soft mask.

use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Name, Object, Stream};
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{ColorMode, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Affine;

use crate::*;

/// A page with a 2 × 1 gray image (black, white), colorized with `colorize`.
fn doc(mode: ColorMode, colorize: Option<Paint>) -> Document {
    let mut d = Document::new_with_mode(40.0, 20.0, mode);
    d.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), global: true, spot: true });
    let gray = image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap();
    let mut png = vec![];
    gray.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("pic".into(), ImageBlob::png(png));
    let im = ImageObject { key: "pic".into(), width: 2, height: 1, xf: Affine::scale(20.0), link: None, placement: Default::default(), pixels: None };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.extras_mut().colorize = colorize.map(Box::new);
    d.insert(d.default_layer(), 0, n).unwrap();
    d
}

/// Each image XObject of `pdf`: its color space's name (the first of an array) with the names
/// in the array's second item, and whether it has a soft mask.
fn images(pdf: &[u8]) -> Vec<(String, Vec<String>, bool)> {
    let file = Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let mut out = vec![];
    for o in file.objects() {
        let Object::Stream(s) = o else { continue };
        let d = s.dict();
        if d.get::<Name<'_>>(b"Subtype").is_none_or(|n| n.as_ref() != b"Image") || d.get::<bool>(b"ImageMask") == Some(true) {
            continue;
        }
        let (space, names) = match (d.get::<Name<'_>>(b"ColorSpace"), d.get::<Array<'_>>(b"ColorSpace")) {
            (Some(n), _) => (n.as_str().to_string(), vec![]),
            (_, Some(a)) => {
                let mut it = a.flex_iter();
                let name = it.next::<Name<'_>>().unwrap().as_str().to_string();
                let names = it.next::<Array<'_>>().map(|a| a.iter::<Name<'_>>().map(|n| n.as_str().to_string()).collect()).unwrap_or_default();
                (name, names)
            }
            _ => continue,
        };
        out.push((space, names, d.get::<Stream<'_>>(b"SMask").is_some()));
    }
    out
}

fn pdf(d: &Document) -> Vec<u8> {
    export_with_report(d, &PdfOptions { created: Some(1_700_000_000), ..PdfOptions::uncompressed() }).unwrap().bytes
}

#[test]
fn a_spot_color_writes_the_image_in_its_ink() {
    let spot = Paint::Solid { color: Color::cmyk(0.0, 0.5, 0.5, 0.0), swatch: Some("Ink".into()), tint: 0.5 };
    let out = pdf(&doc(ColorMode::Cmyk, Some(spot)));
    let found = images(&out);
    assert!(found.contains(&("DeviceN".into(), vec!["Ink".into()], true)), "{found:?}");
    assert!(String::from_utf8_lossy(&out).contains("/Separation/Ink"), "the ink's Separation space");
}

#[test]
fn a_process_color_writes_the_image_in_that_color_with_a_soft_mask() {
    let out = pdf(&doc(ColorMode::Cmyk, Some(Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)))));
    // The image and its soft mask, a gray image.
    assert_eq!(images(&out), [("DeviceGray".into(), vec![], false), ("DeviceCMYK".into(), vec![], true)]);
    // Uncolorized: the gray image itself, opaque.
    assert_eq!(images(&pdf(&doc(ColorMode::Cmyk, None))), [("DeviceGray".into(), vec![], false)]);
}

/// Registration prints on every plate: the process inks and the document's spot inks.
#[test]
fn registration_writes_the_image_in_every_ink() {
    let reg = Paint::Solid { color: Color::cmyk(1.0, 1.0, 1.0, 1.0), swatch: Some(vectorcraft_color::swatch::REGISTRATION.into()), tint: 1.0 };
    let found = images(&pdf(&doc(ColorMode::Cmyk, Some(reg.clone()))));
    let all = ["Cyan", "Magenta", "Yellow", "Black", "Ink"].map(String::from).to_vec();
    assert!(found.contains(&("DeviceN".into(), all, true)), "{found:?}");

    // A spot swatch in a color group is one of the document's inks too.
    let mut d = doc(ColorMode::Cmyk, Some(reg));
    let grouped = Swatch { name: "Grouped Ink".into(), paint: Paint::solid(Color::cmyk(1.0, 0.0, 1.0, 0.0)), global: true, spot: true };
    d.swatch_groups.push(vectorcraft_color::SwatchGroup { name: "Group".into(), swatches: vec![grouped] });
    let found = images(&pdf(&d));
    let all = ["Cyan", "Magenta", "Yellow", "Black", "Ink", "Grouped Ink"].map(String::from).to_vec();
    assert!(found.contains(&("DeviceN".into(), all, true)), "{found:?}");
}

/// The Compression settings resample a colorized image as they do a grayscale one.
#[test]
fn colorized_images_are_downsampled() {
    let mut d = Document::new(100.0, 100.0);
    let gray = image::GrayImage::from_fn(600, 600, |x, y| image::Luma([((x + y) % 200) as u8]));
    let mut png = vec![];
    gray.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("pic".into(), ImageBlob::png(png));
    // One inch wide: 600 ppi.
    let im = ImageObject {
        key: "pic".into(),
        width: 600,
        height: 600,
        xf: Affine::scale(72.0 / 600.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.extras_mut().colorize = Some(Box::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0))));
    d.insert(d.default_layer(), 0, n).unwrap();
    let mut settings: PdfSettings =
        serde_json::from_value(serde_json::json!({"compression": {"gray": {"downsample": "bicubic", "ppi": 150}}})).unwrap();
    settings.compression.compress_text = false;
    let out = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("/Width 150") && !text.contains("/Width 600"), "resampled to 150 ppi");
}

/// Printed separations keep a colorized image's pixels on every plate and separate its color:
/// a spot-colorized image prints on its ink's plate only. An uncolorized image of the same pixels
/// is separated as before.
#[test]
fn separations_print_the_image_on_its_ink_plate() {
    let spot = Paint::Solid { color: Color::cmyk(0.0, 1.0, 1.0, 0.0), swatch: Some("Ink".into()), tint: 1.0 };
    let mut d = doc(ColorMode::Cmyk, Some(spot));
    let im = ImageObject { key: "pic".into(), width: 2, height: 1, xf: Affine::scale(20.0), link: None, placement: Default::default(), pixels: None };
    let plain = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    d.insert(d.default_layer(), 1, plain).unwrap();
    let settings: PrintSettings = serde_json::from_value(serde_json::json!({"output": {"mode": "separations"}})).unwrap();
    let names: Vec<String> = print_inks(&d, &settings).0.into_iter().map(|i| i.name).collect();
    let job = plan(&d, &PrintOptions { settings, ..Default::default() }).unwrap();
    assert_eq!(names.len(), job.plates.len());
    for (name, p) in names.iter().zip(&job.plates) {
        let images: Vec<_> = p.layers[0]
            .children()
            .unwrap()
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Image(im) => Some((p.image_blob(im).and_then(|b| b.coverage()).unwrap().2, n.extras().colorize.as_deref().cloned())),
                _ => None,
            })
            .collect();
        let [(colorized, Some(Paint::Solid { color, .. })), (plain, None)] = images.as_slice() else { panic!("{name}: {images:?}") };
        assert_eq!(colorized, &[255, 0], "{name}: the black pixel still covers");
        let k = if name == "Ink" { 1.0 } else { 0.0 };
        assert_eq!(color, &Color::Gray { k }, "{name}: the color is the plate's coverage");
        // The gray image itself has no spot ink.
        if name == "Ink" {
            assert_eq!(plain, &[0, 0], "{name}: the uncolorized image");
        }
    }
}

/// A pixelated colorized image (an envelope's with Anti-Alias off) is written without
/// /Interpolate, as an uncolorized one is.
#[test]
fn a_pixelated_colorized_image_asks_viewers_not_to_smooth_it() {
    for pixelated in [false, true] {
        let mut d = doc(ColorMode::Cmyk, Some(Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0))));
        let id = d.layers[0].children().unwrap()[0].id;
        d.node_mut(id).unwrap().extras_mut().pixelated = pixelated;
        let out = pdf(&d);
        assert_eq!(images(&out), [("DeviceGray".into(), vec![], false), ("DeviceCMYK".into(), vec![], true)], "pixelated: {pixelated}");
        assert_eq!(String::from_utf8_lossy(&out).contains("/Interpolate"), !pixelated, "pixelated: {pixelated}");
    }
}
