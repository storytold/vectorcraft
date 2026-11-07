//! SVG Options and hidden layers: Save As SVG keeps them (hidden), Export As leaves them out, and
//! a save with Preserve Editing Capabilities reopens as the document.

use serde_json::{Value, json};

use crate::tests_svg::app;

#[test]
fn save_as_svg_keeps_hidden_layers_and_export_as_leaves_them_out() {
    let (mut app, written) = app("/tmp/layers.svg");
    let notes = app.run("layer.new", json!({"name": "Notes"})).unwrap()["id"].clone();
    app.run("shape.ellipse", json!({"x": 50, "y": 40, "width": 20, "height": 20})).unwrap();
    app.run("layer.setProps", json!({"id": notes, "visible": false})).unwrap();
    app.run("file.export.svg", Value::Null).unwrap();
    crate::dialogs::confirm(&mut app).unwrap();
    app.run("file.saveAs", Value::Null).unwrap();
    let d = app.ui.dialog.as_mut().expect("SVG Options");
    d.fields.insert("preserveEditing".into(), json!(true));
    crate::dialogs::confirm(&mut app).unwrap();
    let text = |i: usize| String::from_utf8(written.borrow()[i].1.clone()).unwrap();
    assert!(!text(0).contains("Notes"), "exported: {}", text(0));
    assert!(text(1).contains("id=\"Notes\"") && text(1).contains("display:none"), "saved: {}", text(1));
    // Reopened, the saved SVG is the document again.
    let before = app.session.doc().unwrap().doc.layers.clone();
    let saved = text(1);
    app.session.execute("document.open", &json!({"name": "layers.svg", "dataBase64": vectorcraft_format::base64_encode(saved.as_bytes())})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.layers, before);
}
