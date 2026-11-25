//! Generator plug-ins in Object › Plug-ins: listed with the filters, a generator with parameters
//! opens dialog `pluginGenerate`, whose OK draws new generated art, and Edit Generated Art… opens
//! it with a selected object's parameters and draws the object again. Each test installs a
//! generator of its own and removes it: plug-ins are process-wide.

use craft::menu::Node;
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat::{self as tpl, Given};

use super::tests_export::{cancel, fields, frame, kind, set, try_confirm, try_set};
use super::*;
use crate::Services;
use crate::VectorcraftApp;

/// The generator's art: a 10 pt square, and a 20 pt one when its `size` is 20.
const SMALL: &str = r#"{"objects":[{"type":"path","name":"Small","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]},{"p":[0,10]}]}]}}]}"#;
const BIG: &str = r#"{"objects":[{"type":"path","name":"Big","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[20,0]},{"p":[20,20]},{"p":[0,20]}]}]}}]}"#;

/// An app with a window, where the dialogs open, and generator `id` installed.
fn app(id: &str) -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
    let manifest = tpl::generator(id, r#"{"size":{"type":"int","min":1,"max":100,"default":10}}"#);
    let wasm = wat::parse_str(tpl::when_given_else(&manifest, Given::Params, r#""size":20"#, BIG, SMALL)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    app
}

/// The names of the art of generated art object `id`.
fn art(app: &VectorcraftApp, id: u64) -> Vec<String> {
    let st = app.session.doc().unwrap();
    let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    n.children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect()
}

/// The undo steps of the active document.
fn steps(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

#[test]
fn object_plug_ins_draws_and_edits_generated_art() {
    let mut app = app("test.ui.generator");
    let menu = plugin::object_menu();
    let listed = menu.iter().any(|n| {
        matches!(n, Node::Item(item) if item.command.id == "plugin.dialog"
            && item.label == "Test test.ui.generator…"
            && item.command.params["id"] == "test.ui.generator")
    });
    assert!(listed, "{menu:?}");
    let edit_listed =
        menu.iter().any(|n| matches!(n, Node::Item(item) if item.command.id == plugin::EDIT_COMMAND && item.label == plugin::EDIT_GENERATED));
    assert!(edit_listed, "{menu:?}");

    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    assert!(!plugin::one_generated_selected(&app));
    app.run("plugin.dialog", json!({"id": "test.ui.generator"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some(plugin::GENERATE_KIND));
    assert_eq!(fields(&app)["size"].as_f64(), Some(10.0));
    frame(&mut app);
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none());
    assert!(plugin::one_generated_selected(&app));
    let id = app.session.doc().unwrap().selection.objects[0].0;
    let before = steps(&app);
    assert_eq!(art(&app, id), ["Small"]);

    // Edit Generated Art… shows the saved parameters; OK draws the art again as one undo step.
    app.run(plugin::EDIT_COMMAND, json!({})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some(plugin::GENERATE_KIND));
    assert_eq!(fields(&app)["size"].as_f64(), Some(10.0));
    frame(&mut app);
    set(&mut app, "size", json!(20));
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none());
    assert_eq!(art(&app, id), ["Big"]);
    assert_eq!(steps(&app), before + 1);
    assert_eq!(app.session.doc().unwrap().history.undo.last().map(|s| s.label.as_str()), Some("Edit Generated Art"));

    // A value out of the parameter's range is refused, and Cancel leaves the art as it is.
    app.run(plugin::EDIT_COMMAND, json!({})).unwrap();
    frame(&mut app);
    assert_eq!(fields(&app)["size"].as_f64(), Some(20.0));
    assert!(try_set(&mut app, "size", json!(500)).is_err());
    cancel(&mut app);
    assert!(kind(&app).is_none());
    assert_eq!(art(&app, id), ["Big"]);
    assert_eq!(steps(&app), before + 1);

    // Without its plug-in, the object can't be drawn again: the error names it.
    app.run("plugin.remove", json!({"id": "test.ui.generator"})).unwrap();
    let e = app.run(plugin::EDIT_COMMAND, json!({})).unwrap_err();
    assert!(e.contains("“Test test.ui.generator” (test.ui.generator) isn't installed"), "{e}");
    assert!(kind(&app).is_none());
    assert_eq!(art(&app, id), ["Big"]);
}

/// Edit Generated Art… is for one selected object: a generated art object selected with another
/// object is not. OK in its dialog draws the object of the document it was opened in: after another
/// document comes to the front, OK draws nothing there, closes the dialog and says why.
#[test]
fn edit_generated_art_is_for_one_object_of_the_document_it_opened_in() {
    let mut app = app("test.ui.generator2");
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
    let before = steps(&app);
    app.ui.status.clear();
    try_confirm(&mut app).unwrap();
    assert!(kind(&app).is_none());
    assert!(app.ui.status.contains("the document it was prepared for is no longer active"), "{}", app.ui.status);
    assert_eq!(art(&app, second), ["Small"]);
    assert_eq!(steps(&app), before);
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(art(&app, first), ["Small"]);
    app.run("plugin.remove", json!({"id": "test.ui.generator2"})).unwrap();
}
