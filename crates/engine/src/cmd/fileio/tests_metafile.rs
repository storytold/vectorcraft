//! EMF and WMF through the commands: export of one artboard or each, raster effects as images,
//! open, place, paste, and the format lists.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::*;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The EMF's record types.
fn emf_kinds(b: &[u8]) -> Vec<u32> {
    let mut out = vec![];
    let mut at = 0;
    while at + 8 <= b.len() {
        out.push(u32_at(b, at));
        at += u32_at(b, at + 4) as usize;
    }
    out
}

/// Two 100 pt artboards side by side, a red rectangle on each.
fn two_boards() -> Session {
    let mut s = session(100.0, 100.0, 2);
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.rectangle", &json!({"x": second + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s
}

#[test]
fn export_writes_an_emf_or_wmf_of_one_artboard() {
    let mut s = two_boards();
    let r = s.execute("document.export", &json!({"format": "emf"})).unwrap();
    assert_eq!(r["format"], "emf");
    let emf = b64(&r);
    assert_eq!(vectorcraft_metafile::sniff(&emf), Some(vectorcraft_metafile::Kind::Emf));
    // The first artboard's frame in 0.01 mm.
    let side = (100.0 * 2540.0 / 72.0f64).round() as i32;
    assert_eq!([i32_at(&emf, 24), i32_at(&emf, 28), i32_at(&emf, 32), i32_at(&emf, 36)], [0, 0, side, side]);
    assert_eq!(emf_kinds(&emf).iter().filter(|k| **k == 62).count(), 1, "one rectangle on it");
    // The second artboard alone; serialize gives the same bytes.
    let second = b64(&s.execute("document.export", &json!({"format": "emf", "artboard": 1})).unwrap());
    assert_eq!(emf_kinds(&second).iter().filter(|k| **k == 62).count(), 1);
    assert_eq!(b64(&s.execute("document.serialize", &json!({"format": "emf"})).unwrap()), emf);
    let wmf = b64(&s.execute("document.export", &json!({"format": "wmf"})).unwrap());
    assert_eq!(vectorcraft_metafile::sniff(&wmf), Some(vectorcraft_metafile::Kind::Wmf));
    assert_eq!(writable_format(None, Some("x/pic.wmf")).unwrap().id, "wmf");
    assert_eq!(writable_format(Some("EMF"), None).unwrap().id, "emf");
    // One file holds one artboard.
    assert!(s.execute("document.export", &json!({"format": "emf", "artboards": [0, 1]})).is_err());
}

#[test]
fn use_artboards_writes_one_picture_each_or_the_art() {
    let mut s = two_boards();
    let r = s.execute("document.export", &json!({"format": "wmf", "useArtboards": true})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0]["name"].as_str().unwrap().ends_with("-Artboard-1.wmf"), "{files:?}");
    // Without artboards: the bounds of the art (both rectangles).
    let r = s.execute("document.export", &json!({"format": "emf", "useArtboards": false})).unwrap();
    let emf = b64(&r);
    let width_pt = f64::from(i32_at(&emf, 32)) / 2540.0 * 72.0;
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    assert!((width_pt - (second + 30.0)).abs() < 0.05, "{width_pt}");
}

#[test]
fn raster_effects_are_written_as_images() {
    let mut s = session(200.0, 200.0, 1);
    let id = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    let r = s.execute("document.export", &json!({"format": "emf"})).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("raster effects")), "{r}");
    let kinds = emf_kinds(&b64(&r));
    assert!(kinds.contains(&114), "the shadow is an image with transparency");
}

/// Two placed images whose pixel data can't be decoded are left out of an EMF or WMF, and
/// `warnings` holds the note raster exports give, once, without naming either image's key.
#[test]
fn images_that_cant_be_decoded_give_the_raster_note_once() {
    const NOTE: &str = "an image couldn't be decoded and is missing from the picture";
    let mut s = session(200.0, 100.0, 1);
    crate::tests_menucmds::place_undecodable_png(&mut s);
    crate::tests_menucmds::place_undecodable_png_sized(&mut s, 32, 16);
    let keys: Vec<String> = s.doc().unwrap().doc.images.keys().cloned().collect();
    assert_eq!(keys.len(), 2);
    for format in ["emf", "wmf"] {
        let r = s.execute("document.export", &json!({"format": format})).unwrap();
        let w: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(w.iter().filter(|w| **w == NOTE).count(), 1, "{format}: {w:?}");
        assert!(!w.iter().any(|w| keys.iter().any(|k| w.contains(k.as_str()))), "{format}: {w:?}");
    }
}

#[test]
fn metafiles_open_and_place() {
    let mut s = two_boards();
    let emf = b64(&s.execute("document.export", &json!({"format": "emf"})).unwrap());
    let wmf = b64(&s.execute("document.export", &json!({"format": "wmf"})).unwrap());
    for (name, bytes) in [("pic.emf", &emf), ("pic.wmf", &wmf)] {
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
        assert_eq!(r["format"], &name[4..]);
        assert_eq!(r["warnings"], json!([]));
        let st = s.doc().unwrap();
        let ab = st.doc.artboards[0].rect;
        assert!((ab.width() - 100.0).abs() < 0.05 && (ab.height() - 100.0).abs() < 0.05, "{ab:?}");
        let fill = st.doc.layers[0].children().unwrap().iter().find(|n| matches!(n.kind, NodeKind::Path { .. })).unwrap().clone();
        let b = fill.geometric_bounds().unwrap();
        assert!((b.x0 - 10.0).abs() < 0.05 && (b.width() - 30.0).abs() < 0.05, "{b:?}");
        // Save can't write the file back: the document has no path.
        assert!(s.doc().unwrap().path.is_none());
    }
    // Placed: one group clipped to the picture's frame, named after the file.
    let mut s = session(300.0, 300.0, 1);
    let r = s.execute("file.place", &json!({"name": "pic.emf", "dataBase64": vectorcraft_format::base64_encode(&emf), "at": [150, 150]})).unwrap();
    assert_eq!(r["format"], "emf");
    assert!((r["width"].as_f64().unwrap() - 100.0).abs() < 0.05);
    let st = s.doc().unwrap();
    let placed = st.doc.layers[0].children().unwrap().last().unwrap().clone();
    assert_eq!(placed.name.as_deref(), Some("pic.emf"));
    assert!(matches!(placed.kind, NodeKind::Group { clip: true, .. }));
    let ext = fileio::PLACE_EXTS;
    assert!(ext.contains(&"emf") && ext.contains(&"wmf") && OPEN_EXTS.contains(&"emf") && OPEN_EXTS.contains(&"wmf"));
}

#[test]
fn formats_list_emf_and_wmf() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    for k in ["readable", "writable"] {
        let ids: Vec<&str> = r[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(ids.contains(&"emf") && ids.contains(&"wmf"), "{k}: {ids:?}");
    }
    let emf = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "emf").unwrap();
    assert_eq!(emf["mime"], "image/emf");
    assert!(emf["options"]["useArtboards"].is_object());
    assert!(open_filters().any(|(label, ext)| label == "EMF" && ext == ["emf"]));
    assert!(place_filters().any(|(label, ext)| label == "WMF" && ext == ["wmf"]));
    // Export for Screens writes the screen formats only.
    let mut s = two_boards();
    assert!(s.execute("document.exportForScreens", &json!({"formats": [{"format": "emf"}]})).is_err());
}

#[test]
fn a_pasted_metafile_becomes_objects() {
    let mut src = two_boards();
    let emf = b64(&src.execute("document.export", &json!({"format": "emf"})).unwrap());
    let mut s = session(300.0, 300.0, 1);
    let r = s.execute("clipboard.importEmf", &json!({"dataBase64": vectorcraft_format::base64_encode(&emf), "center": [150, 150]})).unwrap();
    assert_eq!(r["count"], 1);
    s.execute("edit.paste", &json!({"center": [150, 150]})).unwrap();
    let pasted = s.doc().unwrap().doc.layers[0].children().unwrap().clone();
    assert_eq!(pasted.len(), 1);
    let b = pasted[0].geometric_bounds().unwrap();
    assert!((b.center().x - 150.0).abs() < 0.05 && (b.width() - 30.0).abs() < 0.05, "{b:?}");
    assert!(s.execute("clipboard.importEmf", &json!({"dataBase64": vectorcraft_format::base64_encode(b"%PDF-1.7")})).is_err());
}
