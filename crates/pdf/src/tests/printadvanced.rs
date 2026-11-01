//! File → Print › Advanced and Color Management: overprints preserved, discarded or simulated in
//! composite output, and colours converted to the printer profile with the rendering intent.

use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, ColorSettings, DEVICE_CMYK, GENERIC_CMYK, Intent};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

/// A 200 × 100 document with a rectangle filled with each `(colour, overprints)`, overlapping
/// one another left to right, in paint order.
fn doc(fills: &[(Color, bool)]) -> Document {
    let mut d = Document::new(200.0, 100.0);
    let l = d.layers[0].id;
    for (i, (c, over)) in fills.iter().enumerate() {
        let x = 10.0 + 30.0 * i as f64;
        let mut a = Appearance::basic(Paint::solid(*c), Paint::None, 0.0);
        for it in &mut a.items {
            *it.overprint_mut() = *over;
        }
        let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 10.0, x + 50.0, 60.0)), a);
        d.insert(Some(l), i, n).unwrap();
    }
    d
}

/// Cyan under a yellow that overprints.
fn overprinted() -> Document {
    doc(&[(Color::cmyk(1.0, 0.0, 0.0, 0.0), false), (Color::cmyk(0.0, 0.0, 1.0, 0.0), true)])
}

fn settings(v: Value) -> PrintSettings {
    serde_json::from_value(v).unwrap()
}

/// The job's bytes, content streams left readable.
fn job(d: &Document, v: Value) -> PrintReport {
    let opts = PrintOptions { settings: settings(v), created: Some(1_791_200_000), uncompressed: true, ..Default::default() };
    print(d, &opts).unwrap()
}

/// The fill colours page `page` sets: (operator, operands).
fn colours(pdf: &[u8], page: usize) -> Vec<(String, Vec<f32>)> {
    let file = Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let pages = file.pages();
    let content = String::from_utf8_lossy(pages.get(page).unwrap().page_stream().unwrap()).into_owned();
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

/// Does any fill or stroke of `d` overprint?
fn overprints(d: &Document) -> bool {
    d.layers.iter().any(|l| l.has_overprint())
}

#[test]
fn settings_saved_before_the_advanced_options_load_with_their_defaults() {
    let s = settings(json!({"copies": 2, "color": {"intent": "perceptual"}}));
    assert_eq!((s.copies, s.color.intent, s.color.preserve_numbers), (2, Intent::Perceptual, true));
    assert_eq!(s.advanced, PrintAdvanced::default());
    assert_eq!((s.advanced.print_as_bitmap, s.advanced.overprints, s.advanced.flattener_preset.as_str()), (false, PrintOverprints::Preserve, ""));
    assert_eq!(s.color.profile, "");
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["advanced"], json!({"printAsBitmap": false, "overprints": "preserve", "flattenerPreset": ""}));
    // A printer profile must be one there is.
    assert!(settings(json!({"color": {"profile": GENERIC_CMYK}})).check().is_ok());
    assert!(matches!(settings(json!({"color": {"profile": "No Such Profile"}})).check(), Err(PdfError::BadSetting(_))));
}

#[test]
fn discarded_and_simulated_overprints_write_no_overprint() {
    let d = overprinted();
    let preserve = job(&d, json!({}));
    assert!(preserve.warnings.is_empty(), "{:?}", preserve.warnings);
    assert!(String::from_utf8_lossy(&preserve.bytes).contains("/OP true/op true/OPM 1"), "preserved overprints overprint in the file");
    assert!(overprints(&printed_document(&d, &settings(json!({})))), "preserved");
    for (how, darkens) in [("discard", false), ("simulate", true)] {
        let set = json!({"advanced": {"overprints": how}});
        assert!(!overprints(&printed_document(&d, &settings(set.clone()))), "{how}: nothing overprints");
        let r = job(&d, set);
        let text = String::from_utf8_lossy(&r.bytes);
        assert!(!text.contains("/OP"), "{how}: no overprint in the file");
        assert_eq!(text.contains("/Darken"), darkens, "{how}");
        assert!(!r.warnings.iter().any(|w| w.contains("knockouts")), "{how}: {:?}", r.warnings);
    }
    // Separations honour overprints whatever the option says.
    let seps = settings(json!({"output": {"mode": "separations"}, "advanced": {"overprints": "discard"}}));
    assert!(overprints(&printed_document(&d, &seps)));
}

#[test]
fn a_cmyk_printer_profile_writes_device_cmyk_with_the_intent() {
    let d = doc(&[(Color::rgb(1.0, 0.0, 0.0), false)]);
    assert_eq!(colours(&job(&d, json!({})).bytes, 0), [("rg".to_string(), vec![1.0, 0.0, 0.0])], "as it is without a profile");
    let to_cmyk = Cms::new(&ColorSettings { cmyk: GENERIC_CMYK.into(), ..cms::active_settings() }).unwrap();
    for intent in [Intent::Perceptual, Intent::Saturation] {
        let c = colours(&job(&d, json!({"color": {"profile": GENERIC_CMYK, "intent": intent}})).bytes, 0);
        let ink = q8(&to_cmyk.srgb_to_cmyk([1.0, 0.0, 0.0], intent));
        assert!(c.len() == 1 && c[0].0 == "k" && close(&c[0].1, &ink), "{intent:?}: {c:?} vs {ink:?}");
    }
    // CMYK colours keep their numbers with Preserve CMYK Numbers.
    let d = doc(&[(Color::cmyk(0.1, 0.2, 0.3, 0.4), false)]);
    let c = colours(&job(&d, json!({"color": {"profile": DEVICE_CMYK, "preserveNumbers": true}})).bytes, 0);
    assert!(c[0].0 == "k" && close(&c[0].1, &q8(&[0.1, 0.2, 0.3, 0.4])), "{c:?}");
}

#[test]
fn separations_separate_with_a_cmyk_printer_profile() {
    // A mid grey: naive device CMYK puts it all on the black plate; a press profile doesn't.
    let d = doc(&[(Color::rgb(0.5, 0.5, 0.5), false)]);
    let cyan = |profile: &str| {
        let r = job(&d, json!({"output": {"mode": "separations"}, "color": {"profile": profile}}));
        colours(&r.bytes, 0)
    };
    let device = cyan(DEVICE_CMYK);
    let press = cyan(GENERIC_CMYK);
    assert_ne!(device, press, "the profile separates");
    // An RGB printer profile doesn't separate.
    let r = job(&d, json!({"output": {"mode": "separations"}, "color": {"profile": cms::SRGB}}));
    assert!(r.warnings.iter().any(|w| w.contains("RGB printer profile")), "{:?}", r.warnings);
}
