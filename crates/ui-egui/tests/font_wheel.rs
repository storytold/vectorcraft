//! Wheel routing when a font field disappears before the next layout pass.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::{Event, MouseWheelSource, MouseWheelUnit, Pos2, RawInput, Rect, Shape, TouchPhase, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

#[test]
fn hiding_control_between_passes_leaves_its_old_fields_wheel_available() {
    assert!(control_wheel_available_after("window.control", json!({})));
}

#[test]
fn hiding_properties_preserves_the_visible_control_fields_wheel() {
    assert!(!control_wheel_available_after("window.panel.close", json!({"panel": "properties"})));
}

#[test]
fn deselecting_type_leaves_the_missing_control_fields_wheel_available() {
    assert!(control_wheel_available_after("select.set", json!({"ids": []})));
}

#[test]
fn a_queued_font_wheel_does_not_restyle_a_new_object_selection() {
    assert_queued_wheel_discarded_after_target_change(false);
}

#[test]
fn a_queued_font_wheel_does_not_restyle_a_new_character_range() {
    assert_queued_wheel_discarded_after_target_change(true);
}

fn assert_queued_wheel_discarded_after_target_change(range: bool) {
    let (ctx, mut app) = selected_type_with_control();
    let first = app.session.active().unwrap().selection.objects.first().unwrap().0;
    let second = app.run("text.create", json!({"x": 30, "y": 120, "text": "Other text", "font": "Inter"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [first]})).unwrap();
    if range {
        app.select_tool("type");
        app.session.set_tool_option("textSelection", &json!({"editing": first, "anchor": 0, "caret": 2}));
    }
    let mut texts = Vec::new();
    for _ in 0..8 {
        texts = frame(&mut app, &ctx, vec![]);
    }
    let point = texts.iter().find(|(t, p)| t == "Inter" && p.y < 150.0).unwrap().1;
    let mut raw = wheel_input(point);
    app.raw_input_hook(&ctx, &mut raw);
    assert!(!raw.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })), "the original field captured its turn");
    if range {
        app.session.set_tool_option("textSelection", &json!({"editing": first, "anchor": 2, "caret": 4}));
    } else {
        app.run("select.set", json!({"ids": [second]})).unwrap();
    }
    let before = app.session.active().unwrap().history.undo.len();
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.undo.len(), before, "the captured turn cannot restyle the new target");
}

#[test]
fn a_font_field_that_appears_under_a_still_pointer_leaves_the_wheel_until_the_pointer_moves() {
    let (ctx, mut app) = selected_type_with_control();
    let mut texts = Vec::new();
    for _ in 0..8 {
        texts = frame(&mut app, &ctx, vec![]);
    }
    let point = texts.iter().find(|(t, p)| t == "Inter" && p.y < 150.0).unwrap().1;
    let selected = app.session.active().unwrap().selection.objects.first().unwrap().0;
    app.run("select.set", json!({"ids": []})).unwrap();
    frame(&mut app, &ctx, vec![Event::PointerMoved(point)]);
    frame(&mut app, &ctx, vec![]);
    app.run("select.set", json!({"ids": [selected]})).unwrap();
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![]);
    }
    let mut still = wheel_input(point);
    still.events.remove(0);
    app.raw_input_hook(&ctx, &mut still);
    assert!(still.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })), "the pointer did not move after the field appeared");
    let mut out = ctx.run_ui(still, |ui| app.ui(ui));
    out.textures_delta.clear();
    let mut moved = wheel_input(point + vec2(1.0, 0.0));
    moved.time = Some(ctx.input(|i| i.time) + 5.0);
    app.raw_input_hook(&ctx, &mut moved);
    assert!(!moved.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })), "the pointer moved onto the field");
}

#[test]
fn artboard_controls_leave_replaced_font_fields_wheel_available() {
    let (ctx, mut app) = selected_type_with_control();
    let mut texts = Vec::new();
    for _ in 0..8 {
        texts = frame(&mut app, &ctx, vec![]);
    }
    let fields: Vec<_> = texts.iter().filter(|(t, _)| t == "Inter").map(|(_, point)| *point).collect();
    assert_eq!(fields.len(), 2, "Control and Properties each show a font field");
    app.select_tool("artboard");
    for point in fields {
        let mut raw = wheel_input(point);
        app.raw_input_hook(&ctx, &mut raw);
        assert!(raw.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })), "the Artboard tool replaces this font field");
    }
}

#[test]
fn the_control_character_popover_keeps_its_visible_font_wheel() {
    let (ctx, mut app) = selected_type_with_control();
    app.run("window.panel.close", json!({"panel": "character"})).unwrap();
    let mut texts = Vec::new();
    for _ in 0..8 {
        texts = frame(&mut app, &ctx, vec![]);
    }
    let link = texts.iter().find(|(t, p)| t == "Character:" && p.y < 100.0).unwrap().1;
    let button = |pressed| Event::PointerButton { pos: link, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(link), button(true)]);
    frame(&mut app, &ctx, vec![button(false)]);
    for _ in 0..3 {
        texts = frame(&mut app, &ctx, vec![]);
    }
    let point = texts
        .iter()
        .find(|(t, p)| t == "Inter" && p.y > link.y + 10.0 && p.y < link.y + 100.0 && (p.x - link.x).abs() < 100.0)
        .expect("the Character popover shows its family field")
        .1;
    let mut raw = wheel_input(point);
    app.raw_input_hook(&ctx, &mut raw);
    assert!(!raw.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })), "the visible popover keeps font wheel input");
    app.run("window.control", json!({})).unwrap();
    let mut raw = wheel_input(point);
    app.raw_input_hook(&ctx, &mut raw);
    assert!(raw.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })), "hiding Control also hides its popover");
}

fn selected_type_with_control() -> (egui::Context, VectorcraftApp) {
    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 250})).unwrap();
    let text = app.run("text.create", json!({"x": 30, "y": 70, "text": "Control wheel", "font": "Inter"})).unwrap();
    app.run("select.set", json!({"ids": [text["id"]]})).unwrap();
    app.run("window.control", json!({})).unwrap();
    (ctx, app)
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Pos2)> {
    let mut raw = RawInput { events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    let texts = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) => Some((t.galley.text().to_string(), t.pos + vec2(4.0, 6.0))),
            _ => None,
        })
        .collect();
    out.textures_delta.clear();
    texts
}

fn control_wheel_available_after(command: &str, params: Value) -> bool {
    let (ctx, mut app) = selected_type_with_control();
    let mut texts = Vec::new();
    for _ in 0..8 {
        texts = frame(&mut app, &ctx, vec![]);
    }
    let point = texts.iter().find(|(t, p)| t == "Inter" && p.y < 150.0).expect("the Control bar's family field is rendered").1;
    app.run(command, params).unwrap();
    let mut raw = wheel_input(point);
    app.raw_input_hook(&ctx, &mut raw);
    raw.events.iter().any(|e| matches!(e, Event::MouseWheel { .. }))
}

fn wheel_input(point: Pos2) -> RawInput {
    RawInput {
        events: vec![
            Event::PointerMoved(point),
            Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                delta: vec2(0.0, -1.0),
                phase: TouchPhase::Move,
                modifiers: Default::default(),
                source: MouseWheelSource::Unknown,
            },
        ],
        ..Default::default()
    }
}
