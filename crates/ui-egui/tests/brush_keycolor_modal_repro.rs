//! Brush Options' Key Color pick through the whole app frame: a real canvas click samples the art
//! while the dialog stays open, the release starts no stroke, and OK saves the sampled key.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use craft::ui::runtime::env::WindowKey;
use craft::ui::shell::{WindowDialogs, embed};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;
use vectorcraft_ui_egui::VectorcraftApp;

const BRUSH: &str = "Original key-color fixture";

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> egui::FullOutput {
    let raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), ..Default::default() };
    embed::before_input(ctx, |out| app.frame_input(out));
    ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    })
}

fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    for _ in 0..4 {
        frame(app, ctx, vec![]).textures_delta.clear();
    }
}

fn sample_press(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, modifiers: Modifiers) {
    frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]).textures_delta.clear();
    frame(app, ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]).textures_delta.clear();
}

fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, modifiers: Modifiers) {
    sample_press(app, ctx, at, modifiers);
    frame(app, ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]).textures_delta.clear();
    frame(app, ctx, vec![Event::ModifiersChanged(Modifiers::NONE)]).textures_delta.clear();
}

/// The screen bounds of the accessible nodes `pick` chooses, as the next frames report them.
fn bounds_of(app: &mut VectorcraftApp, ctx: &egui::Context, pick: impl Fn(&egui::accesskit::Node) -> bool) -> Vec<Rect> {
    for _ in 0..3 {
        let mut output = frame(app, ctx, vec![]);
        output.textures_delta.clear();
        let Some(update) = output.platform_output.accesskit_update else { continue };
        let found: Vec<Rect> = update
            .nodes
            .iter()
            .filter(|(_, node)| pick(node))
            .filter_map(|(_, node)| node.bounds())
            .map(|b| Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

fn click_control(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let bounds = bounds_of(app, ctx, |node| node.label() == Some(label));
    let control = bounds.first().copied().unwrap_or_else(|| panic!("missing Brush Options control: {label}"));
    click_at(app, ctx, control.center(), Modifiers::NONE);
    settle(app, ctx);
}

/// Runs `f` on the open Brush Options.
fn with_options<R>(app: &VectorcraftApp, ctx: &egui::Context, f: impl FnOnce(&mut HostedParamDialog) -> R) -> R {
    let id = app.ui.brush_options.expect("Brush Options is open");
    WindowDialogs::of(ctx, &WindowKey::main()).with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(f)).expect("Brush Options is open")
}

/// The dialog's Key Color: the brush's colour as the dialog opened with it, or the one the eyedropper sampled.
fn key_color(app: &VectorcraftApp, ctx: &egui::Context) -> Color {
    with_options(app, ctx, |dialog| match dialog.fields().get("keyColor") {
        Some(FieldValue::Json(value)) => serde_json::from_value(value.clone()).ok(),
        Some(FieldValue::Color([r, g, b])) => Some(Color::rgb(*r, *g, *b)),
        _ => None,
    })
    .expect("Key Color field")
}

fn open_options(app: &mut VectorcraftApp, ctx: &egui::Context) {
    app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
    settle(app, ctx);
    assert_eq!(key_color(app, ctx), Color::BLACK, "the original brush has a black key, distinct from the blue canvas sample");
}

/// Turns on the Key Color row's eyedropper, as a click on it does; the pick arms the Eyedropper tool.
fn arm_pick(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let id = app.ui.brush_options.expect("Brush Options is open");
    WindowDialogs::of(ctx, &WindowKey::main()).with(|host| host.set_json_field(ctx, id, "keyColorPick", json!(true))).unwrap();
    settle(app, ctx);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert_eq!(app.session.tool_options()["brushKeyColor"], true);
}

/// A document with a hue-shift art brush whose key is black and blue art at the returned screen
/// point, Paintbrush chosen and Brush Options of the brush open.
fn modal_fixture() -> (VectorcraftApp, egui::Context, Pos2) {
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    let art = app.run("shape.rectangle", json!({"x": 300, "y": 30, "width": 20, "height": 12})).unwrap()["id"].clone();
    app.run(
        "brush.new",
        json!({"type": "art", "name": BRUSH, "ids": [art], "params": {"colorization": {"method": "hueShift", "key": Color::BLACK}}}),
    )
    .unwrap();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 120, "width": 80, "height": 60})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
    ctx.enable_accesskit();
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![]).textures_delta.clear();
    }
    let view = app.view_mut().unwrap();
    view.zoom = 3.0;
    view.center = Point::new(200.0, 150.0);
    view.rotation = 0.0;
    settle(&mut app, &ctx);
    let canvas = app.canvas_view().expect("the canvas laid out");
    let at = canvas.to_screen(Point::new(60.0, 150.0));
    app.run("paint.setFill", json!({"color": "#e020c0"})).unwrap();
    app.select_tool("paintbrush");
    open_options(&mut app, &ctx);
    let dialogs = bounds_of(&mut app, &ctx, |node| node.role() == egui::accesskit::Role::Dialog);
    assert!(canvas.rect.contains(at) && dialogs.iter().all(|dialog| !dialog.contains(at)), "the art is on the canvas, outside the dialog");
    (app, ctx, at)
}

fn document_defaults(app: &VectorcraftApp) -> (serde_json::Value, vectorcraft_tools::PaintDefaults) {
    let state = app.session.active().unwrap();
    (
        json!({"document": state.doc, "selection": state.selection, "revision": state.revision,
            "undo": state.history.undo.len(), "redo": state.history.redo.len()}),
        app.session.paint.clone(),
    )
}

#[test]
fn key_color_sampling_reaches_the_canvas_while_brush_options_stays_open() {
    let (mut app, ctx, at) = modal_fixture();
    let before = document_defaults(&app);
    arm_pick(&mut app, &ctx);
    assert_eq!(app.session.tool_options()["stop"], "paintbrush");
    click_at(&mut app, &ctx, at, Modifiers::NONE);
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app, &ctx).to_hex(), "#336699", "the canvas click under the open dialog samples the art");
    assert_eq!(app.session.tool_id(), "paintbrush", "the pick returns to the tool that armed it");
    assert_eq!(app.session.tool_options_of("eyedropper")["brushKeyColor"], false);
    assert!(app.ui.brush_key_color_pick.is_none());
    assert_eq!(document_defaults(&app), before, "only the dialog's Key Color changes");
}

#[test]
fn key_color_release_samples_once_without_lending_or_sending_up_to_paintbrush() {
    let (mut app, ctx, at) = modal_fixture();
    arm_pick(&mut app, &ctx);
    let before = document_defaults(&app);
    let modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    sample_press(&mut app, &ctx, at, modifiers);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(2.0, 0.0))]).textures_delta.clear();
    frame(&mut app, &ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]).textures_delta.clear();
    settle(&mut app, &ctx);
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert!(!app.session.pointer_held(), "the restored Paintbrush gets no press to hold");
    assert!(!app.session.in_interaction(), "no stroke starts");
    assert_eq!(key_color(&app, &ctx).to_hex(), "#336699");
    assert_eq!(document_defaults(&app), before);
}

#[test]
fn key_color_ok_saves_the_sampled_brush_key_and_reopened_cancel_preserves_it() {
    let (mut app, ctx, at) = modal_fixture();
    let before = document_defaults(&app);
    let selection = app.session.active().unwrap().selection.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    arm_pick(&mut app, &ctx);
    click_at(&mut app, &ctx, at, Modifiers::NONE);
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app, &ctx).to_hex(), "#336699");
    assert_eq!(document_defaults(&app), before, "sampling changes only the dialog draft before OK");

    click_control(&mut app, &ctx, "OK");
    assert!(app.ui.brush_options.is_none(), "OK commits and closes Brush Options");
    let state = app.session.active().unwrap();
    assert_eq!(state.selection, selection);
    assert_eq!(state.history.undo.len(), undo + 1, "saving the changed brush definition is one Brush Options edit");
    assert!(state.history.redo.is_empty());
    assert_eq!(app.session.paint, before.1, "OK does not change the default paints");

    app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app, &ctx).to_hex(), "#336699", "reopening reads the sampled key saved by OK");
    let saved = document_defaults(&app);
    click_control(&mut app, &ctx, "Cancel");
    assert!(app.ui.brush_options.is_none());
    assert_eq!(document_defaults(&app), saved, "Cancel leaves the stored definition, history and default paints intact");
    app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app, &ctx).to_hex(), "#336699", "Cancel preserves the stored sampled key");
}
