use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;

use super::KIND;
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// An app with one type object, selected.
fn app() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 40, "y": 100, "text": "abc", "size": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    (app, id)
}

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

#[test]
fn text_tweaker_opens_its_dialog_previews_and_runs_on_ok() {
    let (mut app, id) = app();
    open(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Text Tweaker", "Attribute:", "Font Size", "From:", "To:", "Shape:", "Straight", "Seed:", "Skip Spaces", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(sizes(&app, id), [12.0, 24.0, 36.0], "previewed");
    crate::dialogs::cancel(&mut app);
    assert_eq!(sizes(&app, id), [20.0; 3], "rolled back");
    open(&mut app);
    let n = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1, "one undo step");
    assert_eq!(sizes(&app, id), [12.0, 24.0, 36.0]);
}

#[test]
fn another_attribute_starts_from_its_own_values() {
    let (mut app, _) = app();
    open(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("attribute".into(), json!("rotation"));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields["from"].clone(), d.fields["to"].clone()), (json!(0.0), json!(30.0)));
}
