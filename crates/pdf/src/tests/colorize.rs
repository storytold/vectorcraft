//! Grayscale and 1-bit images colorized by a fill color are written in that color, a spot color
//! as its ink, with the image's coverage as the soft mask.
//! An image mask in a PDF opens as a grayscale image colorized by the color it is painted with.

use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Name, Object, Stream};
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{ColorMode, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};

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
fn a_spot_colorized_image_keeps_the_ink_values_of_its_swatch() {
    let spot = Paint::Solid { color: Color::cmyk(0.0, 0.75, 0.75, 0.1), swatch: Some("Ink".into()), tint: 1.0 };
    let mut d = doc(ColorMode::Cmyk, Some(spot));
    let Some(ink) = d.swatches.iter_mut().find(|s| s.name == "Ink") else { panic!("the Ink swatch") };
    ink.paint = Paint::solid(Color::cmyk(0.0, 0.75, 0.75, 0.1));
    let text = String::from_utf8_lossy(&pdf(&d)).into_owned();
    assert!(text.contains("/Separation/Ink/DeviceCMYK"), "{text}");
    assert!(text.contains("/C1[0 0.75 0.75 0.1]"), "M75 Y75 K10, not 191/255 and 26/255: {text}");
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

/// The pieces of a colorized image under a mesh warp, placed at different widths, paint one
/// colorized image and its one soft mask, with downsampling off and on.
#[test]
fn a_colorized_warped_image_is_written_once_for_all_its_pieces() {
    for compression in [serde_json::json!({}), serde_json::json!({"gray": {"downsample": "bicubic", "ppi": 150}})] {
        let mut d = Document::new(100.0, 100.0);
        let gray = image::GrayImage::from_fn(600, 600, |x, y| image::Luma([((x + y) % 200) as u8]));
        let mut png = vec![];
        gray.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        d.images.insert("pic".into(), ImageBlob::png(png));
        let im = ImageObject {
            key: "pic".into(),
            width: 600,
            height: 600,
            xf: Affine::scale(72.0 / 600.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im.clone())));
        n.extras_mut().colorize = Some(Box::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0))));
        // 0.5 to 1.5 times as wide from left to right.
        let stretch = |p: vectorcraft_geom::Point| vectorcraft_geom::Point::new(p.x * (0.5 + p.x / 144.0), p.y);
        let warped = vectorcraft_doc::live::mesh_warp_image(&n, &im, &stretch, 4, false, false);
        d.insert(d.default_layer(), 0, warped).unwrap();
        let settings: PdfSettings = serde_json::from_value(serde_json::json!({ "compression": compression })).unwrap();
        let out = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
        let found = images(&out);
        assert_eq!(found.len(), 2, "{compression}: the colorized image and its soft mask, not {found:?}");
        assert!(found.contains(&("DeviceRGB".into(), vec![], true)), "{compression}: {found:?}");
    }
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

/// Printed separations separate a colorized image with a color mask as it shows: the pixels the
/// mask hides print on no plate.
#[test]
fn separations_print_a_color_masked_image_as_it_shows() {
    let spot = Paint::Solid { color: Color::cmyk(0.0, 1.0, 1.0, 0.0), swatch: Some("Ink".into()), tint: 1.0 };
    let mut d = doc(ColorMode::Cmyk, Some(spot));
    // The black pixel hidden.
    let mut png = vec![];
    image::GrayAlphaImage::from_raw(2, 1, vec![0, 0, 255, 255])
        .unwrap()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    d.images.insert("shown".into(), ImageBlob::png(png));
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().extras_mut().color_mask = Some(Box::new(vectorcraft_doc::color_mask::ColorMask {
        key: "shown".into(),
        from: "pic".into(),
        background: vectorcraft_doc::color_mask::MaskBackground::Colors,
        colors: vec![[0, 0, 0]],
        contiguous: false,
        measure: vectorcraft_doc::color_mask::MaskMatch::Distance,
        tolerance: 0.0,
        softness: 0.0,
    }));
    let settings: PrintSettings = serde_json::from_value(serde_json::json!({"output": {"mode": "separations"}})).unwrap();
    let job = plan(&d, &PrintOptions { settings, ..Default::default() }).unwrap();
    assert!(!job.plates.is_empty());
    for p in &job.plates {
        let n = &p.layers[0].children().unwrap()[0];
        let NodeKind::Image(im) = &n.kind else { panic!("an image") };
        assert_eq!(p.image_blob(im).and_then(|b| b.coverage()).unwrap().2, [0, 0], "the hidden black pixel prints on no plate");
    }
}

/// A colorized image with a color mask keeps its colorize paint on the plates when its masked
/// pixels take it, as on the canvas, though its own pixels are a preview that doesn't.
#[test]
fn separations_colorize_masked_pixels_whose_own_pixels_are_a_preview() {
    let spot = Paint::Solid { color: Color::cmyk(0.0, 1.0, 1.0, 0.0), swatch: Some("Ink".into()), tint: 1.0 };
    let mut d = doc(ColorMode::Cmyk, Some(spot));
    let preview = Arc::new(vectorcraft_doc::links::missing_placeholder().unwrap());
    d.images
        .insert("pic".into(), ImageBlob { mime: "image/png".into(), bytes: preview.clone(), proxy: Some(preview), file_font: Default::default() });
    let mut png = vec![];
    image::GrayAlphaImage::from_raw(2, 1, vec![0, 0, 255, 255])
        .unwrap()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    d.images.insert("shown".into(), ImageBlob::png(png));
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().extras_mut().color_mask = Some(Box::new(vectorcraft_doc::color_mask::ColorMask {
        key: "shown".into(),
        from: "pic".into(),
        background: vectorcraft_doc::color_mask::MaskBackground::Colors,
        colors: vec![[0, 0, 0]],
        contiguous: false,
        measure: vectorcraft_doc::color_mask::MaskMatch::Distance,
        tolerance: 0.0,
        softness: 0.0,
    }));
    let settings: PrintSettings = serde_json::from_value(serde_json::json!({"output": {"mode": "separations"}})).unwrap();
    let job = plan(&d, &PrintOptions { settings, ..Default::default() }).unwrap();
    assert!(!job.plates.is_empty());
    for p in &job.plates {
        let n = &p.layers[0].children().unwrap()[0];
        let NodeKind::Image(im) = &n.kind else { panic!("an image") };
        assert!(n.extras().colorize.is_some(), "the masked pixels take the colorize paint");
        assert_eq!(p.image_blob(im).and_then(|b| b.coverage()).unwrap().2, [0, 0], "separated as the masked pixels");
    }
}

/// A linked image showing the missing-link placeholder is written in the placeholder's own colors
/// though it keeps a colorize paint for its file, and printed separations separate it as an
/// uncolorized image.
#[test]
fn a_colorized_missing_link_placeholder_is_written_in_its_own_colors() {
    let placeholder = || {
        let mut d = doc(ColorMode::Cmyk, None);
        let bytes = Arc::new(vectorcraft_doc::links::missing_placeholder().unwrap());
        d.images
            .insert("pic".into(), ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes), file_font: Default::default() });
        d
    };
    let plain = placeholder();
    let mut d = placeholder();
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().extras_mut().colorize = Some(Box::new(Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0))));
    let (colorized, uncolorized) = (images(&pdf(&d)), images(&pdf(&plain)));
    assert!(!colorized.is_empty());
    assert_eq!(colorized, uncolorized, "the placeholder as it is, without a soft mask from its coverage");
    let settings: PrintSettings = serde_json::from_value(serde_json::json!({"output": {"mode": "separations"}})).unwrap();
    let job = plan(&d, &PrintOptions { settings, ..Default::default() }).unwrap();
    assert!(!job.plates.is_empty());
    for p in &job.plates {
        let n = &p.layers[0].children().unwrap()[0];
        let NodeKind::Image(im) = &n.kind else { panic!("the image") };
        assert_eq!((im.key.as_str(), n.extras().colorize.is_none()), ("pic", true), "separated as the image it shows");
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

/// Ink options naming no ink give one warning, its lists cut short, however many there are.
#[test]
fn ink_options_naming_no_ink_give_one_short_warning() {
    let d = doc(ColorMode::Cmyk, None);
    let inks: Vec<serde_json::Value> = (0..10_000).map(|i| serde_json::json!({"name": format!("Missing {i}")})).collect();
    let settings: PrintSettings = serde_json::from_value(serde_json::json!({"output": {"mode": "separations", "inks": inks}})).unwrap();
    let (list, warnings) = print_inks(&d, &settings);
    assert!(list.iter().any(|i| i.name == "Cyan"));
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("`Missing 0`") && warnings[0].contains("and 9990 more") && warnings[0].len() < 500, "{}", warnings[0]);
}

/// An 8 × 1 image mask whose 0 bits (columns 2 to 5) are painted.
const MASK: &str = "<< /Type /XObject /Subtype /Image /Width 8 /Height 1 /ImageMask true /BitsPerComponent 1 /Filter /ASCIIHexDecode /Length 3 >>\nstream\nC3>\nendstream";

/// A page drawing [`MASK`] (`/Im1`) with `content`, with a spot ink `SpotRed` as `/CS0`.
fn mask_page(content: &str) -> Vec<u8> {
    let spot = "[/Separation /SpotRed /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >>]";
    let resources = format!("/ColorSpace << /CS0 {spot} >> /XObject << /Im1 {} 0 R >>", first_extra(1));
    pdf_with(&[PdfPage { resources, ..PdfPage::new(200.0, 100.0, content) }], &[MASK], None)
}

/// The imported images, in paint order: their colorize paint and gray pixels.
fn imported_images(d: &Document) -> Vec<(Option<Paint>, Vec<u8>)> {
    let mut out = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            if let NodeKind::Image(im) = &n.kind {
                let blob = d.images.get(&im.key).unwrap();
                let gray = image::load_from_memory(&blob.bytes).unwrap();
                assert_eq!(gray.color(), image::ColorType::L8, "a grayscale image");
                out.push((n.extras().colorize.as_deref().cloned(), gray.into_luma8().into_raw()));
            }
        });
    }
    out
}

/// What the mask paints is black, the rest white; a spot color keeps its swatch and tint.
#[test]
fn an_image_mask_opens_colorized_by_its_spot_color() {
    let d = crate::import(&mask_page("/CS0 cs 0.5 scn q 80 0 0 10 10 50 cm /Im1 Do Q")).unwrap();
    let images = imported_images(&d);
    assert_eq!(images.len(), 1);
    let (paint, gray) = &images[0];
    assert_eq!(gray, &[255, 255, 0, 0, 0, 0, 255, 255]);
    let Some(Paint::Solid { swatch, tint, .. }) = paint else { panic!("colorized: {paint:?}") };
    assert_eq!((swatch.as_deref(), *tint), (Some("SpotRed"), 0.5));
    assert!(d.swatch("SpotRed").is_some_and(|s| s.spot), "a spot swatch");
}

/// A mask drawn twice keeps each color it is painted with, over the same pixels.
#[test]
fn an_image_mask_drawn_in_two_colors_keeps_both() {
    let d = crate::import(&mask_page("1 0 0 rg q 80 0 0 10 10 20 cm /Im1 Do Q 0 0 1 rg q 80 0 0 10 10 60 cm /Im1 Do Q")).unwrap();
    let images = imported_images(&d);
    let hex: Vec<_> = images.iter().map(|(p, _)| p.as_ref().and_then(Paint::color).map(|c| c.to_hex())).collect();
    assert_eq!(hex, [Some("#ff0000".to_string()), Some("#0000ff".to_string())]);
    assert_eq!(images[0].1, images[1].1);
}

/// An uncolored tiling pattern whose cell draws a mask, filled in red and then in blue, gives a
/// pattern for each color: the readings of its cell differ by the mask's color.
#[test]
fn an_uncolored_pattern_drawing_an_image_mask_keeps_each_color() {
    let (mask, pattern) = (first_extra(1), first_extra(1) + 1);
    let resources = format!("/ColorSpace << /CS1 [/Pattern /DeviceRGB] >> /Pattern << /P0 {pattern} 0 R >>");
    let cell = format!(
        "<< /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /XObject << /Im1 {mask} 0 R >> >> /Length 28 >>\nstream\nq 10 0 0 10 0 0 cm /Im1 Do Q\nendstream"
    );
    let content = "/CS1 cs 1 0 0 /P0 scn 0 0 100 40 re f 0 0 1 /P0 scn 0 50 100 40 re f";
    let d = crate::import(&pdf_with(&[PdfPage { resources, ..PdfPage::new(100.0, 100.0, content) }], &[MASK, &cell], None)).unwrap();
    let mut colors = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            for item in &n.appearance.items {
                let Paint::Pattern { pattern, .. } = item.paint() else { continue };
                let mut hex = vec![];
                for a in &d.pattern(pattern).unwrap().art {
                    a.walk(&mut |c| hex.extend(c.extras().colorize.as_deref().and_then(Paint::color).map(|c| c.to_hex())));
                }
                colors.push(hex);
            }
        });
    }
    assert_eq!(colors, [["#ff0000"], ["#0000ff"]]);
}

const SPOT_RED: &str = "[/Separation /SpotRed /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >>]";

/// A page drawing a `w` × 1 image in color space `space` (`bpc` bits a sample, more dictionary
/// entries `more`, samples `hex`), then the `extra` objects after it.
fn separation_page(space: &str, w: u32, bpc: u8, more: &str, hex: &str, extra: &[&str]) -> Document {
    separation_report(space, w, bpc, more, hex, extra, "q 80 0 0 10 10 50 cm /Im1 Do Q").document
}

/// [`separation_page`] drawn with `content`, with the import's report. `more` may name filters
/// after ASCIIHex (`/Filter [/ASCIIHexDecode …]`).
fn separation_report(space: &str, w: u32, bpc: u8, more: &str, hex: &str, extra: &[&str], content: &str) -> ImportReport {
    let filter = if more.contains("/Filter") { "" } else { "/Filter /ASCIIHexDecode" };
    let image = format!(
        "<< /Type /XObject /Subtype /Image /Width {w} /Height 1 /ColorSpace {space} /BitsPerComponent {bpc} {more} {filter} /Length {} >>\nstream\n{hex}>\nendstream",
        hex.len() + 1
    );
    let resources = format!("/XObject << /Im1 {} 0 R >>", first_extra(1));
    let mut objects = vec![image.as_str()];
    objects.extend_from_slice(extra);
    let pdf = pdf_with(&[PdfPage { resources, ..PdfPage::new(100.0, 100.0, content) }], &objects, None);
    import_with_report(&pdf, &ImportOptions::default()).unwrap()
}

/// An image in a spot ink's Separation space opens as a grayscale image of its tints, colorized by
/// the ink's spot swatch at 100 %.
#[test]
fn a_separation_image_opens_colorized_by_its_ink() {
    let d = separation_page(SPOT_RED, 4, 8, "", "0055AAFF", &[]);
    let images = imported_images(&d);
    assert_eq!(images.len(), 1);
    let (paint, gray) = &images[0];
    assert_eq!(gray, &[255, 170, 85, 0], "the tints 0, 1/3, 2/3 and 1");
    let Some(Paint::Solid { swatch, tint, .. }) = paint else { panic!("colorized: {paint:?}") };
    assert_eq!((swatch.as_deref(), *tint), (Some("SpotRed"), 1.0));
    assert!(d.swatch("SpotRed").is_some_and(|s| s.spot), "a spot swatch");
}

/// Samples of 1, 4 and 16 bits are unpacked, and the Decode array applied.
#[test]
fn a_separation_image_reads_its_samples_and_decode_array() {
    for (bpc, w, more, hex, want) in [
        (1, 8, "/Decode [1 0]", "C3", vec![255, 255, 0, 0, 0, 0, 255, 255]),
        (4, 3, "", "0F80", vec![255, 0, 119]),
        (16, 2, "", "0000FFFF", vec![255, 0]),
    ] {
        let d = separation_page(SPOT_RED, w, bpc, more, hex, &[]);
        let images = imported_images(&d);
        assert_eq!(images.len(), 1, "{bpc} bits");
        assert_eq!(images[0].1, want, "{bpc} bits");
        assert!(images[0].0.is_some(), "{bpc} bits: colorized");
    }
}

/// A Separation of a process plate colorizes in that plate's CMYK color; the ink `None`, which
/// paints nothing, is left to the interpreter.
#[test]
fn a_separation_image_of_a_process_plate_or_of_no_ink() {
    let black = "[/Separation /Black /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 0 1] /N 1 >>]";
    let d = separation_page(black, 2, 8, "", "00FF", &[]);
    let images = imported_images(&d);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].0, Some(Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0))));
    assert!(d.swatches.iter().all(|s| !s.spot), "no spot swatch for a process plate");
    let none = "[/Separation /None /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >>]";
    let d = separation_page(none, 2, 8, "", "00FF", &[]);
    let mut colorized = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            if matches!(n.kind, NodeKind::Image(_)) {
                colorized.push(n.extras().colorize.is_some());
            }
        });
    }
    assert!(colorized.iter().all(|c| !c), "{colorized:?}");
}

/// A Separation image's soft mask becomes its opacity mask.
#[test]
fn a_separation_image_keeps_its_soft_mask() {
    let smask = "<< /Type /XObject /Subtype /Image /Width 4 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length 9 >>\nstream\nFF00FF00>\nendstream";
    let d = separation_page(SPOT_RED, 4, 8, &format!("/SMask {} 0 R", first_extra(1) + 1), "0055AAFF", &[smask]);
    let mut found = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            if matches!(n.kind, NodeKind::Image(_)) && n.extras().colorize.is_some() {
                found.push(n.extras().mask.is_some());
            }
        });
    }
    assert_eq!(found, [true]);
}

/// The same Separation image drawn twice is colorized twice, over the same pixels.
#[test]
fn a_separation_image_drawn_twice_is_colorized_twice() {
    let d = separation_report(SPOT_RED, 4, 8, "", "0055AAFF", &[], "q 80 0 0 10 10 20 cm /Im1 Do Q q 80 0 0 10 10 60 cm /Im1 Do Q").document;
    let images = imported_images(&d);
    assert_eq!(images.len(), 2);
    assert_eq!(images[0], images[1]);
    assert!(images[0].0.is_some(), "colorized");
}

/// A one-component JPEG in a Separation space is read through its decoder.
#[test]
fn a_separation_jpeg_opens_colorized_by_its_ink() {
    let mut jpeg = vec![];
    jpeg_encoder::Encoder::new(&mut jpeg, 100).encode(&[0, 0, 255, 255, 0, 0, 255, 255], 8, 1, jpeg_encoder::ColorType::Luma).unwrap();
    let hex: String = jpeg.iter().map(|b| format!("{b:02X}")).collect();
    let d = separation_page(SPOT_RED, 8, 8, "/Filter [/ASCIIHexDecode /DCTDecode]", &hex, &[]);
    let images = imported_images(&d);
    assert_eq!(images.len(), 1);
    assert!(images[0].0.is_some(), "colorized");
    // Sample 0 lays no ink (white), 255 lays it all (black); JPEG blurs the edges a little.
    let gray = &images[0].1;
    assert!(gray[0] > 200 && gray[2] < 55 && gray[4] > 200 && gray[6] < 55, "{gray:?}");
}

/// A Separation image whose tints can't be read (its data cut short) opens in RGB each time it is
/// drawn, with one warning and no swatch for its ink.
#[test]
fn a_separation_image_that_cant_be_read_stays_rgb() {
    let r = separation_report(SPOT_RED, 4, 8, "", "0055", &[], "q 80 0 0 10 10 20 cm /Im1 Do Q q 80 0 0 10 10 60 cm /Im1 Do Q");
    let mut colorized = vec![];
    for l in &r.document.layers {
        l.walk(&mut |n| {
            if matches!(n.kind, NodeKind::Image(_)) {
                colorized.push(n.extras().colorize.is_some());
            }
        });
    }
    assert_eq!(colorized, [false, false]);
    assert!(r.document.swatches.iter().all(|s| !s.spot), "no swatch for an ink nothing uses");
    assert_eq!(r.warnings.iter().filter(|w| w.contains("spot ink")).count(), 1, "{:?}", r.warnings);
}

/// A Type 4 tint transform as an object, `code` its program.
fn type4(code: &str, inputs: usize) -> String {
    let domain = "0 1 ".repeat(inputs);
    format!("<< /FunctionType 4 /Domain [{}] /Range [0 1 0 1 0 1 0 1] /Length {} >>\nstream\n{code}\nendstream", domain.trim_end(), code.len())
}

/// The imported images' blobs, in paint order.
fn image_blobs(d: &Document) -> Vec<(ImageBlob, bool)> {
    let mut out = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            if let NodeKind::Image(im) = &n.kind {
                out.push((d.images[&im.key].clone(), n.extras().colorize.is_some()));
            }
        });
    }
    out
}

/// A DeviceN image of one colorant opens as a Separation image does: its tints, colorized by the ink.
#[test]
fn a_devicen_image_of_one_ink_opens_colorized_by_it() {
    let space = "[/DeviceN [/SpotRed] /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >>]";
    let d = separation_page(space, 4, 8, "", "0055AAFF", &[]);
    let images = imported_images(&d);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].1, [255, 170, 85, 0]);
    assert!(matches!(&images[0].0, Some(Paint::Solid { swatch: Some(s), .. }) if s == "SpotRed"), "{:?}", images[0].0);
}

/// A DeviceN image of process plates only opens in CMYK, each colorant's tint in its plate.
#[test]
fn a_devicen_image_of_process_plates_opens_in_cmyk() {
    let space = format!("[/DeviceN [/Cyan /Magenta] /DeviceCMYK {} 0 R]", first_extra(1) + 1);
    // Cyan and Magenta → (c, m, 0, 0); pixels: none, cyan, magenta, both.
    let d = separation_page(&space, 4, 8, "", "0000FF0000FFFFFF", &[&type4("{ 0 0 }", 2)]);
    let blobs = image_blobs(&d);
    assert_eq!(blobs.len(), 1);
    let (blob, colorized) = &blobs[0];
    assert!(blob.is_cmyk() && !colorized, "a CMYK image");
    let inks = blob.cmyk().unwrap().data;
    assert_eq!(inks, [0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0]);

    // 16 bits a sample, the ink None among the plates, and a Decode array that inverts Cyan.
    let space = format!("[/DeviceN [/Cyan /None /Black] /DeviceCMYK {} 0 R]", first_extra(1) + 1);
    let d = separation_page(&space, 2, 16, "/Decode [1 0 0 1 0 1]", "FFFF0000000000000000FFFF", &[&type4("{ exch pop 0 0 3 -1 roll }", 3)]);
    let blobs = image_blobs(&d);
    assert_eq!(blobs[0].0.cmyk().unwrap().data, [0, 0, 0, 0, 255, 0, 0, 255]);
}

/// A DeviceN image of a spot ink and a process plate (a duotone) still opens as RGB: an image can
/// hold one ink or the process plates, not both.
#[test]
fn a_duotone_devicen_image_still_opens_as_rgb() {
    let space = format!("[/DeviceN [/Black /SpotRed] /DeviceCMYK {} 0 R]", first_extra(1) + 1);
    let d = separation_page(&space, 2, 8, "", "FF0000FF", &[&type4("{ dup 3 -1 roll 0 4 1 roll }", 2)]);
    let blobs = image_blobs(&d);
    assert_eq!(blobs.len(), 1);
    assert!(!blobs[0].0.is_cmyk() && !blobs[0].1, "RGB, not colorized");
}

/// A raster drawn again is placed from what the first drawing found, as the same image at its own
/// size, without its pixels being converted again.
#[test]
fn a_raster_drawn_twice_is_one_image_placed_twice() {
    let content = "q 80 0 0 10 10 50 cm /Im1 Do Q q 40 0 0 10 10 20 cm /Im1 Do Q";
    let r = separation_report("/DeviceRGB", 2, 8, "", "FF0000 00FF00", &[], content);
    let mut placed = vec![];
    r.document.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            placed.push((im.key.clone(), im.xf.as_coeffs()[0]));
        }
    });
    assert_eq!(placed.len(), 2, "{placed:?}");
    assert_eq!(placed[0].0, placed[1].0);
    assert!(placed[0].1 > placed[1].1 * 1.9 && placed[0].1 < placed[1].1 * 2.1, "{placed:?}");
    assert_eq!(r.document.images.len(), 1);
}
