//! Control-channel input (`ui.key`, `ui.click`, `ui.drag`) holds its modifiers in egui's input
//! state for the frames it spans, so handlers reading `i.modifiers` see them as they see the
//! keyboard's.

use egui::Modifiers;
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::control::{ControlRequest, Outcome};

/// Send one control request (it must succeed).
pub(crate) fn control(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (req, _rx) = ControlRequest::new(method, params);
    let Outcome::Done(r) = crate::control::handle(app, ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], true, "{r}");
}

/// Frames through the host's input hook until the synthetic queue is drained, and one more (the
/// keyboard's modifiers come back); `draw` lays out what the input reaches. → the modifiers
/// held in each frame.
pub(crate) fn run_synthetic(app: &mut VectorcraftApp, ctx: &egui::Context, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<Modifiers> {
    let mut held = vec![];
    loop {
        let last = app.synthetic.is_empty();
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            held.push(ui.input(|i| i.modifiers));
            draw(app, ui);
        });
        out.textures_delta.clear();
        if last {
            return held;
        }
    }
}

fn keys(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::shortcuts::handle(app, ui.ctx());
}

/// A document with one selected 10 × 10 square at (10, 10) → (app, its id).
fn one_square() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    (app, NodeId(id))
}

fn top_left(app: &VectorcraftApp, id: NodeId) -> (f64, f64) {
    let b = app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    (b.x0, b.y0)
}

fn object_count(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().doc.layers[0].children().unwrap().len()
}

#[test]
fn shift_arrow_nudges_ten_points_and_alt_arrow_copies() {
    let (mut app, id) = one_square();
    let ctx = egui::Context::default();
    control(&mut app, &ctx, "ui.key", json!({"key": "ArrowDown", "shift": true}));
    let held = run_synthetic(&mut app, &ctx, keys);
    assert_eq!(top_left(&app, id), (10.0, 20.0), "Shift+↓ nudges by 10 pt");
    assert!(held.first().is_some_and(|m| m.shift));
    assert_eq!(held.last(), Some(&Modifiers::NONE), "the keyboard's modifiers come back");
    control(&mut app, &ctx, "ui.key", json!({"key": "ArrowRight", "alt": true}));
    run_synthetic(&mut app, &ctx, keys);
    assert_eq!(object_count(&app), 2, "Alt+→ nudges a copy");
}

#[test]
fn a_typed_key_holds_its_modifiers_for_its_text() {
    let (mut app, _) = one_square();
    let ctx = egui::Context::default();
    control(&mut app, &ctx, "ui.key", json!({"key": "B", "shift": true, "text": "B"}));
    run_synthetic(&mut app, &ctx, keys);
    assert_eq!(app.session.tool_id(), "blobBrush", "Shift+B, not B");
}

#[test]
fn a_drag_holds_its_modifiers_from_the_press_to_the_release() {
    let (mut app, _) = one_square();
    let ctx = egui::Context::default();
    control(&mut app, &ctx, "ui.drag", json!({"x": 10, "y": 10, "toX": 50, "toY": 50, "steps": 3, "shift": true}));
    let held = run_synthetic(&mut app, &ctx, |_, _| {});
    let shift: Vec<bool> = held.iter().map(|m| m.shift).collect();
    // Move to the start, press, three moves, release, then the keyboard's modifiers again.
    assert_eq!(shift, [false, true, true, true, true, true, false]);
}

#[test]
fn a_shift_click_on_a_layers_selection_square_adds_to_the_selection() {
    let (mut app, a) = one_square();
    let b = NodeId(app.run("shape.rectangle", json!({"x": 40, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let ctx = egui::Context::default();
    // Rows top down: the layer, then `b`; a row's selection square is 17 pt right of its target circle.
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::panels::layers::show(&mut app, ui));
    out.textures_delta.clear();
    let mut circles: Vec<egui::Pos2> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(cs) if cs.radius == 5.0 => Some(cs.center),
            _ => None,
        })
        .collect();
    circles.sort_by(|p, q| p.y.total_cmp(&q.y));
    let square = circles[1] + egui::vec2(17.0, 0.0);
    control(&mut app, &ctx, "ui.click", json!({"x": square.x, "y": square.y, "shift": true}));
    run_synthetic(&mut app, &ctx, crate::panels::layers::show);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![a, b], "added, not replaced");
}
