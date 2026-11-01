//! Save PDF › Output: colours converted to the destination (or their numbers preserved), ICC
//! profiles tagging them, images converted too, and the output intent and Trapped entries.

use std::io::Cursor;
use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Dict, Name, ObjectIdentifier, Stream};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ColorSettings, DEVICE_CMYK, GENERIC_CMYK, SRGB};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ColorMode, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::*;

/// A document of `mode` with a rectangle filled with each of `fills`, side by side.
fn doc(mode: ColorMode, fills: &[Color]) -> Document {
    let mut d = Document::new(200.0, 100.0);
    d.color_mode = mode;
    let l = d.layers[0].id;
    for (i, c) in fills.iter().enumerate() {
        let x = 10.0 + 30.0 * i as f64;
        let n =
            Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0)), Appearance::basic(Paint::solid(*c), Paint::None, 0.0));
        d.insert(Some(l), i, n).unwrap();
    }
    d
}

/// The export of `d` with `settings` (content streams left readable).
fn pdf(d: &Document, settings: Value) -> ExportReport {
    let mut settings: PdfSettings = serde_json::from_value(settings).unwrap();
    settings.compression.compress_text = false;
    export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap()
}

/// The fill colours the first page's content sets: (operator, operands).
fn colours(pdf: &[u8]) -> Vec<(String, Vec<f32>)> {
    let file = Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let pages = file.pages();
    let content = String::from_utf8_lossy(pages.iter().next().unwrap().page_stream().unwrap()).into_owned();
    let (mut nums, mut out) = (vec![], vec![]);
    for t in content.split_whitespace() {
        if let Ok(v) = t.parse::<f32>() {
            nums.push(v);
            continue;
        }
        if ["rg", "k", "g", "scn"].contains(&t) {
            out.push((t.to_string(), nums.clone()));
        }
        nums.clear();
    }
    out
}

/// `v` as the writer stores it (8 bits a component).
fn q8(v: &[f32]) -> Vec<f32> {
    v.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0).collect()
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

/// The colour management of a CMYK destination, as the export builds it.
fn to_cmyk(dest: &str) -> Cms {
    Cms::new(&ColorSettings { cmyk: dest.into(), ..cms::active_settings() }).unwrap()
}

/// The ICC profiles the ICC-based colour spaces of `pdf` use: (components, profile bytes).
fn icc_profiles(pdf: &[u8]) -> Vec<(i32, Vec<u8>)> {
    let file = Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let text = String::from_utf8_lossy(pdf);
    let mut out = vec![];
    for (at, _) in text.match_indices("[/ICCBased ") {
        let n: i32 = text[at + 11..].split(' ').next().unwrap().parse().unwrap();
        let s = file.xref().get::<Stream<'_>>(ObjectIdentifier::new(n, 0)).unwrap();
        let entry = (s.dict().get::<i32>(b"N").unwrap(), s.decoded().unwrap().into_owned());
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out
}

#[test]
fn converting_to_a_cmyk_destination_writes_k_instead_of_rg() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let d = doc(ColorMode::Rgb, &[red]);
    assert_eq!(colours(&pdf(&d, json!({})).bytes), [("rg".to_string(), vec![1.0, 0.0, 0.0])], "without conversion");
    let ink = q8(&to_cmyk(GENERIC_CMYK).srgb_to_cmyk([1.0, 0.0, 0.0], cms::active_settings().intent));
    for conversion in ["destination", "preserveNumbers"] {
        let r = pdf(&d, json!({"output": {"conversion": conversion, "destination": GENERIC_CMYK}}));
        let c = colours(&r.bytes);
        assert!(c.len() == 1 && c[0].0 == "k" && close(&c[0].1, &ink), "{conversion}: {c:?} vs {ink:?}");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }
    // A blank destination is the document's profile: the RGB document's colours stay RGB.
    assert_eq!(colours(&pdf(&d, json!({"output": {"conversion": "destination"}})).bytes)[0].0, "rg");
}

#[test]
fn preserve_numbers_keeps_cmyk_values_that_destination_converts() {
    let ink = Color::cmyk(0.1, 0.2, 0.3, 0.4);
    let d = doc(ColorMode::Rgb, &[ink, Color::Gray { k: 0.25 }]);
    let numbers = q8(&[0.1, 0.2, 0.3, 0.4]);
    let fills = |conversion: &str, dest: &str| colours(&pdf(&d, json!({"output": {"conversion": conversion, "destination": dest}})).bytes);
    let kept = fills("preserveNumbers", DEVICE_CMYK);
    assert!(kept[0].0 == "k" && close(&kept[0].1, &numbers), "{kept:?}");
    // Grey stays grey.
    assert!(kept[1].0 == "g" && close(&kept[1].1, &q8(&[0.75])), "{kept:?}");
    // Convert to destination: CMYK of another profile goes through Lab into the destination.
    let converted = fills("destination", DEVICE_CMYK);
    let expected = q8(&to_cmyk(DEVICE_CMYK).lab_to_cmyk(cms::active().lab(&ink), cms::active_settings().intent));
    assert!(converted[0].0 == "k" && close(&converted[0].1, &expected), "{converted:?} vs {expected:?}");
    assert!(converted[0].1.iter().zip(&numbers).any(|(a, b)| (a - b).abs() > 0.02), "the numbers change: {converted:?}");
    // …unless the destination is their own profile.
    let own = fills("destination", &cms::active_settings().cmyk);
    assert!(close(&own[0].1, &numbers), "{own:?}");
}

#[test]
fn converting_to_rgb_turns_cmyk_into_rg() {
    let ink = Color::cmyk(0.0, 1.0, 1.0, 0.0);
    let d = doc(ColorMode::Cmyk, &[ink]);
    assert_eq!(colours(&pdf(&d, json!({})).bytes)[0].0, "k");
    let c = colours(&pdf(&d, json!({"output": {"conversion": "preserveNumbers", "destination": SRGB}})).bytes);
    assert!(c[0].0 == "rg" && close(&c[0].1, &q8(&cms::active().display_rgb(&ink))), "{c:?}");
}

#[test]
fn an_unknown_destination_is_refused() {
    let d = doc(ColorMode::Rgb, &[Color::BLACK]);
    let set: PdfSettings = serde_json::from_value(json!({"output": {"conversion": "destination", "destination": "No Such Press"}})).unwrap();
    assert!(matches!(set.check(), Err(PdfError::BadSetting(m)) if m.contains("No Such Press")));
    assert!(export(&d, &PdfOptions { settings: set, ..Default::default() }).is_err());
    // Not converting, the destination doesn't matter.
    let set: PdfSettings = serde_json::from_value(json!({"output": {"destination": "No Such Press"}})).unwrap();
    assert_eq!(set.check(), Ok(()));
}

#[test]
fn profiles_tag_colours_with_icc_based_spaces() {
    let d = doc(ColorMode::Rgb, &[Color::rgb(0.2, 0.4, 0.6), Color::cmyk(0.1, 0.2, 0.3, 0.4), Color::Gray { k: 0.5 }]);
    let none = pdf(&d, json!({}));
    assert!(icc_profiles(&none.bytes).is_empty());
    assert!(colours(&none.bytes).iter().all(|c| c.0 != "scn"));
    let all = pdf(&d, json!({"output": {"profiles": "all"}}));
    let profiles = icc_profiles(&all.bytes);
    let mut n: Vec<i32> = profiles.iter().map(|p| p.0).collect();
    n.sort();
    assert_eq!(n, [1, 3, 4], "grey, RGB and CMYK spaces");
    assert!(profiles.iter().any(|p| p.0 == 4 && p.1 == *cms::icc_bytes(&cms::active_settings().cmyk).unwrap()), "the working CMYK profile");
    assert!(colours(&all.bytes).iter().all(|c| c.0 == "scn"), "{:?}", colours(&all.bytes));
    // The destination profile tags converted colours.
    let dest = pdf(&d, json!({"output": {"conversion": "destination", "destination": DEVICE_CMYK, "profiles": "destination"}}));
    let profiles = icc_profiles(&dest.bytes);
    assert!(profiles.iter().any(|p| p.0 == 4 && p.1 == *cms::icc_bytes(DEVICE_CMYK).unwrap()), "the destination");
    // Tagged source profiles: only a document that was assigned profiles is tagged.
    assert!(icc_profiles(&pdf(&d, json!({"output": {"profiles": "taggedSource"}})).bytes).is_empty());
    let mut tagged = d.clone();
    tagged.color_profiles.cmyk = Some(GENERIC_CMYK.into());
    assert_eq!(icc_profiles(&pdf(&tagged, json!({"output": {"profiles": "taggedSource"}})).bytes).len(), 3);
}

#[test]
fn tagged_cmyk_documents_blend_in_the_cmyk_profile_space() {
    let mut d = doc(ColorMode::Cmyk, &[Color::cmyk(1.0, 0.0, 0.0, 0.0)]);
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)),
        Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    n.opacity = 0.5;
    let l = d.layers[0].id;
    d.insert(Some(l), 1, n).unwrap();
    let r = pdf(&d, json!({"output": {"profiles": "all"}}));
    let text = String::from_utf8_lossy(&r.bytes);
    let group = text.split("/Group<<").nth(1).unwrap();
    let cs: i32 = group.split("/CS ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
    let space = text.split(&format!("\n{cs} 0 obj\n")).nth(1).unwrap();
    let stream: i32 = space.strip_prefix("[/ICCBased ").unwrap().split(' ').next().unwrap().parse().unwrap();
    let file = Pdf::new(Arc::new(r.bytes.clone())).unwrap();
    assert_eq!(file.xref().get::<Stream<'_>>(ObjectIdentifier::new(stream, 0)).unwrap().dict().get::<i32>(b"N"), Some(4), "{group}");
}

/// A 4 × 4 PNG of one colour.
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::from(image::RgbImage::from_pixel(4, 4, image::Rgb(rgb)))
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn image_doc(bytes: Vec<u8>) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("img".into(), ImageBlob::new("image/png", bytes));
    let image =
        ImageObject { key: "img".into(), width: 4, height: 4, xf: Affine::scale(10.0), link: None, placement: Default::default(), pixels: None };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(image)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The colour of the first image of the PDF `bytes` imports as, at its first pixel.
fn first_pixel(bytes: &[u8]) -> [u8; 3] {
    let back = import(bytes).unwrap();
    let blob = back.images.values().next().unwrap();
    image::load_from_memory(&blob.bytes).unwrap().to_rgb8().get_pixel(0, 0).0
}

#[test]
fn images_are_converted_too() {
    let d = image_doc(png([200, 40, 30]));
    let plain = pdf(&d, json!({}));
    assert!(String::from_utf8_lossy(&plain.bytes).contains("/ColorSpace/DeviceRGB"));
    let r = pdf(&d, json!({"output": {"conversion": "preserveNumbers", "destination": GENERIC_CMYK}}));
    assert!(String::from_utf8_lossy(&r.bytes).contains("/ColorSpace/DeviceCMYK"), "a CMYK image");
    // Converted to CMYK and compressed with JPEG: a CMYK JPEG.
    let jpeg =
        pdf(&d, json!({"compression": {"color": {"compression": "jpeg"}}, "output": {"conversion": "destination", "destination": GENERIC_CMYK}}));
    let t = String::from_utf8_lossy(&jpeg.bytes);
    assert!(t.contains("/DCTDecode") && t.contains("/ColorSpace/DeviceCMYK"), "{t}");
    assert!(jpeg.warnings.is_empty(), "{:?}", jpeg.warnings);
    // An RGB destination other than sRGB changes the pixels (unless their numbers are kept).
    assert_eq!(first_pixel(&plain.bytes), [200, 40, 30]);
    let p3 = |conversion: &str| first_pixel(&pdf(&d, json!({"output": {"conversion": conversion, "destination": cms::DISPLAY_P3}})).bytes);
    assert_ne!(p3("destination"), [200, 40, 30]);
    assert_eq!(p3("preserveNumbers"), [200, 40, 30]);
    // Grey images stay grey.
    let grey = image_doc(png([90, 90, 90]));
    let r = pdf(&grey, json!({"output": {"conversion": "destination", "destination": GENERIC_CMYK}}));
    assert!(String::from_utf8_lossy(&r.bytes).contains("/ColorSpace/DeviceGray"));
}

#[test]
fn printers_marks_keep_their_inks() {
    let d = doc(ColorMode::Cmyk, &[Color::cmyk(0.0, 0.0, 1.0, 0.0)]);
    let r = pdf(&d, json!({"marks": {"colorBars": true}, "output": {"conversion": "destination", "destination": SRGB}}));
    let ops: Vec<String> = colours(&r.bytes).into_iter().map(|c| c.0).collect();
    assert!(ops.contains(&"rg".to_string()) && ops.contains(&"k".to_string()), "art converted, colour bars kept: {ops:?}");
}

/// The catalog's output intent of `bytes` (opened with `password`).
fn output_intent(bytes: &[u8], password: Option<&str>, f: impl FnOnce(&Dict<'_>)) {
    let file = crate::pages::open(bytes, password).unwrap();
    let xref = file.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap();
    let intents = catalog.get::<Array<'_>>(b"OutputIntents").unwrap();
    f(&intents.iter::<Dict<'_>>().next().unwrap());
}

#[test]
fn the_output_intent_embeds_its_profile_and_trapped_is_written() {
    let mut d = doc(ColorMode::Cmyk, &[Color::cmyk(0.0, 0.0, 1.0, 0.0)]);
    let out = json!({"outputIntent": GENERIC_CMYK, "outputCondition": "Coated (press)", "outputConditionId": "Généric", "registry": "http://example.com", "trapped": true});
    for (title, password) in [("Poster", None), ("", None), ("Poster", Some("secret"))] {
        d.title = title.into();
        let mut set = json!({"output": out});
        if let Some(p) = password {
            set["security"] = json!({"openPassword": p});
        }
        let r = pdf(&d, set);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        output_intent(&r.bytes, password, |oi| {
            assert_eq!(oi.get::<Name<'_>>(b"S").unwrap().as_ref(), b"GTS_PDFX");
            let text = |k: &[u8]| oi.get::<craft_pdf::syntax::object::String<'_>>(k).map(|s| s.as_bytes().to_vec());
            assert_eq!(text(b"OutputCondition").unwrap(), b"Coated (press)");
            assert_eq!(text(b"RegistryName").unwrap(), b"http://example.com");
            // Non-ASCII text is UTF-16 with its byte order mark.
            let id: Vec<u8> = [0xFE, 0xFF].into_iter().chain("Généric".encode_utf16().flat_map(u16::to_be_bytes)).collect();
            assert_eq!(text(b"OutputConditionIdentifier").unwrap(), id);
            let profile = oi.get::<Stream<'_>>(b"DestOutputProfile").unwrap();
            assert_eq!(profile.dict().get::<i32>(b"N"), Some(4));
            assert_eq!(profile.decoded().unwrap().as_ref(), &*cms::icc_bytes(GENERIC_CMYK).unwrap());
        });
        if password.is_none() {
            assert!(String::from_utf8_lossy(&r.bytes).contains("/Trapped/True"), "title {title:?}");
        }
        assert_eq!(
            import_with_report(&r.bytes, &ImportOptions { password: password.map(str::to_string), ..Default::default() })
                .unwrap()
                .document
                .artboards
                .len(),
            1
        );
    }
    // A condition identifier alone (a registered printing condition) names no profile; untrapped
    // output says so.
    let r = pdf(&d, json!({"output": {"outputConditionId": "CGATS TR 001"}}));
    output_intent(&r.bytes, None, |oi| assert!(oi.get::<Stream<'_>>(b"DestOutputProfile").is_none()));
    assert!(String::from_utf8_lossy(&r.bytes).contains("/Trapped/False"));
    // Nothing asked, nothing written.
    let plain = String::from_utf8_lossy(&pdf(&d, json!({})).bytes).into_owned();
    assert!(!plain.contains("/OutputIntents") && !plain.contains("/Trapped"));
}

#[test]
fn pdf_a_keeps_its_own_output_intent() {
    let d = doc(ColorMode::Rgb, &[Color::BLACK]);
    let r = pdf(&d, json!({"standard": "pdfA2b", "output": {"outputIntent": GENERIC_CMYK, "trapped": true}}));
    let text = String::from_utf8_lossy(&r.bytes);
    assert!(text.contains("GTS_PDFA1") && !text.contains("GTS_PDFX") && !text.contains("/Trapped"));
    assert!(r.warnings.iter().any(|w| w.contains("PDF/A")), "{:?}", r.warnings);
}

#[test]
fn pdf_a_tags_cmyk_colours_with_the_cmyk_profile() {
    let d = doc(ColorMode::Cmyk, &[Color::cmyk(0.1, 0.2, 0.3, 0.4)]);
    let r = pdf(&d, json!({"standard": "pdfA2b"}));
    assert!(icc_profiles(&r.bytes).iter().any(|p| p.0 == 4));
}

/// The streams of `bytes` that hold an ICC profile (`acsp` at offset 36), decoded.
fn icc_streams(bytes: &[u8]) -> Vec<Vec<u8>> {
    let file = Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let count = crate::patch::Xref::read(bytes).unwrap().count() as i32;
    (1..count)
        .filter_map(|n| file.xref().get::<Stream<'_>>(ObjectIdentifier::new(n, 0)))
        .map(|s| s.decoded().unwrap().into_owned())
        .filter(|data| data.get(36..40) == Some(b"acsp"))
        .collect()
}

/// [`pdf`] with the version and the creation date the standard of `settings` asks for.
fn standard_pdf(d: &Document, settings: Value) -> ExportReport {
    let mut settings: PdfSettings = serde_json::from_value(settings).unwrap();
    (settings.compatibility, settings.compression.compress_text) = (settings.standard.version(), false);
    export_with_report(d, &PdfOptions { settings, created: Some(1_700_000_000), ..Default::default() }).unwrap()
}

#[test]
fn a_profile_that_tags_colours_and_is_the_output_intent_is_embedded_once() {
    let d = doc(ColorMode::Cmyk, &[Color::cmyk(0.1, 0.2, 0.3, 0.4)]);
    let working = cms::active_settings().cmyk;
    let icc = cms::icc_bytes(&working).unwrap();
    for set in [
        json!({"standard": "pdfX4"}),
        json!({"standard": "pdfX4", "output": {"outputIntent": working}}),
        // PDF/X-3 and PDF/X-4 always tag their colours: `profiles` doesn't apply.
        json!({"standard": "pdfX4", "output": {"profiles": "none", "outputIntent": working}}),
        json!({"standard": "pdfX3"}),
        json!({"output": {"profiles": "all", "outputIntent": working}}),
    ] {
        let r = standard_pdf(&d, set.clone());
        let streams = icc_streams(&r.bytes);
        assert_eq!(streams.len(), 1, "{set}: ICC profile streams");
        assert!(streams[0] == *icc, "{set}: the working CMYK profile");
        assert!(colours(&r.bytes).iter().all(|c| c.0 == "scn"), "{set}: tagged colours");
        output_intent(&r.bytes, None, |oi| assert_eq!(oi.get::<Stream<'_>>(b"DestOutputProfile").unwrap().decoded().unwrap().as_ref(), &*icc));
    }
    // Another output intent profile is its own stream.
    let r = standard_pdf(&d, json!({"standard": "pdfX4", "output": {"outputIntent": DEVICE_CMYK}}));
    assert_eq!(icc_streams(&r.bytes).len(), 2);
    output_intent(&r.bytes, None, |oi| {
        assert_eq!(oi.get::<Stream<'_>>(b"DestOutputProfile").unwrap().decoded().unwrap().as_ref(), &*cms::icc_bytes(DEVICE_CMYK).unwrap())
    });
}

/// The profiles assigned to a document (Edit › Assign Profile) are its colour spaces whatever the
/// working spaces of the session that exports it: a document opened again in a new session,
/// which starts with the default working spaces, keeps its profiles. Its CMYK colours are tagged
/// with its CMYK profile, which is also the PDF/X output intent, and its RGB colours separate
/// through it.
#[test]
fn colours_are_in_the_profiles_assigned_to_the_document() {
    let working = cms::active_settings().cmyk;
    assert_ne!(working, DEVICE_CMYK, "the document is assigned another profile than the working one");
    let intent = cms::active_settings().intent;
    let grey = Color::rgb(0.5, 0.5, 0.5);
    let mut d = doc(ColorMode::Cmyk, &[Color::cmyk(0.1, 0.2, 0.3, 0.4), grey]);
    d.color_profiles.cmyk = Some(DEVICE_CMYK.into());
    let r = standard_pdf(&d, json!({"standard": "pdfX4"}));
    assert!(!r.warnings.iter().any(|w| w.contains("profile")), "{:?}", r.warnings);
    let own = cms::icc_bytes(DEVICE_CMYK).unwrap();
    assert!(icc_streams(&r.bytes) == [own.to_vec()], "tagged with the document's CMYK profile, not the working one");
    output_intent(&r.bytes, None, |oi| assert_eq!(oi.get::<Stream<'_>>(b"DestOutputProfile").unwrap().decoded().unwrap().as_ref(), &*own));
    let c = colours(&r.bytes);
    assert!(c[0].0 == "scn" && close(&c[0].1, &q8(&[0.1, 0.2, 0.3, 0.4])), "CMYK keeps its numbers: {c:?}");
    let separated = q8(&to_cmyk(DEVICE_CMYK).to_cmyk(&grey, intent));
    assert!(!close(&separated, &q8(&cms::active().to_cmyk(&grey, intent))), "the two profiles separate grey differently");
    assert!(c[1].0 == "scn" && close(&c[1].1, &separated), "RGB separated through the document's profile: {c:?} vs {separated:?}");
    // A profile that isn't available leaves the working space, with a warning.
    d.color_profiles.cmyk = Some("No Such Press".into());
    let r = standard_pdf(&d, json!({"standard": "pdfX4"}));
    assert!(r.warnings.iter().any(|w| w.contains("No Such Press")), "{:?}", r.warnings);
    assert!(icc_streams(&r.bytes) == [cms::icc_bytes(&working).unwrap().to_vec()]);
}
