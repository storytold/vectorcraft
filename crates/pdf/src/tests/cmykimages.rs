//! CMYK images keep their CMYK samples from import to export (issue #320): a CMYK JPEG passes
//! through unchanged, other CMYK images keep their ink amounts, and only the Output settings
//! convert them.

use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Name, Object, Stream};
use serde_json::json;
use vectorcraft_color::cms::{self, DEVICE_CMYK, GENERIC_CMYK, SRGB};
use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientMethod, GradientPaint, GradientStop, Paint, Swatch};
use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{Appearance, ColorMode, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

use crate::*;

/// `inks` (four bytes a pixel) as a CMYK JPEG: Adobe's inverted samples, with its marker.
fn cmyk_jpeg(inks: &[u8], w: u16, h: u16) -> Vec<u8> {
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 100).encode(inks, w, h, jpeg_encoder::ColorType::Cmyk).unwrap();
    out
}

fn flat(inks: [u8; 4], n: usize) -> Vec<u8> {
    inks.repeat(n)
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}

/// A 300 × 100 pt page with a CMYK rectangle and image XObjects `images` (dictionary entries
/// after the type, stream data), 50 pt wide each: objects 5…, then the `extra` objects.
fn pdf_with_images(images: &[(String, Vec<u8>)], extra: &[Vec<u8>]) -> Vec<u8> {
    let xobjects: String = (0..images.len()).map(|i| format!("/Im{i} {} 0 R ", 5 + i)).collect();
    let mut content = String::from("0 1 0.6 0.2 k 0 80 300 20 re f\n");
    for i in 0..images.len() {
        content.push_str(&format!("q 50 0 0 50 {} 10 cm /Im{i} Do Q\n", 10 + 60 * i));
    }
    let mut objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 100] /Resources << /XObject << {xobjects}>> >> /Contents 4 0 R >>").into_bytes(),
        stream("", content.as_bytes()),
    ];
    for (dict, data) in images {
        objs.push(stream(&format!("/Type /XObject /Subtype /Image {dict}"), data));
    }
    objs.extend_from_slice(extra);
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut at = vec![];
    for (i, o) in objs.iter().enumerate() {
        at.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for a in at {
        out.extend_from_slice(format!("{a:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// An image dictionary's entries: `w` × `h` CMYK pixels, with `more`.
fn cmyk_dict(w: u32, h: u32, more: &str) -> String {
    format!("/Width {w} /Height {h} /ColorSpace /DeviceCMYK /BitsPerComponent 8 {more}")
}

const INVERTED: &str = "/Decode [1 0 1 0 1 0 1 0]";

/// An image XObject of a written PDF: its colour space and the components and bytes of its ICC
/// profile, its Decode array, its filter, its stream data and whether it has a soft mask.
#[derive(Debug)]
struct Written {
    space: String,
    icc_n: Option<u8>,
    icc: Option<Vec<u8>>,
    decode: Vec<f32>,
    filter: Option<String>,
    data: Vec<u8>,
    width: u32,
    smask: bool,
}

fn written(pdf: &[u8]) -> Vec<Written> {
    let file = Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let mut out = vec![];
    for o in file.objects() {
        let Object::Stream(s) = o else { continue };
        let d = s.dict();
        if d.get::<Name<'_>>(b"Subtype").is_none_or(|n| n.as_ref() != b"Image") {
            continue;
        }
        let (space, icc_n, icc) = match (d.get::<Name<'_>>(b"ColorSpace"), d.get::<Array<'_>>(b"ColorSpace")) {
            (Some(n), _) => (n.as_str().to_string(), None, None),
            (_, Some(a)) => {
                let mut it = a.flex_iter();
                let name = it.next::<Name<'_>>().unwrap().as_str().to_string();
                let profile = it.next::<Stream<'_>>();
                let n = profile.as_ref().and_then(|s| s.dict().get::<u8>(b"N"));
                (name, n, profile.map(|s| s.decoded().unwrap().into_owned()))
            }
            _ => (String::new(), None, None),
        };
        out.push(Written {
            space,
            icc_n,
            icc,
            decode: d.get::<Array<'_>>(b"Decode").map(|a| a.iter::<f32>().collect()).unwrap_or_default(),
            filter: d.get::<Name<'_>>(b"Filter").map(|n| n.as_str().to_string()),
            data: s.raw_data().to_vec(),
            width: d.get::<u32>(b"Width").unwrap_or(0),
            smask: d.get::<Stream<'_>>(b"SMask").is_some(),
        });
    }
    out
}

/// The blob each image of `d` shows, in paint order.
fn blobs(d: &Document) -> Vec<&ImageBlob> {
    let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!() };
    children
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Image(im) => d.images.get(&im.key),
            _ => None,
        })
        .collect()
}

/// Options with settings `s` (at their standard's version).
fn opts(s: serde_json::Value) -> PdfOptions {
    let settings: PdfSettings = serde_json::from_value(s).unwrap();
    let compatibility = if settings.standard == Standard::None { settings.compatibility } else { settings.standard.version() };
    PdfOptions { settings: PdfSettings { compatibility, ..settings }, created: Some(1_700_000_000), ..Default::default() }
}

fn close(a: &[u8], b: &[u8], tolerance: u8) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= tolerance)
}

#[test]
fn a_cmyk_jpeg_passes_through_open_and_every_pdf_export() {
    // The issue's file: a DeviceCMYK JPEG (Adobe's inverted samples, Decode [1 0 …]) beside a
    // DeviceCMYK fill.
    let jpg = cmyk_jpeg(&flat([0, 102, 255, 0], 32 * 16), 32, 16);
    let source = pdf_with_images(&[(cmyk_dict(32, 16, &format!("/Filter /DCTDecode {INVERTED}")), jpg.clone())], &[]);
    let r = import_with_report(&source, &ImportOptions::default()).unwrap();
    assert_eq!(r.document.color_mode, ColorMode::Cmyk);
    let b = blobs(&r.document);
    assert_eq!(b.len(), 1);
    assert_eq!((b[0].mime.as_str(), b[0].bytes.as_slice()), ("image/jpeg", jpg.as_slice()), "the JPEG as it is");
    assert!(b[0].cmyk().unwrap().data.as_chunks::<4>().0.iter().all(|p| close(p, &[0, 102, 255, 0], 2)));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);

    for (settings, space, icc_n) in [
        (json!({}), "DeviceCMYK", None),
        (json!({"output": {"conversion": "preserveNumbers"}}), "DeviceCMYK", None),
        (json!({"standard": "pdfX1a"}), "DeviceCMYK", None),
        (json!({"standard": "pdfX3"}), "ICCBased", Some(4)),
        (json!({"standard": "pdfX4"}), "ICCBased", Some(4)),
        (json!({"output": {"profiles": "all"}}), "ICCBased", Some(4)),
    ] {
        let out = export_with_report(&r.document, &opts(settings.clone())).unwrap();
        let w = written(&out.bytes);
        assert_eq!(w.len(), 1, "{settings}: {w:?}");
        assert_eq!((w[0].space.as_str(), w[0].icc_n), (space, icc_n), "{settings}");
        assert_eq!(w[0].filter.as_deref(), Some("DCTDecode"), "{settings}");
        assert_eq!(w[0].decode, [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0], "{settings}");
        assert!(w[0].data == jpg, "{settings}: the JPEG data unchanged");
        assert!(out.warnings.iter().all(|w| !w.contains("image")), "{settings}: {:?}", out.warnings);
        // And open again: the same JPEG.
        let back = import(&out.bytes).unwrap();
        assert_eq!(blobs(&back)[0].bytes.as_slice(), jpg.as_slice(), "{settings}");
    }
}

#[test]
fn other_cmyk_images_keep_their_ink_amounts() {
    let inks: Vec<u8> = [[0, 0, 0, 0], [255, 0, 0, 0], [10, 20, 30, 40], [0, 102, 255, 0]].concat();
    let z = |data: &[u8]| crate::output::deflate(data).unwrap();
    let inverted: Vec<u8> = inks.iter().map(|v| 255 - v).collect();
    let wide: Vec<u8> = inks.iter().flat_map(|v| (*v as u16 * 257).to_be_bytes()).collect();
    let images = [
        (cmyk_dict(2, 2, "/Filter /FlateDecode"), z(&inks)),
        (cmyk_dict(2, 2, &format!("/Filter /FlateDecode {INVERTED}")), z(&inverted)),
        (cmyk_dict(2, 2, "/Filter /FlateDecode").replace("/BitsPerComponent 8", "/BitsPerComponent 16"), z(&wide)),
        // A CMYK JPEG holding the ink amounts themselves (no Decode array): not Adobe's convention.
        (cmyk_dict(2, 2, "/Filter /DCTDecode"), cmyk_jpeg(&inverted, 2, 2)),
        // ICC-based with four components (object 10: after the page's 4 objects and 5 images).
        (cmyk_dict(2, 2, "/Filter /FlateDecode").replace("/DeviceCMYK", "[/ICCBased 10 0 R]"), z(&inks)),
    ];
    let source = pdf_with_images(&images, &[stream("/N 4", b"")]);
    let r = import_with_report(&source, &ImportOptions::default()).unwrap();
    let b = blobs(&r.document);
    assert_eq!(b.len(), 5, "{:?}", r.warnings);
    for (i, blob) in b.iter().enumerate() {
        assert_eq!(blob.mime, "image/tiff", "image {i}");
        let got = blob.cmyk().unwrap();
        assert_eq!((got.width, got.height), (2, 2));
        let tolerance = if i == 3 { 3 } else { 0 };
        assert!(close(&got.data, &inks, tolerance), "image {i}: {:?}", got.data);
    }
    // Written as CMYK, losslessly: they open with the same ink amounts.
    let out = export_with_report(&r.document, &opts(json!({}))).unwrap();
    let w = written(&out.bytes);
    assert!(!w.is_empty() && w.iter().all(|w| w.space == "DeviceCMYK" && w.decode.is_empty()), "{w:?}");
    let back = import(&out.bytes).unwrap();
    for blob in blobs(&back) {
        assert!(close(&blob.cmyk().unwrap().data, &inks, 3));
    }
}

#[test]
fn cmyk_images_with_odd_samples_open_in_rgb_with_a_warning() {
    let four_bits = cmyk_dict(2, 2, "").replace("/BitsPerComponent 8", "/BitsPerComponent 4");
    let r = import_with_report(&pdf_with_images(&[(four_bits, vec![0x0F; 8])], &[]), &ImportOptions::default()).unwrap();
    assert_eq!(blobs(&r.document)[0].mime, "image/png");
    assert!(r.warnings.iter().any(|w| w == crate::import_image::UNREAD), "{:?}", r.warnings);
}

/// An opacity mask's image: its blob and pixel size.
type MaskImage<'d> = (&'d ImageBlob, u32, u32);

/// The image nodes on the first layer: each one's blob and its opacity mask's image, if it has one.
fn masked(d: &Document) -> Vec<(&ImageBlob, Option<MaskImage<'_>>)> {
    let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!() };
    children
        .iter()
        .filter_map(|n| {
            let NodeKind::Image(im) = &n.kind else { return None };
            let mask = n.extras().mask.as_ref().map(|m| match &m.art.kind {
                NodeKind::Image(a) => (&d.images[&a.key], a.width, a.height),
                other => panic!("mask art is an image: {other:?}"),
            });
            Some((&d.images[&im.key], mask))
        })
        .collect()
}

#[test]
fn masked_cmyk_images_keep_their_inks_and_carry_the_mask_as_an_opacity_mask() {
    let inks = flat([0, 102, 255, 0], 4);
    let z = |data: &[u8]| crate::output::deflate(data).unwrap();
    // Object 6 (the first after the two images): a soft mask; image 2 a colour key masking its
    // first two pixels.
    let smask = stream("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", &[255, 128, 255, 128]);
    let keyed: Vec<u8> = [[10, 20, 30, 40], [10, 20, 30, 40], [0, 102, 255, 0], [0, 102, 255, 0]].concat();
    let images = [
        (cmyk_dict(2, 2, "/SMask 7 0 R /Filter /FlateDecode"), z(&inks)),
        (cmyk_dict(2, 2, "/Mask [10 10 20 20 30 30 40 40] /Filter /FlateDecode"), z(&keyed)),
    ];
    let source = pdf_with_images(&images, &[smask]);
    let r = import_with_report(&source, &ImportOptions::default()).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let m = masked(&r.document);
    assert_eq!(m.len(), 2);
    for ((blob, mask), (want, alpha)) in m.iter().zip([(&inks, [255, 128, 255, 128]), (&keyed, [0, 0, 255, 255])]) {
        assert_eq!(blob.mime, "image/tiff");
        assert!(close(&blob.cmyk().unwrap().data, want, 0), "the inks as they were");
        let (mblob, mw, mh) = mask.expect("an opacity mask");
        assert_eq!((mw, mh), (2, 2));
        let grey = image::load_from_memory(&mblob.bytes).unwrap().to_luma8();
        assert_eq!(grey.into_raw(), alpha, "the mask's values");
    }
    // The mask art covers the image exactly.
    let NodeKind::Layer { children, .. } = &r.document.layers[0].kind else { panic!() };
    let img = children.iter().find(|n| matches!(n.kind, NodeKind::Image(_))).unwrap();
    assert_eq!(img.geometric_bounds(), img.extras().mask.as_ref().unwrap().art.geometric_bounds());
    // Written as CMYK with the mask in grey; it opens with the same inks.
    let out = export_with_report(&r.document, &opts(json!({}))).unwrap();
    let w = written(&out.bytes);
    assert!(w.iter().filter(|w| w.space == "DeviceCMYK").count() == 2 && w.iter().any(|w| w.space == "DeviceGray"), "{w:?}");
    let back = import_with_report(&out.bytes, &ImportOptions::default()).unwrap();
    let inked: Vec<Vec<u8>> = all_images(&back.document).iter().filter_map(|b| b.cmyk()).map(|i| i.data).collect();
    assert!(inked.iter().any(|d| close(d, &inks, 3)) && inked.iter().any(|d| close(d, &keyed, 3)), "{:?}", back.warnings);
    // A mask that hides nothing adds none.
    let opaque = stream("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", &[255; 4]);
    let r = import_with_report(
        &pdf_with_images(&[(cmyk_dict(2, 2, "/SMask 6 0 R /Filter /FlateDecode"), z(&inks))], &[opaque]),
        &ImportOptions::default(),
    )
    .unwrap();
    assert!(masked(&r.document)[0].1.is_none());
}

/// Every image blob a document's art uses (mask art included).
fn all_images(d: &Document) -> Vec<&ImageBlob> {
    let mut keys = vec![];
    d.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            keys.push(im.key.clone());
        }
        if let Some(m) = &n.extras().mask {
            m.art.walk(&mut |a| {
                if let NodeKind::Image(im) = &a.kind {
                    keys.push(im.key.clone());
                }
            });
        }
    });
    keys.iter().filter_map(|k| d.images.get(k)).collect()
}

/// A document with CMYK image `blob` of `w` × `h` pixels placed 40 pt wide.
fn image_doc(blob: ImageBlob, w: u32, h: u32) -> Document {
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    d.images.insert("img".into(), blob);
    let image = ImageObject {
        key: "img".into(),
        width: w,
        height: h,
        xf: Affine::scale(40.0 / w as f64),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(image)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// An image placed from a DCS file prints each of its plates on its own ink: a spot plate on its
/// spot ink's plate, or as its process equivalent when spots print as process. Its composite, the
/// CMYK samples every other reader sees, never prints on a separation.
#[test]
fn a_dcs_image_prints_each_plate_on_its_own_ink() {
    use vectorcraft_doc::separated::{InkPlate, Separated};
    let plate = |name: &str, cmyk: [f32; 4], data: [u8; 2]| InkPlate { name: name.into(), cmyk, data: data.to_vec() };
    let s = Separated::new(2, 1, vec![plate("Cyan", [0.0; 4], [255, 0]), plate("Spot Orange", [0.0, 0.6, 1.0, 0.0], [128, 0])]).unwrap();
    let mut d = image_doc(ImageBlob::separated_tiff(&s, None, &[]).unwrap(), 2, 1);
    d.swatches.push(Swatch { name: "Spot Orange".into(), paint: Paint::solid(Color::cmyk(0.0, 0.6, 1.0, 0.0)), global: true, spot: true });
    let plates = |spots_to_process: bool| {
        let settings: PrintSettings = serde_json::from_value(json!({"output": {"mode": "separations", "spotsToProcess": spots_to_process}})).unwrap();
        let names: Vec<String> = print_inks(&d, &settings).0.into_iter().map(|i| i.name).collect();
        let job = plan(&d, &PrintOptions { settings, ..Default::default() }).unwrap();
        let coverage: Vec<Vec<u8>> = job
            .plates
            .iter()
            .map(|p| {
                let NodeKind::Image(im) = &p.layers[0].children().unwrap()[0].kind else { panic!("an image") };
                p.image_blob(im).and_then(|b| b.coverage()).unwrap().2
            })
            .collect();
        names.into_iter().zip(coverage).collect::<Vec<_>>()
    };
    let own = plates(false);
    let get = |all: &[(String, Vec<u8>)], name: &str| all.iter().find(|(n, _)| n == name).map(|(_, c)| c.clone());
    assert_eq!(get(&own, "Cyan"), Some(vec![255, 0]));
    assert_eq!(get(&own, "Magenta"), Some(vec![0, 0]), "the spot ink stays off the process plates");
    assert_eq!(get(&own, "Yellow"), Some(vec![0, 0]));
    assert_eq!(get(&own, "Spot Orange"), Some(vec![128, 0]));
    let process = plates(true);
    assert_eq!(get(&process, "Spot Orange"), None);
    assert_eq!(get(&process, "Magenta"), Some(vec![77, 0]), "60% of the spot plate's 50%");
    assert_eq!(get(&process, "Yellow"), Some(vec![128, 0]));
}

/// A DCS spot plate that prints as process prints as the document's swatch of its ink does (the
/// swatch kept in place of the plate's), not as the equivalent its file gives.
#[test]
fn a_dcs_spot_plate_printing_as_process_takes_the_documents_swatch() {
    use vectorcraft_doc::separated::{InkPlate, Separated};
    let plate = |name: &str, cmyk: [f32; 4], data: [u8; 2]| InkPlate { name: name.into(), cmyk, data: data.to_vec() };
    let s = Separated::new(2, 1, vec![plate("Cyan", [0.0; 4], [255, 0]), plate("Spot Orange", [0.0, 0.6, 1.0, 0.0], [0, 128])]).unwrap();
    let mut d = image_doc(ImageBlob::separated_tiff(&s, None, &[]).unwrap(), 2, 1);
    let plates = |d: &Document, spots_to_process: bool| {
        let settings: PrintSettings = serde_json::from_value(json!({"output": {"mode": "separations", "spotsToProcess": spots_to_process}})).unwrap();
        let names: Vec<String> = print_inks(d, &settings).0.into_iter().map(|i| i.name).collect();
        let job = plan(d, &PrintOptions { settings, ..Default::default() }).unwrap();
        let coverage = job.plates.iter().map(|p| {
            let NodeKind::Image(im) = &p.layers[0].children().unwrap()[0].kind else { panic!("an image") };
            p.image_blob(im).and_then(|b| b.coverage()).unwrap().2
        });
        names.into_iter().zip(coverage).collect::<Vec<_>>()
    };
    let get = |all: &[(String, Vec<u8>)], name: &str| all.iter().find(|(n, _)| n == name).map(|(_, c)| c.clone());
    // A spot swatch of the ink in another color.
    d.swatches.push(Swatch { name: "Spot Orange".into(), paint: Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)), global: true, spot: true });
    let process = plates(&d, true);
    assert_eq!(get(&process, "Cyan"), Some(vec![255, 128]), "the swatch's cyan");
    assert_eq!(get(&process, "Magenta"), Some(vec![0, 0]), "not the file's magenta");
    assert_eq!(get(&process, "Yellow"), Some(vec![0, 0]));
    // A process swatch of the ink's name: the job prints no plate for it.
    d.swatches.retain(|w| w.name != "Spot Orange");
    d.swatches.push(Swatch { name: "Spot Orange".into(), paint: Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)), global: true, spot: false });
    let own = plates(&d, false);
    assert_eq!(get(&own, "Spot Orange"), None);
    assert_eq!(get(&own, "Black"), Some(vec![0, 128]), "the swatch's black");
    assert_eq!(get(&own, "Yellow"), Some(vec![0, 0]));
}

#[test]
fn the_output_settings_convert_cmyk_images_and_compression_recompresses_them_in_cmyk() {
    let ink = [0u8, 102, 255, 0];
    let tiff = ImageBlob::cmyk_tiff(&Inks::new(4, 4, flat(ink, 16)).unwrap()).unwrap();
    let d = image_doc(tiff, 4, 4);
    let first = |s: serde_json::Value| {
        let out = export_with_report(&d, &opts(s.clone())).unwrap();
        let w = written(&out.bytes);
        assert_eq!(w.len(), 1, "{s}");
        let back = import(&out.bytes).unwrap();
        let blob = blobs(&back)[0].clone();
        (w.into_iter().next().unwrap(), blob, out.warnings)
    };
    // Kept: its numbers.
    for s in [json!({}), json!({"output": {"conversion": "preserveNumbers", "destination": DEVICE_CMYK}})] {
        let (w, blob, warnings) = first(s.clone());
        assert_eq!(w.space, "DeviceCMYK", "{s}");
        assert_eq!(blob.cmyk().unwrap().data, flat(ink, 16), "{s}");
        assert!(warnings.is_empty(), "{s}: {warnings:?}");
    }
    // To another CMYK profile: other numbers, still CMYK.
    let (w, blob, _) = first(json!({"output": {"conversion": "destination", "destination": DEVICE_CMYK}}));
    assert_eq!(w.space, "DeviceCMYK");
    let converted = blob.cmyk().unwrap().data;
    assert_ne!(converted, flat(ink, 16));
    let source = cms::active();
    let dest = cms::Cms::new(&cms::ColorSettings { cmyk: DEVICE_CMYK.into(), ..source.settings().clone() }).unwrap();
    let want = dest.lab_to_cmyk(source.cmyk_display_lab(ink.map(|v| v as f32 / 255.0)), source.settings().intent).map(|v| (v * 255.0).round() as u8);
    assert!(close(&converted[..4], &want, 3), "{converted:?} vs {want:?}");
    // To RGB: the colour shown on screen.
    let (w, blob, _) = first(json!({"output": {"conversion": "destination", "destination": SRGB}}));
    assert_eq!(w.space, "DeviceRGB");
    let shown = source.cmyk_to_srgb(ink.map(|v| v as f32 / 255.0), false).map(|v| (v * 255.0).round() as u8);
    let px = image::load_from_memory(&blob.bytes).unwrap().to_rgb8().get_pixel(0, 0).0;
    assert!(close(&px, &shown, 3), "{px:?} vs {shown:?}");
    // JPEG compression: a CMYK JPEG (Adobe's inverted samples, undone by the Decode array).
    let (w, blob, warnings) = first(json!({"compression": {"color": {"compression": "jpeg", "quality": "maximum"}}}));
    assert_eq!((w.space.as_str(), w.filter.as_deref()), ("DeviceCMYK", Some("DCTDecode")));
    assert_eq!(w.decode, [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(blob.cmyk().unwrap().data.as_chunks::<4>().0.iter().all(|p| close(p, &ink, 3)));
    // Resampled: fewer pixels, the same inks.
    let jpg = ImageBlob::new("image/jpeg", cmyk_jpeg(&flat(ink, 64 * 64), 64, 64));
    let d = image_doc(jpg, 64, 64);
    let s = json!({"compression": {"color": {"downsample": "bicubic", "ppi": 72.0, "abovePpi": 100.0}}});
    let out = export_with_report(&d, &opts(s)).unwrap();
    let w = written(&out.bytes);
    assert_eq!((w[0].space.as_str(), w[0].width), ("DeviceCMYK", 40), "{w:?}");
    assert!(blobs(&import(&out.bytes).unwrap())[0].cmyk().unwrap().data.as_chunks::<4>().0.iter().all(|p| close(p, &ink, 3)));
    // And PDF/X-1a keeps the numbers of a CMYK image (no conversion through RGB).
    let (w, blob, _) = first(json!({"standard": "pdfX1a", "output": {"destination": GENERIC_CMYK}}));
    assert_eq!(w.space, "DeviceCMYK");
    assert_eq!(blob.cmyk().unwrap().data, flat(ink, 16));
}

/// A version 2.1 output profile of CMYK colors with a one-tag table (`cprt`), its header and table
/// consistent with its size; `edit` changes it after it is made.
fn cmyk_profile(edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut icc = vec![0u8; 156];
    icc[0..4].copy_from_slice(&156u32.to_be_bytes());
    icc[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    icc[12..16].copy_from_slice(b"prtr");
    icc[16..20].copy_from_slice(b"CMYK");
    icc[20..24].copy_from_slice(b"Lab ");
    icc[36..40].copy_from_slice(b"acsp");
    icc[128..132].copy_from_slice(&1u32.to_be_bytes());
    icc[132..136].copy_from_slice(b"cprt");
    icc[136..140].copy_from_slice(&144u32.to_be_bytes());
    icc[140..144].copy_from_slice(&12u32.to_be_bytes());
    icc[144..156].copy_from_slice(b"text\0\0\0\0VC\0\0");
    edit(&mut icc);
    icc
}

/// `inks` (four bytes a pixel) as a CMYK JPEG carrying `icc` in its `ICC_PROFILE` segments.
fn cmyk_jpeg_with(inks: &[u8], w: u16, h: u16, icc: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = jpeg_encoder::Encoder::new(&mut out, 100);
    e.add_icc_profile(icc).unwrap();
    e.encode(inks, w, h, jpeg_encoder::ColorType::Cmyk).unwrap();
    out
}

/// A CMYK TIFF of `w` × `h` pixels of `ink` carrying `icc` as its tag 34675 (written as Open
/// stores a CMYK JPEG that carries one).
fn cmyk_tiff_with(ink: [u8; 4], w: u16, h: u16, icc: &[u8]) -> ImageBlob {
    let jpg = cmyk_jpeg_with(&flat(ink, usize::from(w) * usize::from(h)), w, h, icc);
    let (tiff, _) = vectorcraft_doc::cmyk::turned_cmyk(&jpg, image::metadata::Orientation::NoTransforms, None).unwrap();
    let blob = ImageBlob::new("image/tiff", tiff);
    assert_eq!(blob.cmyk_icc_profile().as_deref(), Some(icc), "the TIFF carries the profile");
    blob
}

/// A CMYK image whose numbers are kept is written in an ICC-based space of the CMYK profile it
/// carries, byte for byte, when the Profiles setting or the standard includes source profiles:
/// a TIFF written again, a JPEG kept as it is or compressed again, and an image with its mask as
/// its soft mask. Its inks are those it had. Without source profiles it is DeviceCMYK, and a
/// converted image never carries the profile its old numbers were in.
#[test]
fn a_cmyk_image_that_keeps_its_numbers_carries_its_cmyk_profile() {
    let ink = [0u8, 102, 255, 0];
    let icc = cmyk_profile(|_| {});
    let tiff = cmyk_tiff_with(ink, 4, 4, &icc);
    let tiff_inks = tiff.cmyk().unwrap().data;
    let d = image_doc(tiff, 4, 4);
    let export = |d: &Document, s: serde_json::Value| {
        let out = export_with_report(d, &opts(s.clone())).unwrap_or_else(|e| panic!("{s}: {e}"));
        let w: Vec<Written> = written(&out.bytes).into_iter().filter(|w| w.space != "DeviceGray").collect();
        assert_eq!(w.len(), 1, "{s}: {w:?}");
        (w.into_iter().next().unwrap(), out.bytes)
    };
    let tagged = |w: &Written, s: &serde_json::Value| {
        assert_eq!((w.space.as_str(), w.icc_n), ("ICCBased", Some(4)), "{s}");
        assert!(w.icc.as_deref() == Some(icc.as_slice()), "{s}: the image's own profile, byte for byte");
    };
    for s in [
        json!({"output": {"profiles": "all"}}),
        json!({"output": {"profiles": "taggedSource"}}),
        json!({"standard": "pdfX4"}),
        json!({"standard": "pdfX3"}),
        json!({"output": {"profiles": "all", "conversion": "preserveNumbers", "destination": DEVICE_CMYK}}),
    ] {
        let (w, bytes) = export(&d, s.clone());
        tagged(&w, &s);
        assert_eq!(blobs(&import(&bytes).unwrap())[0].cmyk().unwrap().data, tiff_inks, "{s}: the inks as they were");
    }
    // Without source profiles, untagged; PDF/X-1a images are always DeviceCMYK.
    for s in [json!({}), json!({"output": {"profiles": "destination"}}), json!({"standard": "pdfX1a", "output": {"profiles": "all"}})] {
        let (w, _) = export(&d, s.clone());
        assert!(w.icc.as_deref() != Some(icc.as_slice()), "{s}: {w:?}");
    }
    let (w, _) = export(&d, json!({"standard": "pdfX1a", "output": {"profiles": "all"}}));
    assert_eq!(w.space, "DeviceCMYK");
    // Converted to another CMYK profile: its new numbers aren't in the old profile.
    let (w, _) = export(&d, json!({"output": {"profiles": "all", "conversion": "destination", "destination": DEVICE_CMYK}}));
    assert!(w.icc.as_deref() != Some(icc.as_slice()), "{w:?}");

    // A CMYK JPEG kept as it is carries the profile only when source profiles are included, and
    // PDF/X-1a writes it untagged.
    let jpg = cmyk_jpeg_with(&flat(ink, 64 * 64), 64, 64, &icc);
    let d = image_doc(ImageBlob::new("image/jpeg", jpg.clone()), 64, 64);
    let s = json!({"output": {"profiles": "all"}});
    let (w, _) = export(&d, s.clone());
    tagged(&w, &s);
    assert!(w.data == jpg, "the JPEG data unchanged");
    for s in [json!({"standard": "pdfX1a"}), json!({})] {
        let (w, _) = export(&d, s.clone());
        assert_eq!((w.space.as_str(), w.filter.as_deref()), ("DeviceCMYK", Some("DCTDecode")), "{s}");
        assert!(w.data == jpg, "{s}: the JPEG data unchanged");
    }
    // Resampled and compressed again, it keeps the profile.
    let s = json!({"output": {"profiles": "all"}, "compression": {"color": {"downsample": "bicubic", "ppi": 72.0, "abovePpi": 100.0}}});
    let (w, bytes) = export(&d, s.clone());
    tagged(&w, &s);
    assert_eq!((w.width, w.filter.as_deref()), (40, Some("DCTDecode")), "{s}");
    assert!(blobs(&import(&bytes).unwrap())[0].cmyk().unwrap().data.as_chunks::<4>().0.iter().all(|p| close(p, &ink, 3)));

    // With its mask as its soft mask.
    let mut d = masked_doc(png(image::GrayImage::from_raw(2, 2, vec![255, 128, 0, 64]).unwrap()), true, false);
    d.images.insert("img".into(), cmyk_tiff_with(ink, 2, 2, &icc));
    let s = json!({"output": {"profiles": "all"}});
    let (w, bytes) = export(&d, s.clone());
    tagged(&w, &s);
    assert!(w.smask, "{w:?}");
    assert!(!String::from_utf8_lossy(&bytes).contains("/S/Luminosity"), "no luminosity soft mask");
}

/// A CMYK JPEG kept as it is whose header declares far more pixels than the app reads is left out
/// with a warning, as other images that large are.
#[test]
fn a_kept_cmyk_jpeg_declaring_too_many_pixels_is_left_out() {
    let mut jpg = cmyk_jpeg_with(&flat([0, 102, 255, 0], 4), 2, 2, &cmyk_profile(|_| {}));
    // Height and width of the SOF0 segment (after its marker, length and precision): 16384 × 16384.
    let sof = jpg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    jpg[sof + 5..sof + 9].copy_from_slice(&[0x40, 0, 0x40, 0]);
    assert_eq!(ImageBlob::new("image/jpeg", jpg.clone()).header_size(), Some((16384, 16384)));
    let d = image_doc(ImageBlob::new("image/jpeg", jpg), 2, 2);
    for s in [
        json!({"compression": {"color": {"downsample": "none"}}}),
        json!({"output": {"profiles": "all"}, "compression": {"color": {"downsample": "none"}}}),
    ] {
        let out = export_with_report(&d, &opts(s.clone())).unwrap();
        assert!(written(&out.bytes).is_empty(), "{s}");
        assert!(out.warnings.iter().any(|w| w.contains("couldn't be decoded")), "{s}: {:?}", out.warnings);
    }
}

/// The profile each transparency group of written PDF `bytes` blends in: its ICC-based space's
/// profile, or `None` for DeviceCMYK.
fn group_profiles(bytes: &[u8]) -> Vec<Option<Vec<u8>>> {
    let text = String::from_utf8_lossy(bytes);
    let file = Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let groups: Vec<&str> = text.split("/Group<<").skip(1).filter(|g| g.contains("/S/Transparency")).collect();
    assert!(!groups.is_empty(), "{text}");
    groups
        .into_iter()
        .map(|group| {
            let cs = group.split("/CS").nth(1).unwrap();
            if cs.starts_with("/DeviceCMYK") {
                return None;
            }
            let cs: i32 = cs.trim_start().split(' ').next().unwrap().parse().unwrap();
            let space = text.split(&format!("\n{cs} 0 obj\n")).nth(1).unwrap();
            let stream: i32 = space.strip_prefix("[/ICCBased ").unwrap().split(' ').next().unwrap().parse().unwrap();
            let profile = file.xref().get::<Stream<'_>>(craft_pdf::syntax::object::ObjectIdentifier::new(stream, 0)).unwrap();
            Some(profile.decoded().unwrap().into_owned())
        })
        .collect()
}

/// Transparency in a tagged CMYK document blends in the document's CMYK profile space, not in the
/// space of the profile a CMYK image carries, whichever the file holds first.
#[test]
fn tagged_cmyk_documents_blend_in_their_own_profile_beside_an_image_profile() {
    let icc = cmyk_profile(|_| {});
    let document = cms::icc_bytes(&cms::active_settings().cmyk).unwrap().to_vec();
    // A transparent CMYK rectangle over the image, and the image itself transparent.
    let mut d = image_doc(cmyk_tiff_with([0, 102, 255, 0], 4, 4, &icc), 4, 4);
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)),
        Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    n.opacity = 0.5;
    let l = d.layers[0].id;
    d.insert(Some(l), 1, n).unwrap();
    let mut alone = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    alone.images.insert("img".into(), cmyk_tiff_with([0, 102, 255, 0], 4, 4, &icc));
    let image =
        ImageObject { key: "img".into(), width: 4, height: 4, xf: Affine::scale(10.0), link: None, placement: Default::default(), pixels: None };
    let mut n = Node::new(alone.alloc_id(), NodeKind::Image(Box::new(image)));
    n.opacity = 0.5;
    let l = alone.layers[0].id;
    alone.insert(Some(l), 0, n).unwrap();
    for d in [d, alone] {
        let out = export_with_report(&d, &opts(json!({"output": {"profiles": "all"}}))).unwrap();
        let w = written(&out.bytes);
        assert!(w.iter().any(|w| w.icc.as_deref() == Some(icc.as_slice())), "the image's own profile: {w:?}");
        for profile in group_profiles(&out.bytes) {
            assert!(profile.as_ref().is_none_or(|p| *p == document), "the group blends in the document's CMYK profile or DeviceCMYK");
        }
    }
}

/// Only a well-formed CMYK profile tags an image: one whose size, signature, class, colour space
/// or tag table is wrong leaves it untagged.
#[test]
fn a_cmyk_image_with_a_malformed_profile_is_written_untagged() {
    let tag = |icc: Vec<u8>| crate::images::cmyk_icc(&ImageBlob::new("image/jpeg", cmyk_jpeg_with(&flat([0, 102, 255, 0], 4), 2, 2, &icc)));
    let good = cmyk_profile(|_| {});
    assert_eq!(tag(good.clone()), Some(good));
    let bad: [(&str, Vec<u8>); 7] = [
        ("declared size", cmyk_profile(|i| i[0..4].copy_from_slice(&200u32.to_be_bytes()))),
        ("signature", cmyk_profile(|i| i[36..40].copy_from_slice(b"xxxx"))),
        ("device link class", cmyk_profile(|i| i[12..16].copy_from_slice(b"link"))),
        ("RGB colour space", cmyk_profile(|i| i[16..20].copy_from_slice(b"RGB "))),
        ("tag beyond the end", cmyk_profile(|i| i[136..140].copy_from_slice(&150u32.to_be_bytes()))),
        ("table beyond the end", cmyk_profile(|i| i[128..132].copy_from_slice(&3u32.to_be_bytes()))),
        ("header only", cmyk_profile(|i| i.truncate(100))),
    ];
    for (what, icc) in bad {
        assert_eq!(tag(icc), None, "{what}");
    }
    // And the image is written untagged.
    let icc = cmyk_profile(|i| i[36..40].copy_from_slice(b"xxxx"));
    let d = image_doc(cmyk_tiff_with([0, 102, 255, 0], 4, 4, &icc), 4, 4);
    let out = export_with_report(&d, &opts(json!({"output": {"profiles": "taggedSource"}}))).unwrap();
    let w = written(&out.bytes);
    assert_eq!((w.len(), w[0].space.as_str()), (1, "DeviceCMYK"), "{w:?}");
}

/// A Press Quality export of a 2 × 2 CMYK image masked by PNG `mask` on its own pixel grid
/// (`clip`, `invert`): the file, and the decoded soft mask of each CMYK image (`None`: it has none).
fn export_masked(mask: Vec<u8>, clip: bool, invert: bool) -> (Vec<u8>, Vec<Option<Vec<u8>>>) {
    export_smasks(&masked_doc(mask, clip, invert))
}

/// A document of a 2 × 2 CMYK image masked by PNG `mask` (key `alpha`) on its own pixel grid.
fn masked_doc(mask: Vec<u8>, clip: bool, invert: bool) -> Document {
    let ink = [0u8, 102, 255, 0];
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    d.images.insert("img".into(), ImageBlob::cmyk_tiff(&Inks::new(2, 2, flat(ink, 4)).unwrap()).unwrap());
    d.images.insert("alpha".into(), ImageBlob::new("image/png", mask));
    let image = |key: &str| {
        NodeKind::Image(Box::new(ImageObject {
            key: key.into(),
            width: 2,
            height: 2,
            xf: Affine::scale(20.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        }))
    };
    let mut mask = vectorcraft_doc::OpacityMask::new(Node::new(d.alloc_id(), image("alpha")), clip);
    mask.invert = invert;
    let n = Node::new(d.alloc_id(), image("img")).with_mask(Some(Box::new(mask)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// A Press Quality export of `d`: the file, and the decoded soft mask of each CMYK image (`None`:
/// it has none).
fn export_smasks(d: &Document) -> (Vec<u8>, Vec<Option<Vec<u8>>>) {
    let press = builtin_presets().into_iter().find(|p| p.name == "Press Quality").unwrap().settings;
    let out = export_with_report(d, &PdfOptions { settings: press, created: Some(1_700_000_000), ..Default::default() }).unwrap();
    let file = Pdf::new(Arc::new(out.bytes.clone())).unwrap();
    let smasks = file
        .objects()
        .into_iter()
        .filter_map(|o| match o {
            Object::Stream(s) if s.dict().get::<Name<'_>>(b"ColorSpace").is_some_and(|n| n.as_ref() == b"DeviceCMYK") => Some(s),
            _ => None,
        })
        .map(|s| s.dict().get::<Stream<'_>>(b"SMask").map(|m| m.decoded().unwrap().into_owned()))
        .collect();
    (out.bytes, smasks)
}

fn png(img: impl Into<image::DynamicImage>) -> Vec<u8> {
    let mut out = Vec::new();
    img.into().write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A CMYK image whose opacity mask is a grey image on its own pixel grid (an image's soft mask
/// as it opens) is written with the mask as its `/SMask`, as an RGBA image is: CoreGraphics draws
/// CMYK images through luminosity soft masks on black boxes (#2159).
#[test]
fn a_cmyk_image_masked_by_a_grey_image_of_its_grid_writes_it_as_its_smask() {
    let (bytes, smasks) = export_masked(png(image::GrayImage::from_raw(2, 2, vec![255, 128, 0, 64]).unwrap()), true, false);
    assert_eq!(smasks, [Some(vec![255, 128, 0, 64])], "the CMYK image, its alpha the mask's greys");
    assert!(!String::from_utf8_lossy(&bytes).contains("/S/Luminosity"), "no luminosity soft mask");
    assert!(written(&bytes).iter().any(|w| w.space == "DeviceCMYK"), "the inks stay CMYK");
    // A mask that doesn't clip shows white where its image is transparent: lum · a + 255 · (1 − a).
    let la = image::GrayAlphaImage::from_raw(2, 2, vec![200, 128, 0, 0, 100, 255, 255, 64]).unwrap();
    let (_, smasks) = export_masked(png(la), false, false);
    assert_eq!(smasks, [Some(vec![227, 255, 100, 255])]);
    // An inverted mask stays a luminosity soft mask.
    let (bytes, smasks) = export_masked(png(image::GrayImage::from_raw(2, 2, vec![255, 128, 0, 64]).unwrap()), true, true);
    assert_eq!(smasks, [None]);
    assert!(String::from_utf8_lossy(&bytes).contains("/S/Luminosity"), "a luminosity soft mask");
}

/// A grey mask image with a color mask masks the CMYK image with the pixels it shows: its hidden
/// pixels are clear in the image's `/SMask`.
#[test]
fn a_color_masked_grid_mask_image_writes_the_pixels_it_shows_as_the_smask() {
    let mut d = masked_doc(png(image::GrayImage::from_raw(2, 2, vec![255, 128, 0, 64]).unwrap()), true, false);
    // The last pixel hidden.
    d.images.insert(
        "shown".into(),
        ImageBlob::new("image/png", png(image::GrayAlphaImage::from_raw(2, 2, vec![255, 255, 128, 255, 0, 255, 64, 0]).unwrap())),
    );
    let id = d.layers[0].children().unwrap()[0].id;
    let mut mask = d.node(id).unwrap().extras().mask.clone().unwrap();
    let art = std::sync::Arc::make_mut(&mut mask.art);
    art.extras_mut().color_mask = Some(Box::new(vectorcraft_doc::color_mask::ColorMask {
        key: "shown".into(),
        from: "alpha".into(),
        background: vectorcraft_doc::color_mask::MaskBackground::Colors,
        colors: vec![[64, 64, 64]],
        contiguous: false,
        measure: vectorcraft_doc::color_mask::MaskMatch::Distance,
        tolerance: 0.0,
        softness: 0.0,
    }));
    d.node_mut(id).unwrap().extras_mut().mask = Some(mask);
    let (_, smasks) = export_smasks(&d);
    assert_eq!(smasks, [Some(vec![255, 128, 0, 0])]);
}

/// A pixelated CMYK image (an envelope's with Anti-Alias off) masked by a grey image of its grid
/// is written without /Interpolate, as one without a mask is.
#[test]
fn a_pixelated_cmyk_image_with_its_grid_mask_asks_viewers_not_to_smooth_it() {
    for pixelated in [false, true] {
        let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
        d.images.insert("img".into(), ImageBlob::cmyk_tiff(&Inks::new(2, 2, flat([0u8, 102, 255, 0], 4)).unwrap()).unwrap());
        d.images.insert("alpha".into(), ImageBlob::new("image/png", png(image::GrayImage::from_raw(2, 2, vec![255, 128, 0, 64]).unwrap())));
        let image = |key: &str| {
            let im = ImageObject {
                key: key.into(),
                width: 2,
                height: 2,
                xf: Affine::scale(20.0),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            NodeKind::Image(Box::new(im))
        };
        let mask = vectorcraft_doc::OpacityMask::new(Node::new(d.alloc_id(), image("alpha")), true);
        let mut n = Node::new(d.alloc_id(), image("img")).with_mask(Some(Box::new(mask)));
        n.extras_mut().pixelated = pixelated;
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        let out = export_with_report(&d, &PdfOptions { created: Some(1_700_000_000), ..PdfOptions::uncompressed() }).unwrap().bytes;
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("/S/Luminosity"), "the mask is the image's /SMask");
        assert_eq!(text.contains("/Interpolate"), !pixelated, "pixelated: {pixelated}");
    }
}

/// A 100 × 100 pt document of `mode` whose square has a freeform fill of `a` on the left and `b`
/// on the right, with those points at `opacity`.
fn freeform_doc(mode: ColorMode, a: Color, b: Color, opacity: f32) -> Document {
    let p = |x, c| FreeformPoint { opacity, ..FreeformPoint::new(Point::new(x, 50.0), c) };
    let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
    g.freeform = Some(Freeform { points: vec![p(20.0, a), p(80.0, b)], ..Default::default() });
    let mut d = Document::new_with_mode(100.0, 100.0, mode);
    let square = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
        Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0),
    );
    let l = d.layers[0].id;
    d.insert(Some(l), 0, square).unwrap();
    d
}

#[test]
fn freeform_gradients_of_cmyk_points_are_written_as_cmyk_images() {
    let (cyan, black) = (Color::cmyk(1.0, 0.0, 0.0, 0.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
    for mode in [ColorMode::Cmyk, ColorMode::Rgb] {
        let d = freeform_doc(mode, cyan, black, 1.0);
        let out = export_with_report(&d, &opts(json!({}))).unwrap();
        let w = written(&out.bytes);
        assert_eq!(w.len(), 1, "{mode:?}: opaque, no soft mask: {w:?}");
        assert_eq!(w[0].space, "DeviceCMYK", "{mode:?}");
        let back = import(&out.bytes).unwrap();
        let inks = all_images(&back)[0].cmyk().unwrap();
        let at = (inks.height as usize / 2 * inks.width as usize + inks.width as usize / 2) * 4;
        assert!(close(&inks.data[at..at + 4], &[128, 0, 0, 128], 6), "{mode:?}: {:?}", &inks.data[at..at + 4]);
    }
    // Opacity goes in a soft mask, and the inks stay.
    let w = written(&export_with_report(&freeform_doc(ColorMode::Cmyk, cyan, black, 0.5), &opts(json!({}))).unwrap().bytes);
    assert_eq!(w.len(), 2, "the image and its soft mask: {w:?}");
    assert!(w.iter().any(|i| i.space == "DeviceCMYK") && w.iter().any(|i| i.space == "DeviceGray"), "{w:?}");
    // The Output settings convert the inks like any CMYK image's.
    let d = freeform_doc(ColorMode::Cmyk, cyan, black, 1.0);
    let w = written(&export_with_report(&d, &opts(json!({"output": {"conversion": "destination", "destination": SRGB}}))).unwrap().bytes);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_ne!(w[0].space, "DeviceCMYK", "converted to RGB");
}

/// A 100 × 100 pt document of `mode` whose square is filled with `g`, at one pixel a point.
fn gradient_doc(mode: ColorMode, g: Gradient) -> Document {
    let mut d = Document::new_with_mode(100.0, 100.0, mode);
    d.raster_effects_ppi = 72.0;
    let square = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
        Appearance::basic(Paint::Gradient(Box::new(GradientPaint::new(g))), Paint::None, 0.0),
    );
    let l = d.layers[0].id;
    d.insert(Some(l), 0, square).unwrap();
    d
}

/// A gradient of `stops`, dithered or Perceptual.
fn drawn_as_pixels(kind: GradientKind, stops: Vec<GradientStop>, dither: bool) -> Gradient {
    let method = if dither { GradientMethod::Classic } else { GradientMethod::Perceptual };
    Gradient { dither, method, ..Gradient::new(kind, stops) }
}

fn cyan_to_black() -> Vec<GradientStop> {
    vec![GradientStop::new(0.0, Color::cmyk(1.0, 0.0, 0.0, 0.0)), GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 1.0))]
}

#[test]
fn dithered_and_perceptual_gradients_of_cmyk_stops_are_written_as_cmyk_images() {
    for mode in [ColorMode::Cmyk, ColorMode::Rgb] {
        for kind in [GradientKind::Linear, GradientKind::Radial] {
            for dither in [true, false] {
                let d = gradient_doc(mode, drawn_as_pixels(kind, cyan_to_black(), dither));
                let out = export_with_report(&d, &opts(json!({}))).unwrap();
                let w = written(&out.bytes);
                assert_eq!(w.len(), 1, "{mode:?} {kind:?} dither {dither}: opaque, no soft mask: {w:?}");
                assert_eq!(w[0].space, "DeviceCMYK", "{mode:?} {kind:?} dither {dither}");
                let back = import(&out.bytes).unwrap();
                let inks = all_images(&back)[0].cmyk().unwrap();
                assert_eq!((inks.width, inks.height), (100, 100));
                assert!(inks.data.chunks(4).all(|p| p[1] == 0 && p[2] == 0), "{mode:?} {kind:?} dither {dither}: only cyan and black");
                if kind == GradientKind::Linear {
                    let row = &inks.data[50 * 400..51 * 400];
                    assert!(close(&row[..4], &[254, 0, 0, 1], 3), "{mode:?} dither {dither}: {:?}", &row[..4]);
                    assert!(close(&row[396..], &[1, 0, 0, 254], 3), "{mode:?} dither {dither}: {:?}", &row[396..]);
                    if dither {
                        assert!(close(&row[50 * 4..50 * 4 + 4], &[127, 0, 0, 128], 3), "{mode:?}: {:?}", &row[50 * 4..50 * 4 + 4]);
                    }
                }
            }
        }
    }
    // Opacity goes in a soft mask, and the inks stay.
    let mut stops = cyan_to_black();
    stops[1].opacity = 0.5;
    let w = written(
        &export_with_report(&gradient_doc(ColorMode::Cmyk, drawn_as_pixels(GradientKind::Linear, stops, true)), &opts(json!({}))).unwrap().bytes,
    );
    assert_eq!(w.len(), 2, "the image and its soft mask: {w:?}");
    assert!(w.iter().any(|i| i.space == "DeviceCMYK") && w.iter().any(|i| i.space == "DeviceGray"), "{w:?}");
    // The Output settings convert the inks like any CMYK image's.
    let d = gradient_doc(ColorMode::Cmyk, drawn_as_pixels(GradientKind::Linear, cyan_to_black(), true));
    let w = written(&export_with_report(&d, &opts(json!({"output": {"conversion": "destination", "destination": SRGB}}))).unwrap().bytes);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_ne!(w[0].space, "DeviceCMYK", "converted to RGB");
}

/// The colors between two stops mixed around the hue wheel are not mixes of the stops' inks: a
/// gradient of CMYK stops mixed that way is an RGB image, in a linear, radial, conic or diamond
/// gradient.
#[test]
fn hue_wheel_gradients_of_cmyk_stops_are_written_as_rgb_images() {
    for method in [GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise] {
        for mode in [ColorMode::Cmyk, ColorMode::Rgb] {
            for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Conic, GradientKind::Diamond] {
                let stops = vec![GradientStop::new(0.0, Color::cmyk(0.0, 1.0, 1.0, 0.0)), GradientStop::new(1.0, Color::cmyk(1.0, 1.0, 0.0, 0.0))];
                let d = gradient_doc(mode, Gradient { method, ..Gradient::new(kind, stops) });
                let out = export_with_report(&d, &opts(json!({}))).unwrap();
                let w = written(&out.bytes);
                assert_eq!(w.len(), 1, "{method:?} {mode:?} {kind:?}: one opaque image: {w:?}");
                assert_eq!(w[0].space, "DeviceRGB", "{method:?} {mode:?} {kind:?}");
                assert!(out.warnings.iter().any(|m| m.contains("exported as images")), "{method:?} {mode:?} {kind:?}: {:?}", out.warnings);
            }
        }
    }
}

/// A stop in another model, or linked to a spot swatch, keeps the RGB image.
#[test]
fn gradients_with_a_stop_that_is_not_process_cmyk_stay_rgb_images() {
    let cyan = || GradientStop::new(0.0, Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let others = [Color::rgb(1.0, 0.0, 0.0), Color::gray(0.5), Color::lab(50.0, 10.0, 10.0)];
    for mode in [ColorMode::Cmyk, ColorMode::Rgb] {
        for other in others {
            for dither in [true, false] {
                let d = gradient_doc(mode, drawn_as_pixels(GradientKind::Linear, vec![cyan(), GradientStop::new(1.0, other)], dither));
                let w = written(&export_with_report(&d, &opts(json!({}))).unwrap().bytes);
                assert_eq!(w.len(), 1, "{mode:?} {other:?}: {w:?}");
                assert_eq!(w[0].space, "DeviceRGB", "{mode:?} {other:?} dither {dither}");
            }
        }
    }
    // A stop of a spot swatch whose color is CMYK: a dithered fill is an RGB image with a warning.
    let gold = Color::cmyk(0.0, 0.2, 0.8, 0.1);
    let mut stop = GradientStop::new(1.0, gold);
    stop.swatch = Some("Gold".into());
    let mut d = gradient_doc(ColorMode::Cmyk, drawn_as_pixels(GradientKind::Linear, vec![cyan(), stop.clone()], true));
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(gold), global: true, spot: true });
    let out = export_with_report(&d, &opts(json!({}))).unwrap();
    let w = written(&out.bytes);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].space, "DeviceRGB");
    assert!(out.warnings.iter().any(|m| m.contains("written in process colors")), "{:?}", out.warnings);
    // A stop of a global process swatch is written as its CMYK color.
    let mut d = gradient_doc(ColorMode::Cmyk, drawn_as_pixels(GradientKind::Linear, vec![cyan(), stop], true));
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(gold), global: true, spot: false });
    let w = written(&export_with_report(&d, &opts(json!({}))).unwrap().bytes);
    assert_eq!(w[0].space, "DeviceCMYK", "{w:?}");
}

#[test]
fn freeform_gradients_of_lab_points_are_written_as_their_lab_mix() {
    let d = freeform_doc(ColorMode::Rgb, Color::lab(90.0, -60.0, 40.0), Color::lab(30.0, 60.0, -40.0), 1.0);
    let out = export_with_report(&d, &opts(json!({}))).unwrap();
    assert_eq!(written(&out.bytes)[0].space, "DeviceRGB");
    let back = import(&out.bytes).unwrap();
    let px = image::load_from_memory(&all_images(&back)[0].bytes).unwrap().to_rgba8();
    let got = px.get_pixel(px.width() / 2, px.height() / 2).0;
    let want = Color::lab(60.0, 0.0, 0.0).to_rgba8(1.0);
    assert!(close(&got, &want, 6), "{got:?} vs {want:?}");
}

#[test]
fn gray_freeform_points_are_written_as_black_ink_in_a_cmyk_document() {
    let (a, b) = (Color::gray(0.2), Color::gray(0.8));
    let out = export_with_report(&freeform_doc(ColorMode::Cmyk, a, b, 1.0), &opts(json!({}))).unwrap();
    let w = written(&out.bytes);
    assert_eq!(w[0].space, "DeviceCMYK", "{w:?}");
    let back = import(&out.bytes).unwrap();
    let inks = all_images(&back)[0].cmyk().unwrap();
    let at = (inks.height as usize / 2 * inks.width as usize + inks.width as usize / 2) * 4;
    assert!(close(&inks.data[at..at + 4], &[0, 0, 0, 128], 6), "{:?}", &inks.data[at..at + 4]);
    // An RGB document writes them as an image in RGB.
    let w = written(&export_with_report(&freeform_doc(ColorMode::Rgb, a, b, 1.0), &opts(json!({}))).unwrap().bytes);
    assert_ne!(w[0].space, "DeviceCMYK", "{w:?}");
}

/// #2950: Press Quality converts to the document's profile with its numbers kept, so in a CMYK
/// document an RGB image comes out CMYK, as the press needs; with no conversion (the default
/// preset) it stays RGB, as before.
#[test]
fn press_quality_writes_an_rgb_image_in_a_cmyk_document_as_cmyk() {
    let rgb = image::RgbImage::from_fn(4, 4, |x, y| image::Rgb([(x * 60) as u8, (y * 60) as u8, 128]));
    let blob = ImageBlob { mime: "image/png".into(), bytes: Arc::new(png(rgb)), proxy: None };
    let d = image_doc(blob, 4, 4);
    let press = builtin_presets().into_iter().find(|p| p.name == "Press Quality").unwrap().settings;
    assert_eq!(press.output.conversion, ColorConversion::PreserveNumbers);
    let options = PdfOptions { settings: press, created: Some(1_700_000_000), ..Default::default() };
    let w = written(&export_with_report(&d, &options).unwrap().bytes);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!((w[0].space.as_str(), w[0].icc_n), ("DeviceCMYK", None), "Press Quality");
    let w = written(&export_with_report(&d, &opts(json!({}))).unwrap().bytes);
    assert_eq!(w[0].space, "DeviceRGB", "no conversion");
}

/// #3012 review: in an RGB document Press Quality converts to Document CMYK too, the CMYK
/// profile in effect: a CMYK image keeps its inks (not converted to RGB), and an RGB image comes
/// out CMYK.
#[test]
fn press_quality_writes_cmyk_in_an_rgb_document_too() {
    let press = || PdfOptions { settings: builtin_preset("Press Quality").unwrap().settings, created: Some(1_700_000_000), ..Default::default() };
    assert_eq!(press().settings.output.destination, DOCUMENT_CMYK);
    let ink = [0u8, 102, 255, 0];
    let tiff = ImageBlob::cmyk_tiff(&Inks::new(4, 4, flat(ink, 16)).unwrap()).unwrap();
    let rgb = image::RgbImage::from_fn(4, 4, |x, y| image::Rgb([(x * 60) as u8, (y * 60) as u8, 128]));
    let png_blob = ImageBlob { mime: "image/png".into(), bytes: Arc::new(png(rgb)), proxy: None };
    for (what, blob) in [("CMYK image", tiff), ("RGB image", png_blob)] {
        let mut d = image_doc(blob, 4, 4);
        d.color_mode = ColorMode::Rgb;
        let out = export_with_report(&d, &press()).unwrap();
        let w = written(&out.bytes);
        assert_eq!(w.len(), 1, "{what}: {w:?}");
        assert_eq!(w[0].space, "DeviceCMYK", "{what} in an RGB document");
        // Preserve Numbers: the CMYK image comes back with the inks it had.
        if what == "CMYK image" {
            let back = import(&out.bytes).unwrap();
            assert_eq!(all_images(&back)[0].cmyk().unwrap().data, flat(ink, 16), "{what} inks");
        }
    }
}
