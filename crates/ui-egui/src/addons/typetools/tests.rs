use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, menus};

/// An app with one type object, selected.
fn app() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 40, "y": 100, "text": "abc", "size": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    (app, id)
}

#[allow(dead_code)]
fn sizes(app: &VectorcraftApp, id: NodeId) -> Vec<f64> {
    match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().flat_map(|r| r.text.chars().map(|_| r.style.size).collect::<Vec<_>>()).collect(),
        _ => panic!("not text"),
    }
}

fn open(app: &mut VectorcraftApp) {
    let entries = menus::menu_entries(app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Type Tools"] && e.label == "Text Tweaker…").unwrap();
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &id, p);
}

// PLAN: assumed `app.ui_ctx: egui::Context` (lib.rs:203) and `OpenDialog.id` (host.rs:385)
fn set(app: &VectorcraftApp, key: &str, value: Value) {
    let ctx = app.ui_ctx.clone();
    app.dialogs()
        .expect("window host")
        .with(|host| {
            let id = host.open_dialogs().first().map(|d| d.id).expect("a dialog is open");
            host.set_json_field(&ctx, id, key, value)
        })
        .unwrap();
}

#[test]
fn another_attribute_starts_from_its_own_values() {
    let (mut app, _) = app();
    open(&mut app);
    set(&app, "attribute", json!("rotation"));
    let ctx = app.ui_ctx.clone();
    crate::dialogs::host::pump(&mut app, &ctx);
    let (from, to) = app.dialogs().expect("window host").with(|host| {
        let fields = host.open_dialogs().first().and_then(|d| d.fields.clone()).expect("a dialog is open");
        (fields["from"].as_f64(), fields["to"].as_f64())
    });
    assert_eq!((from, to), (Some(0.0), Some(30.0)));
}
