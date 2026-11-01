//! `document.exportPdf`, `document.pdfSettings` and the PDF options of every PDF export.

use serde_json::{Value, json};

use super::pdf::DEFAULT_PRESET;
use super::*;

fn session(artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": artboards})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn export(s: &mut Session, p: Value) -> Value {
    s.execute("document.exportPdf", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn warnings(v: &Value) -> Vec<String> {
    v["warnings"].as_array().expect("warnings").iter().map(|w| w.as_str().unwrap().to_string()).collect()
}

fn pages(bytes: &[u8]) -> usize {
    vectorcraft_pdf::import(bytes).unwrap().artboards.len()
}

/// `p` plus `format: pdf` (the same options through `document.export`).
fn as_export(p: &Value) -> Value {
    let mut o = p.as_object().cloned().unwrap_or_default();
    o.insert("format".into(), json!("pdf"));
    Value::Object(o)
}

#[test]
fn export_pdf_writes_the_compatibility_header_and_the_range() {
    let mut s = session(3);
    let v = export(&mut s, json!({}));
    let pdf = b64(&v);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert_eq!(pages(&pdf), 3);
    assert_eq!(v["bytes"], pdf.len());
    assert!(warnings(&v).is_empty(), "{v}");
    assert!(b64(&export(&mut s, json!({"compatibility": "1.5"}))).starts_with(b"%PDF-1.5"));
    assert!(b64(&export(&mut s, json!({"compatibility": "2.0"}))).starts_with(b"%PDF-2.0"));
    assert_eq!(pages(&b64(&export(&mut s, json!({"range": "1,3"})))), 2);
    assert_eq!(pages(&b64(&export(&mut s, json!({"artboards": [1]})))), 1);
    // The same options through document.export and document.serialize.
    let v = s.execute("document.export", &as_export(&json!({"compatibility": "1.4", "range": "2"}))).unwrap();
    assert!(b64(&v).starts_with(b"%PDF-1.4") && pages(&b64(&v)) == 1);
    assert!(v["warnings"].is_array());
    let v = s.execute("document.serialize", &as_export(&json!({"compatibility": "1.6"}))).unwrap();
    assert!(b64(&v).starts_with(b"%PDF-1.6"));
}

#[test]
fn bad_options_are_refused() {
    let mut s = session(2);
    for p in [
        json!({"compatibility": "1.2"}),
        json!({"compatibility": "1.3", "standard": "pdfA2b"}),
        json!({"compatibility": "1.3", "flattenerPreset": "Nope"}),
        json!({"compatibility": 1.5}),
        json!({"range": "1-3"}),
        json!({"range": "0"}),
        json!({"artboards": []}),
        json!({"preset": "Nope"}),
        json!({"standard": "pdfX4", "compatibility": "1.7"}),
        json!({"standard": "pdfA2b", "compatibility": "2.0"}),
        json!({"compression": {"color": {"ppi": 0}}}),
        json!({"compression": "zip"}),
        json!({"standard": "pdfA2b", "security": {"openPassword": "x"}}),
        json!({"security": {"openPassword": "x", "permissionsPassword": "x"}}),
    ] {
        let e = s.execute("document.exportPdf", &p).unwrap_err();
        assert!(matches!(e, crate::EngineError::BadParams { .. }), "{p}: {e}");
        assert!(s.execute("document.export", &as_export(&p)).is_err(), "document.export {p}");
    }
}

#[test]
fn options_apply_over_the_preset() {
    let mut s = session(1);
    let a = b64(&export(&mut s, json!({"preset": DEFAULT_PRESET, "compression": {"compressText": false}})));
    let b = b64(&export(&mut s, json!({"preset": "default"})));
    assert!(a.len() > b.len(), "content streams left uncompressed");
    assert!(String::from_utf8_lossy(&a).contains(" re") || String::from_utf8_lossy(&a).contains(" l"));
    // `null` (the default document.formats lists) keeps the preset's value, at any depth.
    let p = json!({"compatibility": null, "compression": null, "security": {"openPassword": null}, "marks": {"weight": null}});
    let v = s.execute("document.pdfSettings", &p).unwrap();
    assert_eq!(v["changed"], json!([]), "{v}");
    assert!(b64(&export(&mut s, p)).starts_with(b"%PDF-1.7"));
}

#[test]
fn pdf_a_refuses_pdf_2() {
    let mut s = session(1);
    let e = s.execute("document.pdfSettings", &json!({"standard": "pdfA2b", "compatibility": "2.0"})).unwrap_err();
    assert!(matches!(e, crate::EngineError::BadParams { .. }), "{e}");
    let pdf = b64(&export(&mut s, json!({"standard": "pdfA2b", "compatibility": "1.6"})));
    assert!(pdf.starts_with(b"%PDF-1.6") && String::from_utf8_lossy(&pdf).contains("pdfaid"));
}

#[test]
fn options_not_applied_yet_and_document_features_warn() {
    let mut s = session(1);
    // Printer's marks are drawn, thumbnails embedded and the file linearised: nothing to report.
    let v = export(&mut s, json!({"thumbnails": true, "fastWebView": true, "marks": {"trim": true}}));
    assert_eq!(warnings(&v), Vec::<String>::new());
    let text = String::from_utf8_lossy(&b64(&v)).into_owned();
    assert!(text.contains("/Thumb ") && text.contains("/Linearized 1"));
    // A pattern stroke is written as its tiles clipped to the stroke: nothing to report.
    let tile = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [tile]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let frame = s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setStroke", &json!({"ids": [frame], "swatch": "Dots"})).unwrap();
    assert_eq!(warnings(&export(&mut s, json!({}))).len(), 0);
    // A knockout group is approximated: its warning comes back from every PDF path.
    s.execute("select.set", &json!({"ids": [frame]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"knockout": "on"})).unwrap();
    let knockout = |w: Vec<String>| w.iter().any(|w| w.contains("knockout groups"));
    assert!(knockout(warnings(&export(&mut s, json!({})))));
    assert!(knockout(warnings(&s.execute("document.export", &as_export(&json!({}))).unwrap())));
    // Other formats have none.
    assert!(warnings(&s.execute("document.export", &json!({"format": "png"})).unwrap()).is_empty());
}

/// A placed 16-bit PNG is written at 8 bits a sample in PDF 1.3 and 1.4 files (PDF/X-3 and the
/// pages of a .ai file too), and keeps 16 bits from PDF 1.5.
#[test]
fn a_16_bit_png_is_written_at_8_bits_before_pdf_1_5() {
    let mut s = session(1);
    let deep = image::ImageBuffer::from_fn(8, 8, |x, y| image::Rgb([x as u16 * 8000, y as u16 * 8000, 40000]));
    let mut png = vec![];
    image::DynamicImage::ImageRgb16(deep).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.execute("file.place", &json!({"name": "deep.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [50, 40]})).unwrap();
    let readable = json!({"compressText": false});
    for (cmd, p, depth) in [
        ("document.exportPdf", json!({"compatibility": "1.3", "compression": readable}), 8),
        ("document.exportPdf", json!({"compatibility": "1.4", "compression": readable}), 8),
        ("document.exportPdf", json!({"standard": "pdfX3", "compression": readable}), 8),
        ("file.saveCopy", json!({"format": "ai", "compatibility": "1.4", "compress": false}), 8),
        ("document.exportPdf", json!({"compatibility": "1.5", "compression": readable}), 16),
        ("document.exportPdf", json!({"compression": readable}), 16),
    ] {
        let v = s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
        let text = String::from_utf8_lossy(&b64(&v)).into_owned();
        let (eight, sixteen) = (text.contains("/BitsPerComponent 8"), text.contains("/BitsPerComponent 16"));
        assert_eq!((eight, sixteen), (depth == 8, depth == 16), "{cmd} {p}");
        assert!(warnings(&v).iter().all(|w| !w.contains("decoded")), "{cmd} {p}: {}", v["warnings"]);
    }
}

/// A placed image whose pixel data can't be decoded is left out of every PDF the engine writes,
/// and the rest of the page is written. Each reply that carries the PDF's warnings has the note
/// raster exports give, once.
#[test]
fn every_pdf_leaves_out_an_image_that_cant_be_decoded() {
    const NOTE: &str = "an image couldn't be decoded and is missing from the picture";
    let mut s = session(1);
    let cut = crate::tests_menucmds::place_undecodable_png(&mut s);
    // Whether each object on the PDF's pages is an image. The pages should hold the rectangle and
    // no image.
    let drawn = |bytes: &[u8]| {
        let mut images = vec![];
        vectorcraft_pdf::import(bytes).unwrap().walk(|n| {
            if !n.is_container() && !matches!(n.kind, vectorcraft_doc::NodeKind::Path { clipping: true, .. }) {
                images.push(matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)));
            }
        });
        images
    };
    for (cmd, p) in [
        ("document.exportPdf", json!({})),
        ("document.exportPdf", json!({"preset": "High Quality Print"})),
        ("document.exportPdf", json!({"standard": "pdfX1a"})),
        ("document.export", json!({"format": "pdf"})),
        ("file.saveCopy", json!({"format": "pdf"})),
        ("file.saveCopy", json!({"format": "ai"})),
        ("file.print", json!({})),
    ] {
        let v = s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
        assert_eq!(warnings(&v).iter().filter(|w| *w == NOTE).count(), 1, "{cmd} {p}: {}", v["warnings"]);
        assert_eq!(drawn(&b64(&v)), [false], "{cmd} {p}");
    }
    // The .ai file reopens with the image as it was placed, from the image bytes in its editing data.
    let ai = b64(&s.execute("file.saveCopy", &json!({"format": "ai"})).unwrap());
    let placed = s.doc().unwrap().doc.images.clone();
    s.execute("document.open", &json!({"name": "cut.ai", "dataBase64": vectorcraft_format::base64_encode(&ai)})).unwrap();
    let reopened = &s.doc().unwrap().doc;
    assert!(reopened.node(cut).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))));
    assert_eq!(reopened.images, placed);
    // Export for Screens, a native file carrying a PDF, and the PDF that Copy offers are written.
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "pdf"}]})).unwrap();
    assert_eq!(drawn(&b64(&r["files"][0])), [false]);
    assert!(s.execute("file.saveCopy", &json!({"format": "vectorcraft", "pdfCompatible": true})).is_ok());
    s.execute("select.set", &json!({"ids": [cut.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    assert!(b64(&s.execute("clipboard.exportPdf", &json!({})).unwrap()).starts_with(b"%PDF"));
    s.execute("prefs.set", &json!({"key": "copyAsPdf", "value": true})).unwrap();
    assert!(s.clipboard_flavours().iter().any(|f| f.mime == crate::cmd::clipboard::PDF));
}

#[test]
fn export_pdf_writes_a_path() {
    let mut s = session(2);
    let dir = std::env::temp_dir().join(format!("vc-exportpdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.pdf").to_string_lossy().to_string();
    let v = export(&mut s, json!({"path": path, "range": "2"}));
    assert_eq!(v["path"], path.as_str());
    assert!(v.get("dataBase64").is_none());
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(v["bytes"], bytes.len());
    assert_eq!(pages(&bytes), 1);
    assert_eq!(s.doc().unwrap().path, None, "exporting never retargets the document");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_settings_summarise_changes_and_warnings() {
    let mut s = Session::new();
    let v = s.execute("document.pdfSettings", &json!({})).unwrap();
    assert_eq!(v["settings"]["compatibility"], "1.7");
    assert_eq!(v["presets"][0], DEFAULT_PRESET);
    assert_eq!(v["changed"], json!([]));
    assert_eq!(v["warnings"], json!([]));
    let p = json!({"compatibility": "1.5", "compression": {"compressText": false}, "createLayers": true, "security": {"copy": false, "openPassword": ""}});
    let v = s.execute("document.pdfSettings", &p).unwrap();
    let changed: Vec<&str> = v["changed"].as_array().unwrap().iter().map(|c| c["option"].as_str().unwrap()).collect();
    assert_eq!(changed, ["compatibility", "compression.compressText", "createLayers", "security.copy"], "key order");
    assert_eq!(v["changed"][0]["value"], "1.5");
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1, "permissions (PDF layers are written at 1.5): {}", v["warnings"]);
    assert!(!v.to_string().contains("Password"), "passwords never come back");
    assert!(s.execute("document.pdfSettings", &json!({"compatibility": "9"})).is_err());
    // With the document: the in-memory export adds the document's own warnings.
    let mut s = session(1);
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let g = s.doc().unwrap().selection.objects[0].0;
    // Raster effects are written as images: nothing to report.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [g]})).unwrap();
    assert_eq!(s.execute("document.pdfSettings", &json!({"includeDocument": true})).unwrap()["warnings"], json!([]));
    s.execute("transparency.set", &json!({"knockout": "on"})).unwrap();
    assert_eq!(s.execute("document.pdfSettings", &json!({})).unwrap()["warnings"], json!([]));
    let full = s.execute("document.pdfSettings", &json!({"includeDocument": true})).unwrap();
    assert!(warnings(&full).iter().any(|w| w.contains("knockout groups")), "{}", full["warnings"]);
}

#[test]
fn formats_list_the_pdf_options() {
    let mut s = Session::new();
    let v = s.execute("document.formats", &json!({})).unwrap();
    let pdf = v["formats"].as_array().unwrap().iter().find(|f| f["id"] == "pdf").unwrap();
    for k in ["artboard", "range", "preset", "standard", "compatibility", "compression", "security"] {
        assert!(pdf["options"].get(k).is_some(), "{k}");
    }
    assert_eq!(pdf["options"]["compatibility"]["default"], "1.7");
    assert_eq!(pdf["options"]["preset"]["default"], DEFAULT_PRESET);
}
