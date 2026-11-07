//! Edit → PDF Presets: built-in presets are read-only, saved ones apply wherever a preset is named,
//! and presets files round-trip.

use serde_json::{Value, json};

use crate::Session;
use crate::cmd::fileio::pdf::DEFAULT_PRESET;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    s
}

fn names(s: &mut Session) -> Vec<String> {
    let v = s.execute("pdf.preset.list", &json!({})).unwrap();
    v["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

fn pdf(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

fn is_bad_params(r: crate::Result<Value>) -> bool {
    matches!(r, Err(crate::EngineError::BadParams { .. }))
}

#[test]
fn built_in_presets_are_listed_and_read_only() {
    let mut s = session();
    let v = s.execute("pdf.preset.list", &json!({})).unwrap();
    let rows = v["presets"].as_array().unwrap();
    assert_eq!(rows[0]["name"], DEFAULT_PRESET);
    assert!(rows.iter().all(|r| r["builtIn"] == true && !r["description"].as_str().unwrap().is_empty()));
    let row = |n: &str| rows.iter().find(|r| r["name"] == n).unwrap_or_else(|| panic!("{n}"));
    for n in ["High Quality Print", "Press Quality", "Smallest File Size", "PDF/X-1a:2001", "PDF/X-3:2002", "PDF/X-4:2010"] {
        assert_eq!(row(n)["builtIn"], true);
    }
    assert!(rows.iter().all(|r| r["supported"] == true), "every built-in standard is written");
    assert_eq!(row(DEFAULT_PRESET)["settings"]["preserveEditing"], true);
    // Read-only: neither changed nor deleted, in any case.
    assert!(is_bad_params(s.execute("pdf.preset.save", &json!({"name": "press quality", "compatibility": "1.4"}))));
    assert!(is_bad_params(s.execute("pdf.preset.save", &json!({"name": "default"}))));
    assert!(is_bad_params(s.execute("pdf.preset.delete", &json!({"name": "Smallest File Size"}))));
    assert_eq!(names(&mut s).len(), 7);
    // Built-in presets apply as named.
    let v = s.execute("document.pdfSettings", &json!({"preset": "Smallest File Size"})).unwrap();
    assert_eq!((&v["settings"]["fastWebView"], &v["changed"]), (&json!(true), &json!([])), "{v}");
    let pdf = s.execute("document.exportPdf", &json!({"preset": "PDF/X-4:2010"})).unwrap();
    assert!(pdf["dataBase64"].is_string(), "PDF/X is written");
}

#[test]
fn saved_presets_apply_wherever_a_preset_is_named() {
    let mut s = session();
    let r = s
        .execute("pdf.preset.save", &json!({"name": "Web", "preset": "Smallest File Size", "compatibility": "1.5", "description": "For the site"}))
        .unwrap();
    assert_eq!((&r["name"], &r["created"]), (&json!("Web"), &json!(true)));
    assert_eq!(names(&mut s).last().map(String::as_str), Some("Web"));
    // document.exportPdf, document.export, serialize and pdfSettings take it.
    let v = s.execute("document.exportPdf", &json!({"preset": "web"})).unwrap();
    assert!(pdf(&v).starts_with(b"%PDF-1.5"), "the preset's compatibility");
    assert!(String::from_utf8_lossy(&pdf(&v)).contains("/Linearized 1"), "the preset's fast web view: {v}");
    let v = s.execute("document.export", &json!({"format": "pdf", "preset": "Web"})).unwrap();
    assert!(pdf(&v).starts_with(b"%PDF-1.5"));
    let v = s.execute("document.serialize", &json!({"format": "pdf", "preset": "Web", "compatibility": "1.6"})).unwrap();
    assert!(pdf(&v).starts_with(b"%PDF-1.6"), "options apply over it");
    let v = s.execute("document.pdfSettings", &json!({"preset": "Web", "thumbnails": true})).unwrap();
    assert_eq!(v["changed"], json!([{"option": "thumbnails", "value": true}]), "{v}");
    assert!(v["presets"].as_array().unwrap().contains(&json!("Web")));
    // Saving it again changes it from its own settings; it can be renamed but not to a taken name.
    let r = s.execute("pdf.preset.save", &json!({"name": "Web", "thumbnails": true})).unwrap();
    assert_eq!((&r["created"], &r["settings"]["compatibility"], &r["settings"]["thumbnails"]), (&json!(false), &json!("1.5"), &json!(true)));
    assert!(is_bad_params(s.execute("pdf.preset.save", &json!({"name": "Web", "newName": "Press Quality"}))));
    assert_eq!(s.execute("pdf.preset.save", &json!({"name": "Web", "newName": "Site"})).unwrap()["name"], "Site");
    assert_eq!(s.prefs.pdf_presets[0].description, "For the site", "kept");
    // Passwords are never stored; new presets get a free name.
    s.execute("pdf.preset.save", &json!({"name": "Locked", "security": {"openPassword": "x"}})).unwrap();
    assert!(s.prefs.pdf_presets[1].settings.security.open_password.is_empty());
    assert_eq!(s.execute("pdf.preset.save", &json!({})).unwrap()["name"], "PDF Preset 1");
    assert_eq!(s.execute("pdf.preset.delete", &json!({"name": "locked"})).unwrap()["deleted"], "Locked");
    assert!(is_bad_params(s.execute("document.exportPdf", &json!({"preset": "Locked"}))), "deleted");
}

#[test]
fn presets_export_and_import_round_trip() {
    let mut s = session();
    s.execute("pdf.preset.save", &json!({"name": "Proof", "preset": "High Quality Print", "marks": {"trim": true}})).unwrap();
    let saved = s.prefs.pdf_presets.clone();
    let data = s.execute("pdf.preset.export", &json!({"names": ["Proof", "Press Quality"]})).unwrap();
    assert_eq!(data["count"], 2);
    let text = data["data"].as_str().unwrap().to_string();
    s.execute("pdf.preset.delete", &json!({"name": "Proof"})).unwrap();
    let r = s.execute("pdf.preset.import", &json!({"data": text})).unwrap();
    assert_eq!(r["imported"], json!(["Proof", "Press Quality 2"]), "a built-in name gets a number");
    assert_eq!(s.prefs.pdf_presets[0], saved[0], "the same preset comes back");
    let press = vectorcraft_pdf::builtin_preset("Press Quality").unwrap();
    assert_eq!(s.prefs.pdf_presets[1].settings, press.settings);
    // replace: saved presets of the same names are overwritten.
    let r = s.execute("pdf.preset.import", &json!({"data": text, "replace": true})).unwrap();
    assert_eq!(r["imported"], json!(["Proof", "Press Quality 3"]));
    assert_eq!(s.prefs.pdf_presets.len(), 3);
    // Through a file, and every saved preset by default.
    let path = std::env::temp_dir().join(format!("vc-pdfpresets-{}.vcpdfpresets", std::process::id())).to_string_lossy().to_string();
    assert_eq!(s.execute("pdf.preset.export", &json!({"path": path})).unwrap()["count"], 3);
    let mut t = session();
    assert_eq!(t.execute("pdf.preset.import", &json!({"path": path})).unwrap()["imported"].as_array().unwrap().len(), 3);
    assert_eq!(t.prefs.pdf_presets, s.prefs.pdf_presets);
    let _ = std::fs::remove_file(&path);
    // Not a presets file, or values out of range: refused, nothing added.
    for bad in [
        json!({"data": "{}"}),
        json!({"data": "{\"format\": \"vcflattener\", \"presets\": []}"}),
        json!({"data": "{\"format\": \"vcpdfpresets\", \"presets\": [{\"name\": \"X\", \"settings\": {\"marks\": {\"weight\": 99}}}]}"}),
        json!({"dataBase64": "!!"}),
        json!({}),
    ] {
        assert!(is_bad_params(t.execute("pdf.preset.import", &bad)), "{bad}");
    }
    assert_eq!(t.prefs.pdf_presets.len(), 3);
    assert!(is_bad_params(Session::new().execute("pdf.preset.export", &json!({}))), "nothing saved to export");
}

#[test]
fn saved_presets_persist_with_the_preferences() {
    let mut s = session();
    s.execute("pdf.preset.save", &json!({"name": "Kept", "compatibility": "1.4"})).unwrap();
    let back: crate::Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back.pdf_presets, s.prefs.pdf_presets);
    // Resetting the preferences keeps them (a library, not a preference).
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.pdf_presets.len(), 1);
    // Preferences saved before PDF presets existed still load.
    let old: crate::Prefs = serde_json::from_value(json!({"keyboardIncrement": 2.0})).unwrap();
    assert!(old.pdf_presets.is_empty());
}
