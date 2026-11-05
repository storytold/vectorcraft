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

/// A `ui.pointer` release sent right after a `ui.key` waits until the key reached the busy tool:
/// C pressed while an arc is dragged out closes the arc drawn on release.
#[test]
fn a_pointer_release_after_a_key_waits_for_the_key() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("tool.select", json!({"tool": "arc"})).unwrap();
    let ctx = egui::Context::default();
    control(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 100, "y": 100}, {"kind": "drag", "x": 200, "y": 180}]}));
    let (key, _key_reply) = ControlRequest::new("ui.key", json!({"key": "c"}));
    let (up, up_reply) = ControlRequest::new("ui.pointer", json!({"events": [{"kind": "up", "x": 200, "y": 180}]}));
    tx.send(key).unwrap();
    tx.send(up).unwrap();
    // Each frame as the host runs it: the input hook, the control requests, then the UI pass.
    let mut released = None;
    for _ in 0..8 {
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut raw);
        app.drain_control(&ctx);
        if let Ok(r) = up_reply.try_recv() {
            released = Some(r);
            break;
        }
        let mut out = ctx.run_ui(raw, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    let r = released.expect("the release ran");
    assert_eq!(r["ok"], true, "{r}");
    let document = app.session.active().unwrap();
    let arc = *document.selection.objects.last().expect("the arc drawn on release");
    let closed = document.doc.node(arc).unwrap().path_data().unwrap().subpaths[0].closed;
    assert!(closed, "C reached the Arc tool before the release");
}

/// A waiting `ui.pointer` request whose caller gave it up is dropped at once: the requests sent
/// after it run without waiting for the input sent before it to be applied.
#[test]
fn a_given_up_pointer_does_not_hold_back_later_requests() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let (key, _key_reply) = ControlRequest::new("ui.key", json!({"key": "c"}));
    let (up, up_reply) = ControlRequest::new("ui.pointer", json!({"events": [{"kind": "up", "x": 200, "y": 180}]}));
    let ticket = up.ticket();
    let (list, list_reply) = ControlRequest::new("ui.tool.list", json!({}));
    tx.send(key).unwrap();
    tx.send(up).unwrap();
    tx.send(list).unwrap();
    app.drain_control(&ctx);
    assert!(list_reply.try_recv().is_err(), "the pointer request waits for the key, ahead of the list");
    assert!(ticket.give_up(), "the waiting pointer request had not started");
    // The key is still queued: no UI pass ran.
    app.drain_control(&ctx);
    let r = list_reply.try_recv().expect("the list ran past the given-up pointer request");
    assert_eq!(r["ok"], true, "{r}");
    assert!(up_reply.try_recv().is_err(), "the given-up pointer request did not run");
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
