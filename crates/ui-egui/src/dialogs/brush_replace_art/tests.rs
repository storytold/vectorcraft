use egui::{Pos2, Rect, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::*;

fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, time: f64) -> egui::FullOutput {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
    let input = egui::RawInput { screen_rect, time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| crate::dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
    out
}

fn press(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, time: f64) {
    frame(ctx, app, vec![], time);
    let out = frame(ctx, app, vec![], time + 0.1);
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("dialog accessibility tree").nodes;
    let button = nodes.iter().find(|(_, node)| node.label().map(str::trim) == Some(label));
    let bounds = button.and_then(|(_, node)| node.bounds()).expect("visible button");
    let at = pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(ctx, app, vec![egui::Event::PointerMoved(at), button(true), button(false)], time + 0.2);
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64, w: f64, h: f64) -> u64 {
    app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap()
}

fn art_height(app: &mut VectorcraftApp, name: &str) -> f64 {
    let art: vectorcraft_doc::Node = serde_json::from_value(app.run("brush.get", json!({"name": name})).unwrap()["art"].clone()).unwrap();
    art.geometric_bounds().unwrap().height()
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
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let bar = rect(&mut app, 10.0, 10.0, 40.0, 8.0);
    app.run("brush.new", json!({"type": "art", "name": "Bar", "ids": [bar]})).unwrap();
    let tall = rect(&mut app, 10.0, 40.0, 40.0, 20.0);
    replace(&mut app, "Bar", &[tall]);
    assert!(app.ui.dialog.is_none());
    assert!((art_height(&mut app, "Bar") - 20.0).abs() < 1e-6);
}

#[test]
fn apply_to_strokes_gives_the_brush_the_new_art() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let (path, tall) = used_brush(&mut app);
    replace(&mut app, "Bar", &[tall]);
    assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND), "strokes use Bar: it asks");
    let ctx = context();
    let before = app.session.active().unwrap().history.undo.len();
    press(&ctx, &mut app, "Apply to Strokes", 0.0);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    assert!((art_height(&mut app, "Bar") - 20.0).abs() < 1e-6);
    let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
    assert_eq!(stroke.brush.as_deref(), Some("Bar"));
}

#[test]
fn leave_strokes_adds_a_copy_and_cancel_changes_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let (path, tall) = used_brush(&mut app);
    let ctx = context();
    let before = app.session.active().unwrap().history.undo.len();
    replace(&mut app, "Bar", &[tall]);
    press(&ctx, &mut app, "Cancel", 0.0);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
    assert!((art_height(&mut app, "Bar") - 8.0).abs() < 1e-6);
    replace(&mut app, "Bar", &[tall]);
    press(&ctx, &mut app, "Leave Strokes", 1.0);
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    assert!((art_height(&mut app, "Bar") - 8.0).abs() < 1e-6, "Bar stays as it was");
    assert!((art_height(&mut app, "Bar copy") - 20.0).abs() < 1e-6);
    let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
    assert_eq!(stroke.brush.as_deref(), Some("Bar"));
}

/// Art a used brush cannot take is refused before the question: no dialog, the reason in the status
/// bar, and the document and its undo history as they were.
#[test]
fn art_with_no_width_is_refused_before_it_asks() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let _ = used_brush(&mut app);
    let flat = app.run("shape.line", json!({"x1": 50, "y1": 100, "x2": 50, "y2": 160})).unwrap()["id"].as_u64().unwrap();
    let st = app.session.active().unwrap();
    let (doc, steps) = (st.doc.clone(), st.history.undo.len());
    replace(&mut app, "Bar", &[flat]);
    assert!(app.ui.dialog.is_none(), "nothing asks");
    assert!(app.ui.status.contains("the selected art has no width"), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none());
    assert_eq!(*st.doc, *doc);
    assert_eq!(st.history.undo.len(), steps);
}

#[test]
fn a_button_changes_no_brush_after_another_document_became_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let (_, tall) = used_brush(&mut app);
    let ctx = context();
    replace(&mut app, "Bar", &[tall]);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    press(&ctx, &mut app, "Apply to Strokes", 0.0);
    assert!(app.ui.dialog.is_none());
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!((art_height(&mut app, "Bar") - 8.0).abs() < 1e-6);
}
