//! Opening password-protected PDFs: the open password or the permissions password unlocks them.

use serde_json::{Value, json};
use vectorcraft_testkit::pdf::{PdfPage, pdf};

use super::*;

#[test]
fn the_permissions_password_opens_a_protected_pdf_too() {
    let mut s = Session::new();
    // The test file's permissions (owner) password is its open password followed by `-owner`.
    let locked = vectorcraft_format::base64_encode(&pdf(&[PdfPage::new(80.0, 40.0, "0 g 0 0 10 10 re f")], Some("pw")));
    let info = |s: &mut Session, pw: &str| s.execute("document.pdfInfo", &json!({"dataBase64": locked, "password": pw})).unwrap();
    assert_eq!(info(&mut s, "pw-owner")["pages"], 1);
    assert_eq!(info(&mut s, "pw-owners")["wrongPassword"], true);
    let open = |s: &mut Session, pw: &str| s.execute("document.open", &json!({"name": "locked.pdf", "dataBase64": locked, "password": pw}));
    let e = open(&mut s, "nope").unwrap_err().to_string();
    assert!(e.contains("password is wrong"), "{e}");
    open(&mut s, "pw-owner").unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.width(), 80.0);
}

#[test]
fn export_with_passwords_writes_a_pdf_that_only_its_passwords_open() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    for compatibility in ["1.4", "1.5", "1.6", "1.7", "2.0"] {
        let p = json!({"compatibility": compatibility, "security": {"openPassword": "open", "permissionsPassword": "owner", "printing": "none"}});
        let out = s.execute("document.exportPdf", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let data = out["dataBase64"].as_str().unwrap().to_string();
        let bytes = vectorcraft_format::base64_decode(&data).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("/Encrypt"), "{compatibility}: not encrypted");
        let info = |s: &mut Session, p: Value| s.execute("document.pdfInfo", &p).unwrap();
        assert_eq!(info(&mut s, json!({"dataBase64": data}))["needsPassword"], true, "{compatibility}");
        assert_eq!(info(&mut s, json!({"dataBase64": data, "password": "nope"}))["wrongPassword"], true, "{compatibility}");
        for pw in ["open", "owner"] {
            assert_eq!(info(&mut s, json!({"dataBase64": data, "password": pw}))["pages"], 1, "{compatibility} {pw}");
        }
        // The open password restores the editable document (Preserve Editing is on by default).
        let mut r = Session::new();
        r.execute("document.open", &json!({"name": "locked.pdf", "dataBase64": data, "password": "open"})).unwrap();
        assert!(r.doc().unwrap().doc.node_count() >= 2, "{compatibility}: the art came back");
    }
}

#[test]
fn permissions_without_a_password_warn_and_a_password_applies_them() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    let warns = |v: &Value| v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("permissions need a password"));
    let open = s.execute("document.exportPdf", &json!({"security": {"printing": "none"}})).unwrap();
    assert!(warns(&open), "{open}");
    let locked = s.execute("document.exportPdf", &json!({"security": {"permissionsPassword": "owner", "printing": "none"}})).unwrap();
    assert!(!warns(&locked), "{locked}");
    let bytes = vectorcraft_format::base64_decode(locked["dataBase64"].as_str().unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("/Encrypt"));
}

/// The writer notes the permissions it didn't apply before the export notes the type it drew in
/// the fallback font: the reply gives what the file loses first, through both commands.
#[test]
fn the_fallback_font_note_comes_before_the_permissions_note() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    s.execute("text.create", &json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    for r in [
        s.execute("document.exportPdf", &json!({"security": {"printing": "none"}})).unwrap(),
        s.execute("document.export", &json!({"format": "pdf", "security": {"printing": "none"}})).unwrap(),
    ] {
        let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        let at = |s: &str| warnings.iter().position(|w| w.contains(s));
        let (font, permissions) = (at("Nonexistent Sans Probe"), at("permissions need a password"));
        assert!(font.is_some() && permissions.is_some() && font < permissions, "{warnings:?}");
    }
}
