//! Control-channel input (`ui.key`, `ui.pointer`) and the whole-frame helper the key tests drive.

use craft::automation::control::{ControlRequest, Reply};
use egui::Modifiers;
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// Send one control request (it must succeed).
pub(crate) fn control(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (req, _rx) = ControlRequest::new(method, params);
    let Reply::Done(r) = crate::control::handle(app, ctx, &req) else { panic!("not done") };
    assert_eq!(r["ok"], true, "{r}");
}

/// One whole app frame (`tick` and `ui`, as production runs them) in an 800 × 600 window with
/// `events`, so test keys go through the key stage, the board and `run_fired`. The first pass of a
/// context builds the keymap: a test that presses a key at once runs one empty frame first.
pub(crate) fn key_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let time = ctx.input(|i| i.time) + 0.05;
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        focused: true,
        time: Some(time),
        events,
        ..Default::default()
    };
    crate::testing::app_frame(app, ctx, raw);
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

/// A press and release of `key` with `modifiers` held.
fn chord(key: egui::Key, modifiers: Modifiers) -> Vec<egui::Event> {
    let k = |pressed| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
    vec![egui::Event::ModifiersChanged(modifiers), k(true), k(false), egui::Event::ModifiersChanged(Modifiers::NONE)]
}

#[test]
fn shift_arrow_nudges_ten_points_and_alt_arrow_copies() {
    let (mut app, id) = one_square();
    let ctx = crate::testing::ui_context();
    key_frame(&mut app, &ctx, vec![]);
    key_frame(&mut app, &ctx, chord(egui::Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!(top_left(&app, id), (10.0, 20.0), "Shift+↓ nudges by 10 pt");
    key_frame(&mut app, &ctx, chord(egui::Key::ArrowRight, Modifiers::ALT));
    assert_eq!(object_count(&app), 2, "Alt+→ nudges a copy");
}

/// A `ui.pointer` release sent right after a `ui.key` waits until the key reached the busy tool:
/// C pressed while an arc is dragged out closes the arc drawn on release.
#[test]
fn a_pointer_release_after_a_key_waits_for_the_key() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("tool.select", json!({"tool": "arc"})).unwrap();
    let ctx = crate::testing::ui_context();
    key_frame(&mut app, &ctx, vec![]);
    control(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 100, "y": 100}, {"kind": "drag", "x": 200, "y": 180}]}));
    let (key, _key_reply) = ControlRequest::new("ui.key", json!({"key": "c"}));
    let (up, up_reply) = ControlRequest::new("ui.pointer", json!({"events": [{"kind": "up", "x": 200, "y": 180}]}));
    tx.send(key).unwrap();
    tx.send(up).unwrap();
    let mut released = None;
    for _ in 0..8 {
        key_frame(&mut app, &ctx, vec![]);
        if let Ok(r) = up_reply.try_recv() {
            released = Some(r);
            break;
        }
    }
    let r = released.expect("the release ran");
    assert_eq!(r["ok"], true, "{r}");
    let document = app.session.active().unwrap();
    let arc = *document.selection.objects.last().expect("the arc drawn on release");
    let closed = document.doc.node(arc).unwrap().path_data().unwrap().subpaths[0].closed;
    assert!(closed, "C reached the Arc tool before the release");
}
