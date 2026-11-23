//! Regression coverage for Brush Options Key Color sampling through the real modal canvas route.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};
use vectorcraft_ui_egui::{VectorcraftApp, canvas};

const BRUSH: &str = "Original key-color fixture";
const PICK_KEY: &str = "Eyedropper: click the art to sample the key color";

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> egui::FullOutput {
    let mut raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
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

fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, modifiers: Modifiers) {
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]).textures_delta.clear();
    frame(app, ctx, vec![press(true)]).textures_delta.clear();
    frame(app, ctx, vec![press(false)]).textures_delta.clear();
    frame(app, ctx, vec![Event::ModifiersChanged(Modifiers::NONE)]).textures_delta.clear();
}

fn click_control(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let mut bounds = None;
    for _ in 0..3 {
        let mut output = frame(app, ctx, vec![]);
        if let Some(update) = &output.platform_output.accesskit_update {
            bounds = update.nodes.iter().find(|(_, node)| node.label() == Some(label)).and_then(|(_, node)| node.bounds());
        }
        output.textures_delta.clear();
        if bounds.is_some() {
            break;
        }
    }
    let bounds = bounds.unwrap_or_else(|| panic!("missing actual Brush Options control: {label}"));
    let at = egui::pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);
    click_at(app, ctx, at, Modifiers::NONE);
    settle(app, ctx);
}

fn key_color(app: &VectorcraftApp) -> Color {
    let dialog = app.ui.dialog.as_ref().expect("Brush Options remains open");
    assert_eq!(dialog.kind, "brushOptions");
    assert_eq!(dialog.fields.get("colorization"), Some(&json!("hueShift")));
    vectorcraft_engine::cmd::color_value(dialog.fields.get("keyColor").expect("Key Color field")).unwrap()
}

fn open_options(app: &mut VectorcraftApp, ctx: &egui::Context) {
    app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
    settle(app, ctx);
    assert_eq!(key_color(app), Color::BLACK, "the original brush has a black key, distinct from the blue canvas sample");
}

#[test]
fn key_color_sampling_reaches_the_canvas_while_brush_options_stays_open() {
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
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![]).textures_delta.clear();
    }
    let view = app.view_mut().unwrap();
    view.zoom = 3.0;
    view.center = Point::new(200.0, 150.0);
    view.rotation = 0.0;
    settle(&mut app, &ctx);
    let point = Point::new(60.0, 150.0);
    let at = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(point);
    assert!(app.canvas_rect.unwrap().contains(at));

    // Positive control: this exact coordinate is colored art and the ordinary real UI route hits it.
    let control_document = app.session.active().unwrap().doc.clone();
    app.run("paint.setFill", json!({"color": "#e020c0"})).unwrap();
    app.select_tool("eyedropper");
    click_at(&mut app, &ctx, at, Modifiers::SHIFT);
    let expected_key = match &app.session.paint.fill {
        Paint::Solid { color, .. } => *color,
        _ => panic!("ordinary Eye sampled a solid color"),
    };
    assert_eq!(expected_key.to_hex(), "#336699", "ordinary real UI Eyedropper hits the sample point");
    assert_eq!(app.session.active().unwrap().doc, control_document);
    assert!(app.session.active().unwrap().selection.objects.is_empty());
    app.run("paint.setFill", json!({"color": "#e020c0"})).unwrap();
    app.select_tool("paintbrush");
    let state = app.session.active().unwrap();
    let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
    let history = (state.history.undo.len(), state.history.redo.len());
    let paint = app.session.paint.clone();
    let unchanged = |app: &VectorcraftApp| {
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document);
        assert_eq!(state.selection, selection);
        assert_eq!(state.revision, revision);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(app.session.paint, paint);
    };

    // Separate callback control, not the failure witness: the existing tool-to-dialog route works.
    open_options(&mut app, &ctx);
    click_control(&mut app, &ctx, PICK_KEY);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert_eq!(app.session.tool_options()["brushKeyColor"], true);
    let view = app.view_info();
    canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, point.x, point.y), view);
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app), expected_key, "the callback can update this hue-shift dialog");
    assert_eq!(app.session.tool_id(), "paintbrush");
    unchanged(&app);
    click_control(&mut app, &ctx, "Cancel");
    assert!(app.ui.dialog.is_none());
    unchanged(&app);

    // Reopen from the unchanged brush so the real canvas witness starts with the original black key.
    open_options(&mut app, &ctx);
    click_control(&mut app, &ctx, PICK_KEY);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert_eq!(app.session.tool_options()["brushKeyColor"], true);
    assert_eq!(app.session.tool_options()["stop"], "paintbrush");
    let fields = app.ui.dialog.as_ref().unwrap().fields.clone();
    let modal = ctx.memory(|memory| memory.area_rect(egui::Id::unique(("dialog", "brushOptions")))).expect("the real Brush Options window bounds");
    let now = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(point);
    assert!(now.distance(at) < 0.1, "the control and witness use the same canvas coordinate");
    assert!(!modal.contains(at), "sample visible art outside the dialog body, not a covered control");
    unchanged(&app);
    click_at(&mut app, &ctx, at, Modifiers::NONE);
    settle(&mut app, &ctx);
    unchanged(&app);
    let actual_key = key_color(&app);
    assert_eq!(actual_key, expected_key, "the armed Key Color eyedropper must receive the real canvas click through the retained dialog");
    let mut expected_fields = fields;
    expected_fields.insert("keyColor".into(), serde_json::to_value(expected_key).unwrap());
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields, expected_fields, "only the dialog Key Color changes");
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert_eq!(app.session.tool_options_of("eyedropper")["brushKeyColor"], false);
}

// Additional controls are appended; the independently reproduced witness above is unchanged.
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
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![]).textures_delta.clear();
    }
    let view = app.view_mut().unwrap();
    view.zoom = 3.0;
    view.center = Point::new(200.0, 150.0);
    view.rotation = 0.0;
    settle(&mut app, &ctx);
    let at = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(Point::new(60.0, 150.0));
    app.run("paint.setFill", json!({"color": "#e020c0"})).unwrap();
    app.select_tool("paintbrush");
    open_options(&mut app, &ctx);
    let modal = ctx.memory(|memory| memory.area_rect(egui::Id::unique(("dialog", "brushOptions")))).unwrap();
    assert!(app.canvas_rect.unwrap().contains(at) && !modal.contains(at));
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

fn escape(app: &mut VectorcraftApp, ctx: &egui::Context) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                Event::ModifiersChanged(Modifiers::NONE),
                Event::Key { key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed, repeat: false, modifiers: Modifiers::NONE },
            ],
        )
        .textures_delta
        .clear();
    }
    settle(app, ctx);
}

fn sample_press(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, modifiers: Modifiers) {
    frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]).textures_delta.clear();
    frame(app, ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]).textures_delta.clear();
}

#[test]
fn unarmed_brush_and_other_modal_keep_blocking_canvas_input() {
    for other_modal in [false, true] {
        let (mut app, ctx, at) = modal_fixture();
        if other_modal {
            click_control(&mut app, &ctx, "Cancel");
            app.run("ui.newBrush", json!({})).unwrap();
            settle(&mut app, &ctx);
        }
        app.select_tool("eyedropper");
        let before = document_defaults(&app);
        click_at(&mut app, &ctx, at, Modifiers::SHIFT);
        settle(&mut app, &ctx);
        assert_eq!(document_defaults(&app), before, "an unarmed modal must still block the same canvas point");
        if !other_modal {
            assert_eq!(key_color(&app), Color::BLACK);
        }
        click_control(&mut app, &ctx, "Cancel");
        assert!(app.ui.dialog.is_none());
        click_at(&mut app, &ctx, at, Modifiers::SHIFT);
        match &app.session.paint.fill {
            Paint::Solid { color, .. } => assert_eq!(color.to_hex(), "#336699", "the same point works after the modal closes"),
            _ => panic!("ordinary Eye returns a solid color"),
        }
    }
}

#[test]
fn key_color_reentry_escape_cancel_and_tool_choice_restore_only_the_owned_pick() {
    let (mut app, ctx, _) = modal_fixture();
    let before = document_defaults(&app);
    click_control(&mut app, &ctx, PICK_KEY);
    assert_eq!(app.session.tool_options()["stop"], "paintbrush");
    click_control(&mut app, &ctx, PICK_KEY);
    assert_eq!(app.session.tool_id(), "paintbrush", "reentry must not capture Eye as its own return tool");
    assert_eq!(key_color(&app), Color::BLACK);
    click_control(&mut app, &ctx, PICK_KEY);
    escape(&mut app, &ctx);
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert_eq!(key_color(&app), Color::BLACK, "the first Escape ends sampling and keeps the draft");
    escape(&mut app, &ctx);
    assert!(app.ui.dialog.is_none(), "the second Escape closes Brush Options");
    open_options(&mut app, &ctx);
    click_control(&mut app, &ctx, PICK_KEY);
    click_control(&mut app, &ctx, "Cancel");
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert!(app.ui.dialog.is_none());
    open_options(&mut app, &ctx);
    click_control(&mut app, &ctx, PICK_KEY);
    app.select_tool("rectangle");
    settle(&mut app, &ctx);
    assert_eq!(app.session.tool_id(), "rectangle", "an explicit choice wins over automatic restoration");
    assert_eq!(key_color(&app), Color::BLACK);
    assert_eq!(document_defaults(&app), before);
}

#[test]
fn key_color_release_samples_once_without_lending_or_sending_up_to_paintbrush() {
    let (mut app, ctx, at) = modal_fixture();
    click_control(&mut app, &ctx, PICK_KEY);
    let before = document_defaults(&app);
    let inputs = app.session.input_count();
    let modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    sample_press(&mut app, &ctx, at, modifiers);
    assert_eq!(app.session.tool_id(), "eyedropper", "Cmd/Ctrl must not lend Selection in this foreground pick");
    assert_eq!(app.session.input_count(), inputs, "the physical Down is retained until its release");
    assert_eq!(key_color(&app), Color::BLACK);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(2.0, 0.0))]).textures_delta.clear();
    assert_eq!(app.session.input_count(), inputs, "holding/moving the pick must not start a stroke");
    frame(&mut app, &ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]).textures_delta.clear();
    assert_eq!(app.session.input_count(), inputs + 2, "only the existing Eye Down and Eye Up reach Session");
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert!(!app.session.pointer_held());
    assert!(!app.session.in_interaction());
    assert_eq!(key_color(&app).to_hex(), "#336699");
    assert_eq!(document_defaults(&app), before);
}

#[test]
fn key_color_lost_press_or_replaced_target_cannot_commit_on_a_later_up() {
    for exit in ["focus", "pointerGone", "dialog", "document"] {
        let (mut app, ctx, at) = modal_fixture();
        click_control(&mut app, &ctx, PICK_KEY);
        sample_press(&mut app, &ctx, at, Modifiers::NONE);
        assert_eq!(key_color(&app), Color::BLACK);
        match exit {
            "focus" => {
                let mut raw =
                    RawInput { focused: false, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), ..Default::default() };
                app.raw_input_hook(&ctx, &mut raw);
                ctx.run_ui(raw, |ui| {
                    app.tick(ui.ctx());
                    app.ui(ui);
                })
                .textures_delta
                .clear();
            }
            "pointerGone" => {
                frame(&mut app, &ctx, vec![Event::PointerGone]).textures_delta.clear();
            }
            "dialog" => {
                app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
            }
            "document" => {
                app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            }
            _ => panic!("unexpected test exit: {exit}"),
        }
        settle(&mut app, &ctx);
        assert_eq!(app.session.tool_id(), "paintbrush", "{exit}: invalidated picks restore the original tool");
        let before = document_defaults(&app);
        let fields = app.ui.dialog.as_ref().map(|dialog| dialog.fields.clone());
        frame(
            &mut app,
            &ctx,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
            ],
        )
        .textures_delta
        .clear();
        settle(&mut app, &ctx);
        assert_eq!(document_defaults(&app), before, "{exit}: an unrelated Up cannot commit a ghost sample or stroke");
        assert_eq!(app.ui.dialog.as_ref().map(|dialog| dialog.fields.clone()), fields, "{exit}: no late callback changes the draft");
    }
}

#[test]
fn key_color_ok_saves_the_sampled_brush_key_and_reopened_cancel_preserves_it() {
    let (mut app, ctx, at) = modal_fixture();
    let before = document_defaults(&app);
    let selection = app.session.active().unwrap().selection.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    click_control(&mut app, &ctx, PICK_KEY);
    click_at(&mut app, &ctx, at, Modifiers::NONE);
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app).to_hex(), "#336699");
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert_eq!(document_defaults(&app), before, "sampling changes only the dialog draft before OK");

    click_control(&mut app, &ctx, "OK");
    assert!(app.ui.dialog.is_none(), "the actual OK button commits and closes Brush Options");
    let state = app.session.active().unwrap();
    assert_eq!(state.selection, selection);
    assert_eq!(state.history.undo.len(), undo + 1, "saving the changed brush definition is one Brush Options edit");
    assert!(state.history.redo.is_empty());
    assert_eq!(app.session.paint, before.1, "OK does not change the default paints");

    // Open from the saved brush, rather than the black-key fixture helper.
    app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app).to_hex(), "#336699", "reopening reads the sampled key saved by OK");
    let saved = document_defaults(&app);
    click_control(&mut app, &ctx, "Cancel");
    assert!(app.ui.dialog.is_none());
    assert_eq!(document_defaults(&app), saved, "Cancel leaves the stored definition, history and default paints intact");
    app.run("ui.brushOptions", json!({"name": BRUSH})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(key_color(&app).to_hex(), "#336699", "Cancel preserves the stored sampled key");
    assert_eq!(document_defaults(&app), saved);
}
