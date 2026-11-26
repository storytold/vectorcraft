//! Scrubbing numeric fields by dragging their label (#400) in the app: the preference, Escape, and one undo step.
//! The step sizes, limits and modifiers are the framework's `ValueDrag`.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::{DocState, Session};

use crate::VectorcraftApp;

/// One pass of the whole window.
// PLAN: assumed hub-w1's `ui_frame` (which sets `ScrubDrags` in `AppValues`) replaces these two calls.
pub(super) fn window(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    app.tick(ui.ctx());
    app.ui(ui);
}

/// One frame of the whole window with `events`. → The rect of the first text drawn as `text`.
fn app_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, text: &str) -> Option<Rect> {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0)));
    let out = crate::testing::app_frame(app, ctx, egui::RawInput { events, screen_rect, ..Default::default() });
    fn find(shape: &egui::Shape, text: &str) -> Option<Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    out.shapes.iter().find_map(|s| find(&s.shape, text))
}

fn app_with_rect() -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    (app, crate::testing::ui_context())
}

fn width(app: &VectorcraftApp) -> f64 {
    app.session.transform_box(&app.session.doc().unwrap().selection.objects).unwrap().rect.width()
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn objects(doc: &DocState) -> usize {
    doc.doc.layers[0].children().map_or(0, |c| c.len())
}

/// Drag the W label by `dx` in four moves (`escape`: Escape before the release).
// The Transform panel's W field scrubs by its label, "W".
fn drag_w(app: &mut VectorcraftApp, ctx: &egui::Context, dx: f32, escape: bool) {
    let mut w = None;
    for _ in 0..4 {
        w = app_frame(app, ctx, vec![], "W");
    }
    let at = w.expect("the W label is shown").center();
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    app_frame(app, ctx, vec![Event::PointerMoved(at)], "");
    app_frame(app, ctx, vec![button(at, true)], "");
    for i in 1..=4 {
        app_frame(app, ctx, vec![Event::PointerMoved(at + vec2(dx * i as f32 / 4.0, 0.0))], "");
    }
    if escape {
        app_frame(app, ctx, vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], "");
    }
    app_frame(app, ctx, vec![button(at + vec2(dx, 0.0), false)], "");
    app_frame(app, ctx, vec![], "");
}

/// With Scrub Numeric Fields on a drag of the W label changes the width and Escape puts it back; off, the same drag
/// changes nothing.
#[test]
fn escape_puts_the_value_back_and_the_preference_turns_scrubbing_off() {
    let (mut app, ctx) = app_with_rect();
    drag_w(&mut app, &ctx, 40.0, false);
    assert!(width(&app) > 100.0, "on: {}", width(&app));
    app.run("edit.undo", json!({})).unwrap();
    assert!((width(&app) - 100.0).abs() < 1e-6);

    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));

    app.run("prefs.set", json!({"key": "scrubNumericFields", "value": false})).unwrap();
    let undo = undo_len(&app);
    drag_w(&mut app, &ctx, 40.0, false);
    assert!((width(&app) - 100.0).abs() < 1e-6, "off: {}", width(&app));
    assert_eq!(undo_len(&app), undo, "off: no step");
}

/// A scrub of the W label resizes the selection live and is one undo step; Escape takes it back without a step (and
/// without deselecting).
#[test]
fn scrubbing_w_in_the_app_is_one_undo_step() {
    let (mut app, ctx) = app_with_rect();
    let undo = undo_len(&app);
    drag_w(&mut app, &ctx, 40.0, false);
    let grown = width(&app);
    assert!(grown > 100.0, "{grown}");
    assert_eq!(undo_len(&app), undo + 1, "one undo step for the drag");
    app.run("edit.undo", json!({})).unwrap();
    assert!((width(&app) - 100.0).abs() < 1e-6);

    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));
    assert_eq!(undo_len(&app), undo, "no step");
    assert!(!app.session.doc().unwrap().selection.objects.is_empty(), "Escape didn't deselect");
}

/// A scrub does not close an undo group that is open in another document, and that document keeps its edits, when
/// Escape ends the drag and when the drag is released.
#[test]
fn a_scrub_leaves_another_documents_undo_group_open() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The second document exists before the group begins, so no group opens in it.
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    app.session.execute("edit.undoGroup.begin", &json!({})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 100, "height": 50})).unwrap();
    app.session.execute("document.activate", &json!({"index": 1})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let ctx = crate::testing::ui_context();
    let other = |app: &VectorcraftApp| (app.session.documents()[0].undo_group.is_some(), objects(&app.session.documents()[0]));
    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));
    assert_eq!(other(&app), (true, 1), "Escape");
    drag_w(&mut app, &ctx, 40.0, false);
    assert!(width(&app) > 100.0, "{}", width(&app));
    assert_eq!(other(&app), (true, 1), "released");
}

/// A scrub in a document whose undo group is already open joins that group's undo step and leaves the group open;
/// Escape puts the document back as it was before the drag, with the edits made in the group, and drops the drag's
/// journal entries.
#[test]
fn a_scrub_inside_an_open_undo_group_joins_its_step() {
    let (mut app, ctx) = app_with_rect();
    app.run("edit.undoGroup.begin", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 100, "height": 50})).unwrap();
    let undo = undo_len(&app);
    let (before, journal) = (app.session.doc().unwrap().doc.clone(), app.session.journal.len());
    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));
    assert!(std::sync::Arc::ptr_eq(&app.session.doc().unwrap().doc, &before), "Escape puts the document back");
    assert_eq!(app.session.journal.len(), journal, "and drops the drag's journal entries");
    assert_eq!(objects(app.session.doc().unwrap()), 2, "the rectangle drawn in the group stays");
    drag_w(&mut app, &ctx, 40.0, false);
    assert!(width(&app) > 100.0, "{}", width(&app));
    assert!(app.session.doc().unwrap().undo_group.is_some(), "the group stays open");
    assert_eq!(undo_len(&app), undo, "the drag joins the group's step");
    app.run("edit.undoGroup.end", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(objects(app.session.doc().unwrap()), 1, "one undo takes back the rectangle and the drag");
}

/// A scrub closes the undo group it opened in its own document when another document becomes active during the drag
/// (as `document.activate` does), and leaves that document's group open.
#[test]
fn a_scrub_closes_its_group_in_its_own_document_after_a_switch() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The second document exists before the group begins, so no group opens in it.
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    app.session.execute("edit.undoGroup.begin", &json!({})).unwrap();
    app.session.execute("document.activate", &json!({"index": 1})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let ctx = crate::testing::ui_context();
    let undo = undo_len(&app);
    let mut w = None;
    for _ in 0..4 {
        w = app_frame(&mut app, &ctx, vec![], "W");
    }
    let at = w.expect("the W label is shown").center();
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    app_frame(&mut app, &ctx, vec![Event::PointerMoved(at)], "");
    app_frame(&mut app, &ctx, vec![button(at, true)], "");
    for i in 1..=4 {
        app_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(10.0 * i as f32, 0.0))], "");
    }
    app.session.set_active(0);
    app_frame(&mut app, &ctx, vec![button(at + vec2(40.0, 0.0), false)], "");
    app_frame(&mut app, &ctx, vec![], "");
    let second = &app.session.documents()[1];
    assert!(second.undo_group.is_none(), "the scrub's group is closed");
    assert_eq!(second.history.undo.len(), undo + 1, "one undo step for the drag");
    assert!(app.session.documents()[0].undo_group.is_some(), "the active document's group stays open");
}

/// With the Artboard tool, a scrub of the W label resizes the active artboard live, runs to its release as one undo
/// step, and Escape takes it back without a step.
#[test]
fn scrubbing_an_artboards_w_is_one_undo_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("artboard");
    let ctx = crate::testing::ui_context();
    let board = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[0].rect.width();
    let undo = undo_len(&app);
    drag_w(&mut app, &ctx, 40.0, false);
    assert!(board(&app) > 400.0, "the release keeps the scrub: {}", board(&app));
    assert_eq!(undo_len(&app), undo + 1, "one undo step for the drag");
    app.run("edit.undo", json!({})).unwrap();
    assert!((board(&app) - 400.0).abs() < 1e-6);

    drag_w(&mut app, &ctx, 40.0, true);
    assert!((board(&app) - 400.0).abs() < 1e-6, "Escape: {}", board(&app));
    assert_eq!(undo_len(&app), undo, "no step");
}
