//! #3575: a native file holding an object, artboard, guide or ruler origin beyond the canvas
//! (`MAX_COORD`) opened, but every move then failed with "result would exceed the canvas": edits
//! check those. Opening it now removes them (or resets the ruler origin) and says so.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

/// A saved two-rectangle document as JSON, with its first rectangle 1e17 pt down when `far_object`.
fn document(far_object: bool) -> Value {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap();
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    if far_object {
        let far = &mut v["document"]["layers"][0]["kind"]["children"][0]["kind"];
        far["path"]["subpaths"][0]["anchors"].as_array_mut().unwrap().iter_mut().for_each(|a| a["p"][1] = json!(a["p"][1].as_f64().unwrap() + 1e17));
        far["live"]["xf"][5] = json!(far["live"]["xf"][5].as_f64().unwrap() + 1e17);
    }
    v
}

/// Open `v` from a file, select everything and move it: → the open reply's warnings.
fn open_and_move(v: &Value, tag: &str) -> (Session, Value) {
    let path = std::env::temp_dir().join(format!("vectorcraft-beyond-canvas-{tag}-{}.vectorcraft", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(v).unwrap()).unwrap();
    let mut s = Session::new();
    let opened = s.execute("document.open", &json!({"path": path}));
    let _ = std::fs::remove_file(&path);
    let opened = opened.unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 5})).expect("everything left can be moved");
    (s, opened["warnings"].clone())
}

#[test]
fn an_object_beyond_the_canvas_is_removed_on_open() {
    let (s, warnings) = open_and_move(&document(true), "object");
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 1, "the far rectangle is gone, the other stays");
    assert!(warnings.to_string().contains("1 object beyond the canvas"), "{warnings}");
}

#[test]
fn an_artboard_a_guide_or_a_ruler_origin_beyond_the_canvas_no_longer_blocks_edits() {
    let mut v = document(false);
    let mut far = v["document"]["artboards"][0].clone();
    far["id"] = json!(9);
    far["rect"] = json!({"x0": 0.0, "y0": 1e17, "x1": 100.0, "y1": 1e17 + 100.0});
    v["document"]["artboards"].as_array_mut().unwrap().push(far);
    v["document"]["guides"] = json!([{"vertical": false, "pos": 1e17}, {"vertical": true, "pos": 50.0}]);
    v["document"]["ruler_origin"] = json!([1e17, 0.0]);
    let (s, warnings) = open_and_move(&v, "setup");
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.guides.len(), d.ruler_origin.x), (1, 1, 0.0));
    assert!(warnings.to_string().contains("1 artboard, 1 guide beyond the canvas"), "{warnings}");
    assert!(warnings.to_string().contains("ruler origin"), "{warnings}");
}

#[test]
fn a_file_whose_artboards_all_lie_beyond_the_canvas_is_refused() {
    let mut v = document(false);
    v["document"]["artboards"][0]["rect"] = json!({"x0": 0.0, "y0": 1e17, "x1": 100.0, "y1": 1e17 + 100.0});
    let path = std::env::temp_dir().join(format!("vectorcraft-beyond-canvas-pages-{}.vectorcraft", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = Session::new().execute("document.open", &json!({"path": path}));
    let _ = std::fs::remove_file(&path);
    let e = r.unwrap_err().to_string();
    assert!(e.contains("artboards all lie beyond the canvas"), "{e}");
}
