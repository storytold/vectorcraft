//! The shared dialog frame at every UI scale (Preferences → User Interface → UI Scaling zooms the
//! whole interface, so a window holds fewer points).

use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

/// The save prompt's rect after a few frames in a window of `physical` pixels at UI scale `zoom`.
fn prompt(app: &mut VectorcraftApp, ctx: &egui::Context, physical: egui::Vec2, zoom: f32) -> (egui::Rect, egui::Rect) {
    ctx.set_zoom_factor(zoom);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, physical / zoom);
    for _ in 0..10 {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        ctx.run_ui(input, |ui| show(app, ui.ctx())).textures_delta.clear();
    }
    let rect = ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", crate::unsaved::KIND)))).unwrap();
    (rect, screen)
}

#[test]
fn the_save_prompt_fits_its_content_and_the_window_at_every_ui_scale() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    for name in ["Untitled-1", "Quarterly campaign poster, final revision with the printer's notes"] {
        app.ui.dialog = Some(Dialog::new(crate::unsaved::KIND, json!({"index": 0, "name": name, "then": "close"})));
        // Scaled up and down while the prompt is open, in a full-size and in the smallest window.
        for (w, h, zoom) in
            [(1440.0, 900.0, 1.0), (1440.0, 900.0, 2.0), (1440.0, 900.0, 1.5), (800.0, 500.0, 2.0), (800.0, 500.0, 1.0), (1440.0, 900.0, 0.75)]
        {
            let (rect, screen) = prompt(&mut app, &ctx, egui::vec2(w, h), zoom);
            let at = format!("{name}: {w} × {h} px at {zoom}×: {rect:?} in {screen:?}");
            assert!(screen.contains_rect(rect), "inside the window, {at}");
            // As tall as its text and buttons (no empty band around the buttons).
            assert!(rect.height() < 200.0, "{at}");
        }
    }
}

/// A dialog is a movable window: dragging its heading moves it, and it then stays where it was
/// dropped (frame after frame).
#[test]
fn dialogs_can_be_dragged_somewhere_else_and_stay_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.dialog = Some(Dialog::new(crate::unsaved::KIND, json!({"index": 0, "name": "Untitled-1", "then": "close"})));
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    let id = egui::Id::new(("dialog", crate::unsaved::KIND));
    let mut frame = |events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        ctx.run_ui(input, |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        ctx.memory(|m| m.area_rect(id)).unwrap()
    };
    for _ in 0..3 {
        frame(vec![]);
    }
    let before = frame(vec![]);
    // Grab the heading (just inside the top-left corner) and drag 200 pt left and 150 up.
    let grab = before.min + egui::vec2(30.0, 12.0);
    let to = grab - egui::vec2(200.0, 150.0);
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(grab)]);
    frame(vec![press(grab, true)]);
    for i in 1..=10 {
        frame(vec![egui::Event::PointerMoved(grab + (to - grab) * (i as f32 / 10.0))]);
    }
    frame(vec![press(to, false)]);
    let after = frame(vec![]);
    let moved = after.min - before.min;
    assert!((moved.x + 200.0).abs() < 2.0 && (moved.y + 150.0).abs() < 2.0, "moved by {moved:?}");
    for _ in 0..3 {
        frame(vec![]);
    }
    assert_eq!(frame(vec![]).min, after.min, "it stays where it was dropped");
}
