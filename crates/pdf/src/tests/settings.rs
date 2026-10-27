//! Save PDF settings: JSON shape, checks and the warnings for options not applied yet.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn settings(v: serde_json::Value) -> PdfSettings {
    serde_json::from_value(v).unwrap()
}

fn doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.insert(Some(layer), 0, n).unwrap();
    d
}

#[test]
fn json_is_camel_case_and_partial_objects_keep_defaults() {
    let d = PdfSettings::default();
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["compatibility"], "1.7");
    assert_eq!(v["standard"], "none");
    assert_eq!(v["compression"]["compressText"], true);
    assert_eq!(v["compression"]["color"]["abovePpi"], 450.0);
    assert_eq!(v["advanced"]["overprint"], "preserve");
    assert_eq!(serde_json::from_value::<PdfSettings>(v).unwrap(), d, "round-trips");
    let s = settings(json!({"compatibility": "1.5", "marks": {"trim": true}, "compression": {"gray": {"downsample": "bicubic"}}}));
    assert_eq!(s.compatibility, Compatibility::Pdf15);
    assert!(s.marks.trim && s.marks.weight == 0.25);
    assert_eq!(s.compression.gray.downsample, Downsample::Bicubic);
    assert_eq!(s.compression.gray.ppi, 300.0);
    assert!(s.compression.compress_text);
    for bad in [json!({"compatibility": "1.2"}), json!({"standard": "pdfX9"}), json!({"marks": {"trim": "yes"}})] {
        assert!(serde_json::from_value::<PdfSettings>(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn passwords_are_never_serialized() {
    let s = settings(json!({"security": {"openPassword": "secret", "permissionsPassword": "owner"}}));
    assert_eq!(s.security.open_password, "secret");
    let text = serde_json::to_string(&s).unwrap();
    assert!(!text.contains("secret") && !text.contains("owner") && !text.contains("Password"), "{text}");
}

#[test]
fn checks_refuse_what_the_writer_cant_honour() {
    assert_eq!(PdfSettings::default().check(), Ok(()));
    let refused = |v: serde_json::Value| settings(v).check().unwrap_err();
    // PDF/X-4 is a PDF 1.6 standard: the default 1.7 is refused.
    assert!(matches!(refused(json!({"standard": "pdfX4"})), PdfError::BadSetting(m) if m.contains("PDF 1.7")));
    assert_eq!(settings(json!({"standard": "pdfX4", "compatibility": "1.6"})).check(), Ok(()));
    // Passwords are written (the file is encrypted); a password with a standard is not.
    assert_eq!(settings(json!({"security": {"openPassword": "x"}})).check(), Ok(()));
    assert_eq!(settings(json!({"security": {"permissionsPassword": "x"}})).check(), Ok(()));
    assert!(matches!(refused(json!({"standard": "pdfA2b", "security": {"openPassword": "x"}})), PdfError::BadSetting(_)));
    assert!(matches!(refused(json!({"compression": {"color": {"ppi": 5}}})), PdfError::BadSetting(m) if m.contains("compression.color.ppi")));
    assert!(matches!(refused(json!({"bleed": {"left": 100}})), PdfError::BadSetting(m) if m.contains("bleed.left")));
    assert!(matches!(refused(json!({"marks": {"weight": 0}})), PdfError::BadSetting(_)));
    assert!(matches!(refused(json!({"advanced": {"fontSubsetPercent": 120}})), PdfError::BadSetting(_)));
    // PDF/A-2 is a PDF 1.7 standard: earlier versions are fine, 2.0 is not.
    assert!(matches!(refused(json!({"standard": "pdfA2b", "compatibility": "2.0"})), PdfError::BadSetting(m) if m.contains("PDF/A-2b")));
    assert_eq!(settings(json!({"standard": "pdfA2b", "compatibility": "1.4"})).check(), Ok(()));
    assert!(Compatibility::ALL.iter().all(|c| Standard::None.allows(*c)));
    assert!(export(&doc(), &PdfOptions { settings: settings(json!({"standard": "pdfX1a"})), ..Default::default() }).is_err());
    assert!(export(&doc(), &PdfOptions { settings: settings(json!({"standard": "pdfX1a", "compatibility": "1.4"})), ..Default::default() }).is_ok());
}

#[test]
fn options_not_applied_yet_come_back_as_warnings() {
    assert!(PdfSettings::default().warnings().is_empty());
    for (v, word) in [
        (json!({"createLayers": true, "compatibility": "1.4"}), "PDF 1.5"),
        (json!({"advanced": {"outlineText": false, "fontSubsetPercent": 35}}), "subset"),
        (json!({"output": {"outputIntent": "No Such Press"}}), "without embedding"),
        (json!({"output": {"registry": "http://example.com"}}), "condition identifier"),
        (json!({"standard": "pdfA2b", "output": {"trapped": true}}), "PDF/A"),
        (json!({"security": {"printing": "low"}}), "permissions"),
    ] {
        let w = settings(v.clone()).warnings();
        assert!(w.len() == 1 && w[0].contains(word), "{v}: {w:?}");
    }
    // Bleed and marks, the default view/overprint choices and applied options (image compression
    // too: a codec the writer lacks is reported when an image needs it; colour output; real text)
    // warn about nothing.
    for v in [
        json!({"output": {"conversion": "destination", "destination": vectorcraft_color::cms::GENERIC_CMYK, "profiles": "all"}}),
        json!({"output": {"conversion": "preserveNumbers", "profiles": "taggedSource"}}),
        json!({"output": {"outputIntent": vectorcraft_color::cms::GENERIC_CMYK, "outputCondition": "Press", "registry": "r", "trapped": true}}),
        json!({"output": {"outputConditionId": "CGATS TR 001"}}),
        json!({"advanced": {"outlineText": false}}),
        json!({"advanced": {"outlineText": true, "fontSubsetPercent": 35}}),
        json!({"bleed": {"useDocument": true, "top": 9}}),
        json!({"bleed": {"top": 9}, "marks": {"trim": true, "registration": true, "colorBars": true, "pageInfo": true}}),
        json!({"includeNonPrinting": true}),
        json!({"createLayers": true}),
        json!({"viewAfterSaving": true}),
        json!({"thumbnails": true, "fastWebView": true}),
        json!({"compression": {"compressText": false}}),
        json!({"compression": {"color": {"compression": "jpeg"}, "mono": {"compression": "ccittG4"}}}),
    ] {
        assert!(settings(v.clone()).warnings().is_empty(), "{v}");
    }
    let r =
        export_with_report(&doc(), &PdfOptions { settings: settings(json!({"thumbnails": true, "marks": {"trim": true}})), ..Default::default() })
            .unwrap();
    // The writer embeds the thumbnails it is given: without them, the one warning says so.
    assert!(r.warnings.len() == 1 && r.warnings[0].contains("thumbnails need the pages drawn"), "{:?}", r.warnings);
    assert!(r.bytes.starts_with(b"%PDF-1.7"));
}
