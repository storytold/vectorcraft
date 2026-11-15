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

#[test]
fn visible_yes_and_no_buttons_control_unused_deletion() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let name = app.run("brush.list", json!({})).unwrap()["brushes"][0]["name"].as_str().unwrap().to_string();
    let ctx = context();
    let before = app.session.active().unwrap().history.undo.len();
    ask(&mut app, json!({"name": name}), false);
    press(&ctx, &mut app, "No", 0.0);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
    assert!(app.run("brush.get", json!({"name": name})).is_ok());
    ask(&mut app, json!({"name": name}), false);
    press(&ctx, &mut app, "Yes", 1.0);
    assert!(app.run("brush.get", json!({"name": name})).is_err());
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
}

#[test]
fn yes_asks_again_when_a_stroke_started_using_the_brush() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let name = "10 pt. Oval";
    let ctx = context();
    ask(&mut app, json!({"names": [name]}), false);
    let path = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap());
    app.run("brush.apply", json!({"name": name})).unwrap();
    let before = app.session.active().unwrap().history.undo.len();
    press(&ctx, &mut app, "Yes", 0.0);
    assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.bool("used")), "asks again with Remove Strokes");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
    let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
    assert_eq!(stroke.brush.as_deref(), Some(name));
    press(&ctx, &mut app, "Remove Strokes", 1.0);
    assert!(app.run("brush.get", json!({"name": name})).is_err());
}

#[test]
fn yes_deletes_nothing_after_another_document_became_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let name = app.run("brush.list", json!({})).unwrap()["brushes"][0]["name"].as_str().unwrap().to_string();
    let ctx = context();
    ask(&mut app, json!({"name": name}), false);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    press(&ctx, &mut app, "Yes", 0.0);
    assert!(app.ui.dialog.is_none());
    assert!(app.run("brush.get", json!({"name": name})).is_ok(), "the second document keeps the brush");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(app.run("brush.get", json!({"name": name})).is_ok(), "the first document keeps the brush");
}

#[test]
fn visible_remove_strokes_and_cancel_buttons_control_used_deletion() {
    for choice in ["Cancel", "Remove Strokes"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let path = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap());
        let name = "10 pt. Oval";
        app.run("brush.apply", json!({"name": name})).unwrap();
        let before = app.session.active().unwrap().history.undo.len();
        let ctx = context();
        ask(&mut app, json!({"name": name}), true);
        press(&ctx, &mut app, choice, 0.0);
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.active().unwrap().history.undo.len(), before + usize::from(choice != "Cancel"));
        let stroke = app.session.active().unwrap().doc.node(path).and_then(|n| n.appearance.stroke()).unwrap();
        assert_eq!(stroke.brush.as_deref(), (choice == "Cancel").then_some(name));
        assert!(stroke.width > 0.0);
    }
}
