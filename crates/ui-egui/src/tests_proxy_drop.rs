//! Dragging the Fill square onto the Stroke square (or back) copies its paint there.

use egui::{Event, PointerButton, Pos2, pos2};
use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

const SIZE: f32 = 40.0;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// One frame drawing the proxy at the top left of the window with `events`.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Pos2 {
    let mut origin = Pos2::ZERO;
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        origin = ui.cursor().min;
        crate::panels::proxy(app, ui, SIZE);
    });
    out.textures_delta.clear();
    origin
}

/// Press at `from`, move to `to` over a few frames, release there.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, to: Pos2) {
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(from)]);
    frame(app, ctx, vec![button(from, true)]);
    for i in 1..=4 {
        let p = from + (to - from) * (i as f32 / 4.0);
        frame(app, ctx, vec![Event::PointerMoved(p)]);
    }
    frame(app, ctx, vec![button(to, false)]);
    frame(app, ctx, vec![]);
}

fn paints(app: &VectorcraftApp) -> (Paint, Paint) {
    crate::panels::current_paints(app)
}

#[test]
fn fill_dragged_onto_stroke_copies_it() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("paint.setStroke", json!({"color": "#0000ff"})).unwrap();
    let (fill, stroke) = paints(&app);
    assert_ne!(fill, stroke);
    let ctx = egui::Context::default();
    let o = frame(&mut app, &ctx, vec![]);
    // The fill square's top-left corner and the stroke square's bottom-right one (they overlap
    // in the middle).
    let (on_fill, on_stroke) = (o + egui::vec2(4.0, 4.0), o + egui::vec2(SIZE - 4.0, SIZE - 4.0));
    drag(&mut app, &ctx, on_fill, on_stroke);
    let (f, s) = paints(&app);
    assert_eq!(f, fill, "the fill stays");
    assert_eq!(s, fill, "the stroke takes the fill");
    // And back: the stroke (now red) onto the fill after the fill changed.
    app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
    drag(&mut app, &ctx, on_stroke, on_fill);
    assert_eq!(paints(&app).0, fill);
}

#[test]
fn a_selected_object_takes_the_dropped_paint() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let ctx = egui::Context::default();
    let o = frame(&mut app, &ctx, vec![]);
    drag(&mut app, &ctx, o + egui::vec2(4.0, 4.0), pos2(o.x + SIZE - 4.0, o.y + SIZE - 4.0));
    let (f, s) = paints(&app);
    assert_eq!(s, f, "the rectangle's stroke takes its fill");
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1, "the selection is kept");
}
