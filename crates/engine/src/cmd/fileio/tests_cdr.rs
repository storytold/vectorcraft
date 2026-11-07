//! CorelDRAW through the commands: export (versions, validation, options), open, place, file info,
//! the compatibility matrix and the format lists.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// Two 200 × 100 pt artboards with a red rectangle on each.
fn drawing() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": second + 10.0, "y": 10, "width": 30, "height": 20})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("document.export", &p).unwrap();
    assert_eq!(r["format"], "cdr");
    // Validation passing is not a warning: the warnings list only what was approximated or lost.
    let w = r["warnings"].as_array().unwrap();
    assert!(!w.iter().any(|w| w.as_str().unwrap().contains("validat")), "{w:?}");
    b64(&r)
}

fn paths(s: &Session) -> usize {
    let mut n = 0;
    s.doc().unwrap().doc.walk(|x| n += usize::from(matches!(x.kind, NodeKind::Path { .. })));
    n
}

#[test]
fn export_open_and_info_round_trip() {
    let mut s = drawing();
    let bytes = export(&mut s, json!({"format": "cdr"}));
    let info = s.execute("document.cdrInfo", &json!({"name": "a.cdr", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert_eq!(info["version"], 16);
    assert_eq!(info["product"], "CorelDRAW X6");
    assert_eq!(info["readable"], true);
    assert_eq!(info["pages"], 2);
    let w = info["pageSize"][0].as_f64().unwrap();
    assert!((w - 200.0 * 25.4 / 72.0).abs() < 0.01, "{w}");
    // Open: recognised by its content, under another name too.
    let mut t = Session::new();
    let r = t.execute("document.open", &json!({"name": "drawing.bin", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert_eq!(r["format"], "cdr");
    assert_eq!(t.doc().unwrap().doc.artboards.len(), 2);
    assert_eq!(paths(&t), 2);
    // Two plain rectangles come in whole: no import notes.
    assert_eq!(r["warnings"], json!([]), "{r}");
}

#[test]
fn every_version_and_the_options_export() {
    let mut s = drawing();
    for v in [7, 12, 13, 14, 15, 16, 18, 20, 22] {
        let bytes = export(&mut s, json!({"format": "cdr", "version": v}));
        assert_eq!(vectorcraft_cdr::detect_version(&bytes).unwrap().version, v);
    }
    let r = s.execute("document.export", &json!({"format": "cdr", "validate": false})).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("without validation")));
    for bad in [json!({"format": "cdr", "version": 5}), json!({"format": "cdr", "version": 30}), json!({"format": "cdr", "version": "x"})] {
        assert!(s.execute("document.export", &bad).is_err(), "{bad}");
    }
}

#[test]
fn place_imports_into_the_open_document() {
    let mut s = drawing();
    let bytes = export(&mut s, json!({"format": "cdr"}));
    let before = paths(&s);
    let p = json!({"name": "logo.cdr", "dataBase64": vectorcraft_format::base64_encode(&bytes), "cdr": {"preserveLayers": false}});
    s.execute("file.place", &p).unwrap();
    assert!(paths(&s) > before, "placed art joins the document");
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 2, "the document's own artboards stay");
}

#[test]
fn bad_files_and_options_are_errors() {
    let mut s = Session::new();
    let e = s.execute("document.cdrInfo", &json!({"name": "x.cdr", "dataBase64": vectorcraft_format::base64_encode(b"not a cdr")})).unwrap_err();
    assert!(e.to_string().contains("CorelDRAW"), "{e}");
    let junk = [b"RIFF\x20\x00\x00\x00CDRG".as_slice(), &[0xEE; 64]].concat();
    // Junk after the header: an error, or an empty document with a damaged-file warning; never a panic.
    match s.execute("document.open", &json!({"name": "x.cdr", "dataBase64": vectorcraft_format::base64_encode(&junk)})) {
        Err(_) => {}
        Ok(r) => assert!(r["warnings"].to_string().contains("damaged"), "{r}"),
    }
    let mut d = drawing();
    let bytes = export(&mut d, json!({"format": "cdr"}));
    let e = s
        .execute("document.open", &json!({"name": "a.cdr", "dataBase64": vectorcraft_format::base64_encode(&bytes), "cdr": {"scale": -1}}))
        .unwrap_err();
    assert!(e.to_string().contains("scale"));
}

#[test]
fn capabilities_and_format_lists() {
    let mut s = Session::new();
    let c = s.execute("document.cdrCapabilities", &json!({})).unwrap();
    assert!(c["features"].as_array().unwrap().len() >= 13);
    assert_eq!(c["defaultExportVersion"], 16);
    let f = format("cdr").unwrap();
    assert!(f.read && f.write && !f.raster);
    assert!(OPEN_EXTS.contains(&"cdr") && PLACE_EXTS.contains(&"cdr"));
    assert!(open_filters().any(|(label, exts)| label == "CorelDRAW" && exts == ["cdr"]));
}
