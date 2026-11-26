use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::addons::dialogs::tests::{dialog_id, frame, open_item, read, set};

/// A window's app with one type object, selected.
fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("text.create", json!({"x": 40, "y": 100, "text": "abc", "size": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app
}

#[test]
fn another_attribute_starts_from_its_own_values() {
    let mut app = app();
    open_item(&mut app, "Type Tools", "Text Tweaker…");
    let dialog = dialog_id(&app);
    set(&app, dialog, "attribute", json!("rotation"));
    frame(&mut app);
    assert_eq!(read(&app, dialog, |fields| (fields.number("from"), fields.number("to"))), (Some(0.0), Some(30.0)));
}
