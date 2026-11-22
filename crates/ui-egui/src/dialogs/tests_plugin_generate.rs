//! Generator plug-ins in Object › Plug-ins: listed with the filters, a generator with parameters
//! opens dialog `pluginGenerate`, whose OK draws new generated art, and Edit Generated Art… opens
//! it with a selected object's parameters and draws the object again. The test installs a
//! generator of its own and removes it: plug-ins are process-wide.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat::{self as tpl, Given};

use super::tests_export::{frame, kind, set};
use super::*;
use crate::Services;
use crate::menus::Item;

/// The generator's art: a 10 pt square, and a 20 pt one when its `size` is 20.
const SMALL: &str = r#"{"objects":[{"type":"path","name":"Small","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]},{"p":[0,10]}]}]}}]}"#;
const BIG: &str = r#"{"objects":[{"type":"path","name":"Big","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[20,0]},{"p":[20,20]},{"p":[0,20]}]}]}}]}"#;

/// The names of the art of generated art object `id`.
fn art(app: &VectorcraftApp, id: u64) -> Vec<String> {
    let st = app.session.doc().unwrap();
    let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    n.children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect()
}

#[test]
fn object_plug_ins_draws_and_edits_generated_art() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let manifest = tpl::generator("test.ui.generator", r#"{"size":{"type":"int","min":1,"max":100,"default":10}}"#);
    let wasm = wat::parse_str(tpl::when_given_else(&manifest, Given::Params, r#""size":20"#, BIG, SMALL)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    let menu = plugin::object_menu();
    let listed = menu
        .iter()
        .any(|i| matches!(i, Item::Cmd(label, "plugin.dialog", p) if *label == "Test test.ui.generator…" && p["id"] == "test.ui.generator"));
    assert!(listed, "{menu:?}");
    assert!(menu.iter().any(|i| matches!(i, Item::Cmd(plugin::EDIT_GENERATED, plugin::EDIT_COMMAND, _))), "{menu:?}");

    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    assert!(!plugin::one_generated_selected(&app));
    app.run("plugin.dialog", json!({"id": "test.ui.generator"})).unwrap();
    assert_eq!(kind(&app), Some(plugin::GENERATE_KIND));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(plugin::one_generated_selected(&app));
    let st = app.session.doc().unwrap();
    let id = st.selection.objects[0].0;
    let steps = st.history.undo.len();
    assert_eq!(art(&app, id), ["Small"]);

    // Edit Generated Art… shows the saved parameters; OK draws the art again as one undo step.
    app.run(plugin::EDIT_COMMAND, json!({})).unwrap();
    assert_eq!(kind(&app), Some(plugin::GENERATE_KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields.get("size"), d.fields.get("__edit")), (Some(&json!(10)), Some(&json!(id))));
    frame(&mut app);
    set(&mut app, "size", json!(20));
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(art(&app, id), ["Big"]);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1);
    assert_eq!(app.session.doc().unwrap().history.undo.last().map(|s| s.label.as_str()), Some("Edit Generated Art"));

    // A bad value keeps the dialog open with the error, and the art as it is.
    app.run(plugin::EDIT_COMMAND, json!({})).unwrap();
    set(&mut app, "size", json!(500));
    assert!(confirm(&mut app).is_err());
    assert_eq!(art(&app, id), ["Big"]);
    app.ui.dialog = None;

    // Without its plug-in, the object can't be drawn again: the error names it.
    app.run("plugin.remove", json!({"id": "test.ui.generator"})).unwrap();
    let e = app.run(plugin::EDIT_COMMAND, json!({})).unwrap_err();
    assert!(e.contains("“Test test.ui.generator” (test.ui.generator) isn't installed"), "{e}");
    assert_eq!(kind(&app), None);
    assert_eq!(art(&app, id), ["Big"]);
}

/// Edit Generated Art… is for one selected object: a generated art object selected with another
/// object is not. OK in its dialog draws the object of the document it was opened in: after another
/// document comes to the front, OK draws nothing there, closes the dialog and says why.
#[test]
fn edit_generated_art_is_for_one_object_of_the_document_it_opened_in() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let manifest = tpl::generator("test.ui.generator2", r#"{"size":{"type":"int","min":1,"max":100,"default":10}}"#);
    let wasm = wat::parse_str(tpl::when_given_else(&manifest, Given::Params, r#""size":20"#, BIG, SMALL)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    let generate = |app: &mut VectorcraftApp| {
        app.run("plugin.generate", json!({"id": "test.ui.generator2", "params": {}})).unwrap();
        app.session.doc().unwrap().selection.objects[0].0
    };

    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let first = generate(&mut app);
    let square = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap();
    app.run("select.set", json!({"ids": [first, square["id"]]})).unwrap();
    assert!(!plugin::one_generated_selected(&app), "a generated art object and a rectangle");
    app.run("select.set", json!({"ids": [first]})).unwrap();
    assert!(plugin::one_generated_selected(&app));

    app.run(plugin::EDIT_COMMAND, json!({})).unwrap();
    frame(&mut app);
    set(&mut app, "size", json!(20));
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let second = generate(&mut app);
    let steps = app.session.doc().unwrap().history.undo.len();
    let e = confirm(&mut app).unwrap_err();
    assert_eq!(e, "the document it was prepared for is no longer active");
    assert!(app.ui.dialog.is_none());
    assert_eq!(art(&app, second), ["Small"]);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps);
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(art(&app, first), ["Small"]);
    app.run("plugin.remove", json!({"id": "test.ui.generator2"})).unwrap();
}
