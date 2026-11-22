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
