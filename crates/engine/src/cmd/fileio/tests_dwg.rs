//! DWG through public commands: sniffing, options, editable placement, history and safe saving.

use super::*;
use vectorcraft_doc::{NodeKind, Unit};

const DRAWING: &[u8] = include_bytes!("../../../../cad/tests/fixtures/dwg/drawing-2018.dwg");

fn source() -> Value {
    json!({"name": "drawing.DWG", "dataBase64": vectorcraft_format::base64_encode(DRAWING)})
}

#[test]
fn dwg_is_readable_by_content_and_extension_but_not_writable() {
    assert_eq!(detect("unknown.bin", DRAWING).unwrap().id, "dwg");
    assert_eq!(detect("wrong.dxf", DRAWING).unwrap().id, "dwg");
    assert_eq!(detect("drawing.DWG", &[]).unwrap().id, "dwg");
    assert!(OPEN_EXTS.contains(&"dwg") && PLACE_EXTS.contains(&"dwg"));
    let mut session = Session::new();
    let formats = session.execute("document.formats", &json!({})).unwrap();
    assert!(formats["readable"].as_array().unwrap().contains(&json!("dwg")));
    assert!(!formats["writable"].as_array().unwrap().contains(&json!("dwg")));
    assert!(!SAVE_FORMATS.contains(&"dwg"));
    assert!(writable_format(None, Some("drawing.DWG")).unwrap_err().contains("DXF"));
}

#[test]
fn dwg_open_info_and_scale_preserve_editable_art() {
    let mut session = Session::new();
    let mut p = source();
    let info = session.execute("document.dxfInfo", &p).unwrap();
    assert_eq!(info["version"], "2018");
    assert_eq!(info["unit"], "Millimeters");
    p["dxf"] = json!({"unit": "mm", "scale": 2, "mergeLayers": true});
    let opened = session.execute("document.open", &p).unwrap();
    assert_eq!(opened["format"], "dwg");
    assert_eq!(opened["restored"], false);
    let state = session.doc().unwrap();
    assert_eq!(state.doc.layers.len(), 1);
    let mut widths = Vec::new();
    let mut texts = Vec::new();
    state.doc.walk(|node| match &node.kind {
        NodeKind::Path { .. } => widths.push(node.geometric_bounds().unwrap().width() / Unit::Millimeters.points()),
        NodeKind::Text(t) => texts.push(t.plain_text()),
        _ => {}
    });
    widths.sort_by(f64::total_cmp);
    assert_eq!(widths.len(), 2);
    assert!((widths[0] - 10.0).abs() < 1e-6 && (widths[1] - 50.0).abs() < 1e-6, "{widths:?}");
    assert_eq!(texts, ["VectorCraft CAD"]);
    // A nonexistent layout fails before adding another document.
    let count = session.docs.len();
    p["dxf"]["layout"] = json!("missing layout");
    assert!(session.execute("document.open", &p).is_err());
    assert_eq!(session.docs.len(), count);
}

#[test]
fn dwg_place_is_one_editable_group_and_one_undo_step() {
    let mut session = Session::new();
    session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let mut p = source();
    p["dxf"] = json!({"fit": true});
    let before = session.doc().unwrap().history.undo.len();
    let placed = session.execute("file.place", &p).unwrap();
    assert_eq!(placed["format"], "dwg");
    assert_eq!(placed["linked"], false);
    let id = session.doc().unwrap().selection.objects[0];
    let node = session.doc().unwrap().doc.node(id).unwrap();
    assert!(node.children().is_some());
    assert_eq!(session.doc().unwrap().history.undo.len(), before + 1);
    session.execute("edit.undo", &json!({})).unwrap();
    assert!(session.doc().unwrap().doc.node(id).is_none());
    session.execute("edit.redo", &json!({})).unwrap();
    assert!(session.doc().unwrap().doc.node(id).is_some());
}

#[test]
fn dwg_import_saves_as_native_without_overwriting_the_source() {
    let mut session = Session::new();
    let read = read_for_open("drawing.dwg", DRAWING, Some("/tmp/source-3450.dwg".into()), &json!({}), false).unwrap();
    open_read(&mut session, read).unwrap();
    assert!(session.doc().unwrap().path.is_none());
    let saved = session.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    let restored = load("saved.vectorcraft", &bytes).unwrap();
    let mut paths = 0;
    restored.doc.walk(|node| paths += usize::from(matches!(node.kind, NodeKind::Path { .. })));
    assert_eq!(paths, 2);
}
