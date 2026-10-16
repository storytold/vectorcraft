//! File → Print: `print.setup` saves the settings with the document (one undo step, kept in the
//! native file), `print.preview` lists what would print, `file.print` writes the PDF.

use serde_json::{Value, json};

use super::*;

/// A 300 × 200 RGB document with a rectangle; and the rectangle's id.
fn session() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    (s, id)
}

/// `file.print {p}` → (PDF bytes, its answer).
fn print(s: &mut Session, p: Value) -> (Vec<u8>, Value) {
    let v = s.execute("file.print", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    (vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap(), v)
}

#[test]
fn setup_is_saved_with_the_document_as_one_undo_step() {
    let (mut s, _) = session();
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["copies"], 1, "defaults before any setup");
    assert!(s.doc().unwrap().doc.print_setup.is_none(), "reading the settings changes nothing");
    let v = s.execute("print.setup", &json!({"settings": {"copies": 3, "media": "a4", "marks": {"trim": true}}})).unwrap();
    assert_eq!((v["settings"]["copies"].clone(), v["settings"]["media"].clone()), (json!(3), json!("a4")));
    let undo = s.doc().unwrap().history.undo.len();
    // Further options go over the saved ones; the same settings again are no new step.
    let v = s.execute("print.setup", &json!({"settings": {"collate": false}})).unwrap();
    let got = (v["settings"]["copies"].clone(), v["settings"]["collate"].clone(), v["settings"]["marks"]["trim"].clone());
    assert_eq!(got, (json!(3), json!(false), json!(true)));
    s.execute("print.setup", &json!({"settings": {"collate": false}})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Saved in the native file, and gone again after undoing both steps.
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(back.print_setup, doc.print_setup);
    assert!(back.print_setup.as_ref().is_some_and(|p| p["copies"] == 3));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.print_setup.is_none());
    // Files from before print settings load without them.
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    v.as_object_mut().unwrap().remove("print_setup");
    assert!(vectorcraft_format::load(v.to_string().as_bytes()).unwrap().print_setup.is_none());
}

#[test]
fn print_writes_the_saved_setup_with_overrides() {
    let (mut s, _) = session();
    s.execute("print.setup", &json!({"settings": {"media": "a4", "scaling": "fit"}})).unwrap();
    let (bytes, v) = print(&mut s, json!({"settings": {"copies": 2}}));
    assert_eq!(v["pages"], 2);
    let info = vectorcraft_pdf::info(&bytes, None).unwrap();
    assert_eq!(info.pages.len(), 2);
    // A4 turned landscape for the wide artboard.
    assert_eq!((info.pages[0].width.round(), info.pages[0].height.round()), (842.0, 595.0));
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["copies"], 1, "printing doesn't change the saved setup");
    // To a file.
    let path = std::env::temp_dir().join(format!("vc-print-{}.pdf", std::process::id()));
    let v = s.execute("file.print", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(v["pages"], 1);
    assert_eq!(std::fs::read(&path).unwrap().len() as u64, v["bytes"].as_u64().unwrap());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn preview_lists_pages_tiles_inks_and_warnings() {
    let (mut s, _) = session();
    let v = s.execute("print.preview", &json!({"settings": {"output": {"mode": "separations"}, "copies": 2}})).unwrap();
    assert_eq!(v["pages"], 8);
    assert_eq!(
        v["sheets"].as_array().unwrap().iter().map(|p| p["ink"].as_str().unwrap()).collect::<Vec<_>>(),
        ["Cyan", "Magenta", "Yellow", "Black"]
    );
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("RGB")), "{}", v["warnings"]);
    assert_eq!(v["settings"]["copies"], 2);
    let v = s.execute("print.preview", &json!({"settings": {"scaling": "tileFull", "scale": {"width": 400, "height": 400}}})).unwrap();
    let tiles = &v["tiles"][0];
    assert_eq!((tiles["columns"].clone(), tiles["rows"].clone()), (json!(2), json!(2)), "{v}");
    assert_eq!(v["pages"], 4);
    assert_eq!(v["sheets"][3]["tile"], 4);
}

#[test]
fn bad_settings_are_bad_params() {
    let (mut s, _) = session();
    for p in [
        json!({"settings": "letter"}),
        json!({"settings": {"copies": 0}}),
        json!({"settings": {"copies": -1}}),
        json!({"settings": {"media": "quarto"}}),
        json!({"settings": {"artboards": "range", "range": "2"}}),
        json!({"settings": {"scaling": "tileFull", "overlap": 500}}),
    ] {
        for c in ["file.print", "print.preview"] {
            assert!(matches!(s.execute(c, &p), Err(EngineError::BadParams { .. })), "{c} {p}");
        }
    }
    // The setup refuses what it can check without laying the document out.
    assert!(matches!(s.execute("print.setup", &json!({"settings": {"copies": 1000}})), Err(EngineError::BadParams { .. })));
    assert!(s.doc().unwrap().doc.print_setup.is_none());
}

#[test]
fn raster_effects_print_as_images() {
    let (mut s, id) = session();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    let (bytes, _) = print(&mut s, json!({}));
    assert!(String::from_utf8_lossy(&bytes).contains("/Image"));
}

#[test]
fn type_in_a_missing_family_warns_as_pdf_export_does() {
    let (mut s, _) = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap()["id"].clone();
    let fallback = |v: &Value| -> Vec<String> {
        v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("fallback font")).map(str::to_string).collect()
    };
    let export = fallback(&s.execute("document.exportPdf", &json!({})).unwrap());
    assert!(export.first().is_some_and(|w| w.contains("Nonexistent Sans Probe")), "{export:?}");
    let (_, v) = print(&mut s, json!({}));
    assert_eq!(fallback(&v), export);
    // Type printed as an image (a raster effect, Print as Bitmap) is drawn in the fallback font too.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    for settings in [json!({}), json!({"advanced": {"printAsBitmap": true}})] {
        let (_, v) = print(&mut s, json!({ "settings": settings }));
        assert_eq!(fallback(&v), export, "{settings}");
    }
}

#[test]
fn type_that_does_not_fit_its_frame_is_named() {
    let (mut s, _) = session();
    let p = json!({"x": 10, "y": 100, "text": "Ab\nCd\nEf\nGh", "size": 12, "area": {"width": 100, "height": 20}});
    let id = s.execute("text.create", &p).unwrap()["id"].clone();
    let over = |v: &Value| -> Vec<String> {
        v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("fit its frame")).map(str::to_string).collect()
    };
    let (_, v) = print(&mut s, json!({}));
    assert_eq!(over(&v), [format!("1 text object doesn't fit its frame: the text past the end of the frame doesn't print (id {id})")]);
    assert_eq!(over(&v), over(&s.execute("print.preview", &json!({})).unwrap()));
    // Type printed as an image is named too.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    for settings in [json!({}), json!({"advanced": {"printAsBitmap": true}})] {
        let (_, v) = print(&mut s, json!({ "settings": settings }));
        assert_eq!(over(&v).len(), 1, "{settings}");
    }
    s.execute("text.areaOptions", &json!({"ids": [id], "height": 150})).unwrap();
    let (_, v) = print(&mut s, json!({}));
    assert_eq!(over(&v), Vec::<String>::new());
}
