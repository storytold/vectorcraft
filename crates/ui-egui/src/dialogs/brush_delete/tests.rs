use craft::ui::dialog::StackDialog;
use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::*;

/// An app with a window context for the dialog host and no document.
pub(in crate::dialogs) fn app() -> VectorcraftApp {
    crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
}

/// Writes a native file for the test to revert to.
pub(in crate::dialogs) fn save_for_revert(app: &mut VectorcraftApp, tag: &str) -> std::path::PathBuf {
    let file = std::env::temp_dir().join(format!("vectorcraft-brush-{tag}-{}.vectorcraft", std::process::id()));
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|err| err.to_string())));
    app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|err| err.to_string())));
    app.run("file.saveAs", json!({"path": file.to_string_lossy()})).unwrap();
    file
}

/// Answers the open question `D` the way its button does (`Some(choice)`: the button of that choice,
/// `None`: Cancel's close), then runs the module's `pump`.
pub(in crate::dialogs) fn answer<D: StackDialog>(app: &mut VectorcraftApp, choice: Option<&str>, pump: fn(&mut VectorcraftApp, &egui::Context)) {
    let ctx = app.ui_ctx.clone().unwrap();
    app.dialogs().unwrap().with(|host| {
        let id = host.get::<D>().expect("the question is open").id();
        match choice {
            Some(choice) => {
                host.set_json_field(&ctx, id, "choice", json!(choice)).unwrap();
                assert!(host.confirm_by_id(&ctx, id));
            }
            None => assert!(host.cancel_by_id(&ctx, id)),
        }
    });
    pump(app, &ctx);
}

/// The `used` flag of the Delete Brush question that is open, if one is.
fn open_question(app: &VectorcraftApp) -> Option<bool> {
    app.dialogs().unwrap().with(|host| host.get::<BrushDeleteQuestion>().and_then(|q| q.0.tag_as::<BrushDeleteAsk>()).map(|ask| ask.used))
}

#[test]
fn yes_asks_again_when_a_stroke_started_using_the_brush() {
    let mut app = app();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let name = "10 pt. Oval";
    ask(&mut app, json!({"names": [name]}), false);
    let path = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap());
    app.run("brush.apply", json!({"name": name})).unwrap();
    let before = app.session.active().unwrap().history.undo.len();
    answer::<BrushDeleteQuestion>(&mut app, Some("proceed"), pump);
    assert_eq!(open_question(&app), Some(true), "asks again with Remove Strokes");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
    let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
    assert_eq!(stroke.brush.as_deref(), Some(name));
    answer::<BrushDeleteQuestion>(&mut app, Some("proceed"), pump);
    assert!(app.run("brush.get", json!({"name": name})).is_err());
}

#[test]
fn yes_deletes_nothing_after_another_document_became_active() {
    let mut app = app();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let name = app.run("brush.list", json!({})).unwrap()["brushes"][0]["name"].as_str().unwrap().to_string();
    ask(&mut app, json!({"name": name}), false);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    answer::<BrushDeleteQuestion>(&mut app, Some("proceed"), pump);
    assert_eq!(open_question(&app), None);
    assert!(app.run("brush.get", json!({"name": name})).is_ok(), "the second document keeps the brush");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(app.run("brush.get", json!({"name": name})).is_ok(), "the first document keeps the brush");
}

/// Remove Strokes runs `brush.delete` and the strokes stay as plain strokes; Cancel changes nothing.
#[test]
fn remove_strokes_runs_the_delete_and_cancel_changes_nothing() {
    for choice in [None, Some("proceed")] {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let path = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap());
        let name = "10 pt. Oval";
        app.run("brush.apply", json!({"name": name})).unwrap();
        let before = app.session.active().unwrap().history.undo.len();
        ask(&mut app, json!({"name": name}), true);
        answer::<BrushDeleteQuestion>(&mut app, choice, pump);
        assert_eq!(open_question(&app), None);
        assert_eq!(app.session.active().unwrap().history.undo.len(), before + usize::from(choice.is_some()));
        let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
        assert_eq!(stroke.brush.as_deref(), choice.is_none().then_some(name));
        assert!(stroke.width > 0.0);
    }
}

#[test]
fn stale_delete_approval_keeps_the_saved_brush_after_revert() {
    let mut app = app();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let slim = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 8})).unwrap()["id"].as_u64().unwrap();
    let tall = app.run("shape.rectangle", json!({"x": 10, "y": 30, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("brush.new", json!({"type": "art", "name": "Bar", "ids": [slim]})).unwrap();
    let file = save_for_revert(&mut app, "delete-revert");
    let brush_height = |app: &mut VectorcraftApp| {
        let brush = app.run("brush.get", json!({"name": "Bar"})).expect("the saved brush still exists");
        let art: vectorcraft_doc::Node = serde_json::from_value(brush["art"].clone()).unwrap();
        art.geometric_bounds().unwrap().height()
    };
    let saved = brush_height(&mut app);
    let st = app.session.active().unwrap();
    let (uid, reverts) = (st.uid, st.reverts);

    app.run("brush.replaceArt", json!({"name": "Bar", "ids": [tall], "applyToStrokes": true})).unwrap();
    assert!((brush_height(&mut app) - saved).abs() > 1e-6);
    ask(&mut app, json!({"names": ["Bar"]}), false);
    app.run("file.revert", json!({"confirmed": true})).unwrap();
    let _ = std::fs::remove_file(file);
    let st = app.session.active().unwrap();
    assert_eq!((st.uid, st.reverts), (uid, reverts + 1));
    assert!((brush_height(&mut app) - saved).abs() < 1e-6);

    let undo = app.session.active().unwrap().history.undo.len();
    answer::<BrushDeleteQuestion>(&mut app, Some("proceed"), pump);
    assert!((brush_height(&mut app) - saved).abs() < 1e-6);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    assert_eq!(app.ui.status, "The document was reverted. Reopen this dialog to continue.");
}
