use serde_json::json;
use vectorcraft_doc::NodeId;

use super::super::brush_delete::tests::{answer, app};
use super::*;

fn rect(app: &mut VectorcraftApp, x: f64, y: f64, w: f64, h: f64) -> u64 {
    app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap()
}

fn art_height(app: &mut VectorcraftApp, name: &str) -> f64 {
    let art: vectorcraft_doc::Node = serde_json::from_value(app.run("brush.get", json!({"name": name})).unwrap()["art"].clone()).unwrap();
    art.geometric_bounds().unwrap().height()
}

/// Is the Replace Brush Art question open?
fn asks(app: &VectorcraftApp) -> bool {
    app.dialogs().unwrap().with(|host| host.get::<ReplaceArtQuestion>().is_some())
}

/// An art brush "Bar" (40 × 8 pt) painting a path, and a 40 × 20 pt rectangle: → (path, rectangle).
fn used_brush(app: &mut VectorcraftApp) -> (NodeId, u64) {
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let bar = rect(app, 10.0, 10.0, 40.0, 8.0);
    app.run("brush.new", json!({"type": "art", "name": "Bar", "ids": [bar]})).unwrap();
    let path = NodeId(rect(app, 100.0, 100.0, 80.0, 40.0));
    app.run("brush.apply", json!({"name": "Bar", "ids": [path.0]})).unwrap();
    (path, rect(app, 10.0, 40.0, 40.0, 20.0))
}

#[test]
fn an_unused_brush_takes_the_art_without_asking() {
    let mut app = app();
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let bar = rect(&mut app, 10.0, 10.0, 40.0, 8.0);
    app.run("brush.new", json!({"type": "art", "name": "Bar", "ids": [bar]})).unwrap();
    let tall = rect(&mut app, 10.0, 40.0, 40.0, 20.0);
    replace(&mut app, "Bar", &[tall]);
    assert!(!asks(&app));
    assert!((art_height(&mut app, "Bar") - 20.0).abs() < 1e-6);
}

/// Each answer of the question runs its command: Apply to Strokes gives the brush the new art and
/// the strokes redraw with it, Leave Strokes adds a copy and keeps the brush, Cancel changes nothing.
#[test]
fn each_answer_runs_its_command() {
    // (answer, undo steps added, height of Bar, height of Bar copy)
    let table = [(Some("proceed"), 1, 20.0, None), (Some("alternate"), 1, 8.0, Some(20.0)), (None, 0, 8.0, None)];
    for (choice, steps, bar, copy) in table {
        let mut app = app();
        let (path, tall) = used_brush(&mut app);
        let before = app.session.active().unwrap().history.undo.len();
        replace(&mut app, "Bar", &[tall]);
        assert!(asks(&app), "strokes use Bar: it asks");
        answer::<ReplaceArtQuestion>(&mut app, choice, pump);
        assert!(!asks(&app));
        assert_eq!(app.session.active().unwrap().history.undo.len(), before + steps, "{choice:?}");
        assert!((art_height(&mut app, "Bar") - bar).abs() < 1e-6, "{choice:?}");
        if let Some(copy) = copy {
            assert!((art_height(&mut app, "Bar copy") - copy).abs() < 1e-6);
        }
        let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
        assert_eq!(stroke.brush.as_deref(), Some("Bar"), "{choice:?}");
    }
}

/// Art a used brush cannot take is refused before the question: no dialog, the reason in the status
/// bar, and the document and its undo history as they were.
#[test]
fn art_with_no_width_is_refused_before_it_asks() {
    let mut app = app();
    let _ = used_brush(&mut app);
    let flat = app.run("shape.line", json!({"x1": 50, "y1": 100, "x2": 50, "y2": 160})).unwrap()["id"].as_u64().unwrap();
    let st = app.session.active().unwrap();
    let (doc, steps) = (st.doc.clone(), st.history.undo.len());
    replace(&mut app, "Bar", &[flat]);
    assert!(!asks(&app), "nothing asks");
    assert!(app.ui.status.contains("the selected art has no width"), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none());
    assert_eq!(*st.doc, *doc);
    assert_eq!(st.history.undo.len(), steps);
}

#[test]
fn a_button_changes_no_brush_after_another_document_became_active() {
    let mut app = app();
    let (_, tall) = used_brush(&mut app);
    replace(&mut app, "Bar", &[tall]);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    answer::<ReplaceArtQuestion>(&mut app, Some("proceed"), pump);
    assert!(!asks(&app));
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!((art_height(&mut app, "Bar") - 8.0).abs() < 1e-6);
}
