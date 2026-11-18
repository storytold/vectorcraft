//! Image Trace Ignore Color's real panel button, tool callback and canvas presentation.
//! All APIs used here exist on the pre-fix base; the first regression fails on the missing button.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, NodeKind, TraceView};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};
use vectorcraft_ui_egui::{CacheKey, VectorcraftApp, canvas, trace_runs};

const SAMPLE_TIP: &str = "Eyedropper: click the source image to sample the ignored color";

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> egui::FullOutput {
    let mut raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1400.0))), ..Default::default() };
    // raw_input_hook already begins the host iteration; do not begin a second one.
    app.raw_input_hook(ctx, &mut raw);
    ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    })
}

fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        trace_runs::poll(app);
        frame(app, ctx, vec![]).textures_delta.clear();
        if !trace_runs::any_running(app) {
            break;
        }
        assert!(Instant::now() < deadline, "the tiny fixture's background trace did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..2 {
        frame(app, ctx, vec![]).textures_delta.clear();
    }
}

fn click_control(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let mut found = None;
    for _ in 0..3 {
        let mut out = frame(app, ctx, vec![]);
        if let Some(update) = &out.platform_output.accesskit_update {
            found = update.nodes.iter().find(|(_, node)| node.label() == Some(label)).and_then(|(_, node)| node.bounds());
        }
        out.textures_delta.clear();
        if found.is_some() {
            break;
        }
    }
    let bounds = found.unwrap_or_else(|| panic!("missing accessible Image Trace control: {label}"));
    let at = egui::pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(at)]).textures_delta.clear();
    frame(app, ctx, vec![press(true)]).textures_delta.clear();
    frame(app, ctx, vec![press(false)]).textures_delta.clear();
    settle(app, ctx);
}

fn fixture(live_trace: bool, rgb: [u8; 3], preview: bool) -> (VectorcraftApp, egui::Context) {
    fixture_with_transparent_edge(live_trace, rgb, preview, false)
}

fn fixture_with_transparent_edge(live_trace: bool, rgb: [u8; 3], preview: bool, transparent: bool) -> (VectorcraftApp, egui::Context) {
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let pixels: Vec<u8> = (0..400)
        .flat_map(|i| {
            if transparent && i % 20 == 19 {
                [0; 4]
            } else if i % 20 < 10 {
                [rgb[0], rgb[1], rgb[2], 255]
            } else {
                [255; 4]
            }
        })
        .collect();
    let image =
        craft_image_codecs::Image::from_raw(20, 20, craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, pixels).unwrap();
    let png = craft_image_codecs::encode(&image, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
    app.run(
        "file.place",
        json!({"name": "sampler-original.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false, "at": [200, 150]}),
    )
    .unwrap();
    if live_trace {
        app.run(
            "imageTrace.make",
            json!({"preset": "Default", "params": {"mode": "blackAndWhite", "threshold": 127, "noise": 1, "ignoreWhite": false}}),
        )
        .unwrap();
    }
    app.select_tool("selection");
    app.ui.image_trace_preview = preview;
    app.ui.image_trace_advanced = true;
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.session.prefs.monitor_profile_display = false;
    app.canvas.worker_started = true; // the existing same-thread CPU canvas route
    // An ordinary dock column gives Advanced the full window height without an auxiliary OS viewport.
    app.run("window.panel.dock", json!({"panel": "imageTrace", "side": "End", "collapsed": false})).unwrap();
    app.run("window.panel.activate", json!({"panel": "imageTrace"})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![]).textures_delta.clear();
    }
    click_control(&mut app, &ctx, "Ignore Color");
    (app, ctx)
}

fn selected(app: &VectorcraftApp) -> Node {
    let state = app.session.active().unwrap();
    state.doc.node(state.selection.objects[0]).unwrap().clone()
}

fn settings(app: &VectorcraftApp) -> Value {
    selected(app).extras().trace.as_deref().unwrap().clone()
}

fn source_point(app: &VectorcraftApp, x: f64) -> Point {
    let selected = selected(app);
    let original = selected.children().and_then(|children| children.first()).map_or(&selected, |child| child.as_ref());
    let NodeKind::Image(image) = &original.kind else { panic!("selected source image") };
    image.xf * Point::new(f64::from(image.width) * x, f64::from(image.height) * 0.5)
}

fn sample(app: &mut VectorcraftApp, ctx: &egui::Context, point: Point) {
    click_control(app, ctx, SAMPLE_TIP);
    assert_eq!(app.session.tool_id(), "eyedropper", "the actual panel button arms sampling");
    let view = app.view_info();
    canvas::dispatch(app, &PointerEvent::new(PointerKind::Down, point.x, point.y), view);
    settle(app, ctx);
}

fn press_armed_canvas(app: &mut VectorcraftApp, ctx: &egui::Context, point: Point) {
    assert_eq!(app.session.tool_id(), "eyedropper", "this is a continuation of the existing armed pick");
    let view = app.view_info();
    canvas::dispatch(app, &PointerEvent::new(PointerKind::Down, point.x, point.y), view);
    canvas::dispatch(app, &PointerEvent::new(PointerKind::Up, point.x, point.y), view);
    settle(app, ctx);
}

#[test]
fn samples_original_trace_pixels_without_painting() {
    let (mut app, ctx) = fixture(true, [37; 3], false);
    let point = source_point(&app, 0.25);
    let state = app.session.active().unwrap();
    let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
    let history = (state.history.undo.len(), state.history.redo.len());
    let paint = app.session.paint.clone();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    assert_eq!(app.session.tool_id(), "eyedropper");
    let at = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(point);
    assert!(app.canvas_rect.unwrap().contains(at));
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]).textures_delta.clear();
    frame(&mut app, &ctx, vec![press(false)]).textures_delta.clear();
    settle(&mut app, &ctx);
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(app.session.paint, paint);
    let state = app.session.active().unwrap();
    assert_eq!(state.doc, document, "Preview off leaves the document unchanged");
    assert_eq!(state.selection, selection);
    assert_eq!(state.revision, revision);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    click_control(&mut app, &ctx, "Trace");
    assert_eq!(settings(&app)["params"]["ignoreColor"], "#252525", "persist the original gray, not the black tracing result");
}

#[test]
fn colored_placed_image_and_live_trace_keep_preview_off_drafts_until_trace() {
    for live_trace in [false, true] {
        let (mut app, ctx) = fixture(live_trace, [37, 83, 149], false);
        let point = source_point(&app, 0.25);
        let id = selected(&app).id;
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        sample(&mut app, &ctx, point);
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document, "placed/live={live_trace}: sample only the panel draft");
        assert_eq!(state.selection, selection);
        assert_eq!(state.revision, revision);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(app.session.paint, paint);
        app.run("select.none", json!({})).unwrap();
        settle(&mut app, &ctx);
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        settle(&mut app, &ctx);
        click_control(&mut app, &ctx, "Trace");
        assert_eq!(settings(&app)["params"]["ignoreColor"], "#255395", "the raw non-gray draft survives the selection round trip");
        assert_eq!(settings(&app)["preset"], "Custom");
        assert_eq!(app.session.active().unwrap().history.undo.len(), history.0 + 1, "Trace creates the only edit");
    }
}

#[test]
fn preview_traces_once_and_undo_redo_restore_the_results() {
    let (mut app, ctx) = fixture(true, [37, 83, 149], true);
    let point = source_point(&app, 0.25);
    let state = app.session.active().unwrap();
    let (before, selection, steps) = (state.doc.clone(), state.selection.clone(), state.history.undo.len());
    let paint = app.session.paint.clone();
    let view = selected(&app).trace_view();
    sample(&mut app, &ctx, point);
    let after = app.session.active().unwrap().doc.clone();
    assert_ne!(after, before);
    assert_eq!(settings(&app)["params"]["ignoreColor"], "#255395");
    assert_eq!(settings(&app)["preset"], "Custom");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);
    // The existing imageTrace.make replaces the trace group and selects its new ID.
    // Keep the logical trace selected, rather than requiring the removed group's ID.
    let mut traced_selection = selection.clone();
    traced_selection.set([selected(&app).id]);
    assert_eq!(app.session.active().unwrap().selection, traced_selection);
    assert!(after.node(selection.objects[0]).is_none());
    assert_eq!(app.session.paint, paint);
    assert_eq!(selected(&app).trace_view(), view);
    assert_eq!(app.session.tool_id(), "selection");
    settle(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "later frames do not retrace");
    app.run("edit.undo", json!({})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().selection, selection);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
    app.run("edit.redo", json!({})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, after);
    assert_eq!(app.session.active().unwrap().selection, traced_selection);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);
}

#[test]
fn escape_and_explicit_tool_choice_do_not_sample_or_paint() {
    for exit in ["escape", "rectangle", "eyedropper"] {
        let (mut app, ctx) = fixture(true, [37, 83, 149], false);
        let old_color = settings(&app)["params"]["ignoreColor"].clone();
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        click_control(&mut app, &ctx, SAMPLE_TIP);
        assert_eq!(app.session.tool_id(), "eyedropper");
        match exit {
            "escape" => {
                let key = |pressed| Event::Key {
                    key: egui::Key::Escape,
                    physical_key: Some(egui::Key::Escape),
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                };
                // Full public input path: claims/ends are observed before tool_key, and
                // Escape's requests are applied only if the tool claims the key.
                frame(&mut app, &ctx, vec![key(true)]).textures_delta.clear();
                frame(&mut app, &ctx, vec![key(false)]).textures_delta.clear();
            }
            chosen => app.select_tool(chosen),
        }
        let expected = if exit == "escape" { "selection" } else { exit };
        assert_eq!(app.session.tool_id(), expected, "{exit}: restore or respect the explicit choice");
        assert!(app.session.tool_options_of("eyedropper")["imageTraceIgnoreColor"].is_null());
        settle(&mut app, &ctx);
        assert!(app.canvas.key.as_ref().unwrap().trace_source.is_none());
        assert_eq!(app.session.paint, paint);
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document);
        assert_eq!(state.selection, selection, "{exit}: Escape does not fall through to deselect");
        assert_eq!(state.revision, revision);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        click_control(&mut app, &ctx, "Trace");
        assert_eq!(settings(&app)["params"]["ignoreColor"], old_color, "{exit}: no hidden draft color was sampled");
    }
}

#[test]
fn outside_and_transparent_misses_stay_armed_until_an_opaque_source_pixel_is_picked() {
    for live_trace in [false, true] {
        let (mut app, ctx) = fixture_with_transparent_edge(live_trace, [37, 83, 149], false, true);
        let hit = source_point(&app, 0.25);
        let outside = source_point(&app, -0.25);
        let transparent = source_point(&app, 0.975);
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        let node = selected(&app);
        let original = if live_trace { node.children().unwrap()[0].as_ref() } else { &node };
        let NodeKind::Image(image) = &original.kind else { panic!("original fixture image") };
        let blob = &state.doc.images[&image.key];
        let keep = |_: f64, _: f64| true;
        assert_eq!(
            blob.sample_pixels(&[(Some([0.95, 0.5, 1.0, 0.55]), 1, &keep)]),
            Some(vec![vec![[0; 4]]]),
            "the missed point is really transparent in the source"
        );
        assert!(blob.sample_object(19.5, 10.0, (image.width, image.height), 1).is_none(), "a transparent source pixel has no color to pick");
        click_control(&mut app, &ctx, SAMPLE_TIP);
        let binding = app.session.tool_options()["imageTraceIgnoreColor"].clone();
        assert!(!binding.is_null());
        for miss in [outside, transparent] {
            press_armed_canvas(&mut app, &ctx, miss);
            assert_eq!(app.session.tool_id(), "eyedropper", "live={live_trace}: a miss stays armed");
            assert!(app.ui.trace_ignore_sampler.is_some());
            assert_eq!(app.session.tool_options()["imageTraceIgnoreColor"], binding);
            assert_eq!(app.session.tool_options()["stop"], "selection");
            assert_eq!(app.session.paint, paint);
            let state = app.session.active().unwrap();
            assert_eq!(state.doc, document);
            assert_eq!(state.selection, selection);
            assert_eq!(state.revision, revision);
            assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        }
        press_armed_canvas(&mut app, &ctx, hit);
        assert_eq!(app.session.tool_id(), "selection", "the valid pick restores the previous tool");
        assert!(app.ui.trace_ignore_sampler.is_none());
        assert!(app.session.tool_options_of("eyedropper")["imageTraceIgnoreColor"].is_null());
        assert_eq!(app.session.paint, paint);
        assert_eq!(app.session.active().unwrap().doc, document, "Preview off keeps the accepted color in the draft");
        click_control(&mut app, &ctx, "Trace");
        assert_eq!(settings(&app)["params"]["ignoreColor"], "#255395", "the next valid source pixel completes the same armed pick");
    }
}

#[test]
fn stale_target_click_is_consumed_before_the_restored_tool_can_select_or_paint() {
    for invalidation in ["selection", "document", "revision_round_trip"] {
        let (mut app, ctx) = fixture(true, [37, 83, 149], false);
        let point = source_point(&app, 0.25);
        let old_document = app.session.active().unwrap().doc.clone();
        let old_uid = app.session.active().unwrap().uid;
        let armed_revision = app.session.active().unwrap().revision;
        click_control(&mut app, &ctx, SAMPLE_TIP);
        match invalidation {
            "selection" => {
                app.run("select.none", json!({})).unwrap();
            }
            "document" => {
                app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            }
            _ => {
                app.run("imageTrace.setView", json!({"view": TraceView::Outlines.id()})).unwrap();
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc, old_document);
                assert_ne!(app.session.active().unwrap().revision, armed_revision);
            }
        }
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        // No UI refresh intervenes: this must exercise the real dispatch stale-Down guard.
        let view = app.view_info();
        canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, point.x, point.y), view);
        assert_eq!(app.session.tool_id(), "selection", "{invalidation}: restore prior tool");
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document);
        assert_eq!(state.selection, selection, "the stale click cannot reselect the source");
        assert_eq!(state.revision, revision);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(app.session.paint, paint);
        assert_eq!(app.session.documents().iter().find(|d| d.uid == old_uid).unwrap().doc, old_document);
        settle(&mut app, &ctx);
        assert!(app.canvas.key.as_ref().unwrap().trace_source.is_none());
    }
}

#[test]
fn a_stale_sampler_and_canvas_press_in_one_frame_do_not_reselect_the_source() {
    let (mut app, ctx) = fixture(true, [37, 83, 149], false);
    let point = source_point(&app, 0.25);
    let target = selected(&app).id;
    click_control(&mut app, &ctx, SAMPLE_TIP);
    app.run("select.none", json!({})).unwrap();
    // The last real layout supplies the canvas rect and view. Canvas itself records this
    // context's zoom_factor as its UI-coordinate scale before routing input.
    let at = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(point);
    assert!(app.canvas_rect.unwrap().contains(at));
    let state = app.session.active().unwrap();
    let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
    let history = (state.history.undo.len(), state.history.redo.len());
    let paint = app.session.paint.clone();
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]).textures_delta.clear();
    assert_eq!(app.session.tool_id(), "selection");
    let state = app.session.active().unwrap();
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection, "the press in the cancellation frame is consumed");
    assert_eq!(state.revision, revision);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    assert_eq!(app.session.paint, paint);
    frame(&mut app, &ctx, vec![press(false)]).textures_delta.clear();
    assert!(app.session.active().unwrap().selection.objects.is_empty());
    // Positive control: the same ordinary, unarmed click really hits and selects this trace.
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]).textures_delta.clear();
    frame(&mut app, &ctx, vec![press(false)]).textures_delta.clear();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![target]);
}

/// A stale sampler consumes its second press, including the release's double-click action.
#[test]
fn a_consumed_stale_sampler_press_does_not_complete_a_double_click() {
    // Run the legitimate control first: this geometry and these real UI frames must isolate.
    check_sampler_double_click(false);
    check_sampler_double_click(true);
}

fn check_sampler_double_click(sampler: bool) {
    fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) {
        let mut raw = RawInput {
            time: Some(time),
            events,
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1400.0))),
            ..Default::default()
        };
        app.raw_input_hook(ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            app.tick(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        assert_eq!(ctx.time(), time, "the real UI frame uses the supplied gesture time");
    }

    let (mut app, ctx) = fixture(true, [37, 83, 149], false);
    let source = selected(&app).id;
    app.session.prefs.double_click_to_isolate = true;
    let first = app.run("shape.rectangle", json!({"x": 100, "y": 140, "width": 20, "height": 20})).unwrap()["id"].clone();
    let second = app.run("shape.rectangle", json!({"x": 120, "y": 140, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [first.clone(), second.clone()], "color": "#cc3322"})).unwrap();
    app.run("select.set", json!({"ids": [first, second]})).unwrap();
    let group = NodeId(app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [source.0]})).unwrap();
    settle(&mut app, &ctx);
    let point = Point::new(110.0, 150.0);
    assert!(point.x < source_point(&app, 0.0).x - 40.0, "the group is outside the source image");
    if sampler {
        click_control(&mut app, &ctx, SAMPLE_TIP);
        assert_eq!(app.session.tool_id(), "eyedropper");
        assert!(app.ui.trace_ignore_sampler.is_some());
    }
    let at = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(point);
    assert!(app.canvas_rect.unwrap().contains(at));
    let (delay, hold) = ctx.options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_duration));
    assert!(delay.is_finite() && delay > 0.0 && hold.is_finite() && hold > 0.0);
    let step = delay.min(hold) / 8.0;
    // Leave the panel-button click outside the interval; only these two canvas clicks count.
    let start = ctx.time() + delay + 1.0;
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_at(&mut app, &ctx, start, vec![Event::PointerMoved(at), press(true)]);
    frame_at(&mut app, &ctx, start + step, vec![press(false)]);
    assert!(!ctx.input(|i| i.pointer.button_double_clicked(PointerButton::Primary)), "the first canvas click is single");
    assert_eq!(app.session.active().unwrap().isolation, None);

    if sampler {
        assert_eq!(app.session.tool_id(), "eyedropper", "the first click is a source-image miss");
        assert!(app.ui.trace_ignore_sampler.is_some(), "the miss keeps sampling armed");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![source], "the miss does not select the group");
        // No intervening UI frame: canvas refresh must consume the stale second press.
        app.run("select.none", json!({})).unwrap();
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        frame_at(&mut app, &ctx, start + 3.0 * step, vec![press(true)]);
        assert_eq!(app.session.tool_id(), "selection");
        assert!(app.ui.trace_ignore_sampler.is_none());
        assert_eq!(app.session.active().unwrap().selection, selection, "the second press was consumed");
        frame_at(&mut app, &ctx, start + 4.0 * step, vec![press(false)]);
        assert!(ctx.input(|i| i.pointer.button_double_clicked(PointerButton::Primary)), "egui recognizes the timed release as a double-click");
        let state = app.session.active().unwrap();
        assert_eq!(state.isolation, None, "the release of the consumed press must not isolate the group");
        assert_eq!(state.doc, document);
        assert_eq!(state.selection, selection);
        assert_eq!(state.revision, revision);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(app.session.paint, paint);
        assert_eq!(app.session.tool_id(), "selection");
        // The consumed release must not take the next independent ordinary gesture with it.
        let next = start + 4.0 * step + delay + 1.0;
        frame_at(&mut app, &ctx, next, vec![Event::PointerMoved(at), press(true)]);
        frame_at(&mut app, &ctx, next + step, vec![press(false)]);
        assert!(!ctx.input(|i| i.pointer.button_double_clicked(PointerButton::Primary)));
        assert_eq!(app.session.active().unwrap().selection.objects, vec![group], "the next ordinary first click selects the group");
        assert_eq!(app.session.active().unwrap().isolation, None);
        frame_at(&mut app, &ctx, next + 3.0 * step, vec![press(true)]);
        frame_at(&mut app, &ctx, next + 4.0 * step, vec![press(false)]);
        assert!(ctx.input(|i| i.pointer.button_double_clicked(PointerButton::Primary)));
        assert_eq!(app.session.active().unwrap().isolation, Some(group), "the next ordinary double-click still isolates the group");
    } else {
        assert_eq!(app.session.active().unwrap().selection.objects, vec![group], "the ordinary first click really selects the group");
        frame_at(&mut app, &ctx, start + 3.0 * step, vec![press(true)]);
        frame_at(&mut app, &ctx, start + 4.0 * step, vec![press(false)]);
        assert!(ctx.input(|i| i.pointer.button_double_clicked(PointerButton::Primary)));
        assert_eq!(app.session.active().unwrap().isolation, Some(group), "the legitimate unarmed double-click isolates the same group");
    }
}

fn capture_canvas(app: &mut VectorcraftApp, ctx: &egui::Context) -> (vectorcraft_render::Rendered, CacheKey) {
    app.canvas.key = None; // force a complete upload, not a partial delta requiring reconstruction
    let mut out = frame(app, ctx, vec![]);
    let id = app.canvas.texture.as_ref().expect("real canvas texture").id();
    let upload =
        out.textures_delta.set.iter().find(|(texture, _)| **texture == id).and_then(|(_, updates)| updates.last()).expect("CPU canvas upload");
    assert!(upload.pos.is_none());
    let egui::ImageData::Color(image) = &upload.image;
    let rendered = vectorcraft_render::Rendered {
        width: image.width() as u32,
        height: image.height() as u32,
        pixels: image.pixels.iter().flat_map(|c| c.to_array()).collect(),
    };
    let key = app.canvas.key.as_ref().unwrap().clone();
    out.textures_delta.clear();
    (rendered, key)
}

fn pixel_at(image: &vectorcraft_render::Rendered, key: &CacheKey, point: Point) -> [u8; 4] {
    assert!(key.pixel.is_none() && key.rot == 0.0, "fixture uses the ordinary unrotated canvas");
    let x = (f64::from(image.width) * 0.5 + (point.x - key.cx) * key.zoom * f64::from(key.ppp)).floor();
    let y = (f64::from(image.height) * 0.5 + (point.y - key.cy) * key.zoom * f64::from(key.ppp)).floor();
    assert!(x >= 0.0 && x < f64::from(image.width) && y >= 0.0 && y < f64::from(image.height));
    let at = ((y as usize * image.width as usize) + x as usize) * 4;
    image.pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn actual_canvas_keeps_its_displayed_view_while_the_original_pixels_are_sampled() {
    let (mut app, ctx) = fixture(true, [37, 83, 149], false);
    let point = source_point(&app, 0.25);
    let control = source_point(&app, 0.75);
    let target: NodeId = selected(&app).id;
    let state = app.session.active().unwrap();
    let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
    let history = (state.history.undo.len(), state.history.redo.len());
    let uid = state.uid;
    let original = state.doc.node(target).unwrap().children().unwrap()[0].clone();
    assert!(!original.visible, "the live trace retains its hidden original");
    let NodeKind::Image(image) = &original.kind else { panic!("original image") };
    let blob = state.doc.images.get(&image.key).unwrap();
    assert_eq!(
        blob.sample_object(f64::from(image.width) * 0.25, f64::from(image.height) * 0.5, (image.width, image.height), 1),
        Some([37, 83, 149, 255])
    );
    let stored = settings(&app);
    let (before, before_key) = capture_canvas(&mut app, &ctx);
    assert_eq!(pixel_at(&before, &before_key, point), [0, 0, 0, 255]);
    assert_eq!(pixel_at(&before, &before_key, control), [255; 4]);
    click_control(&mut app, &ctx, SAMPLE_TIP);
    let (armed, armed_key) = capture_canvas(&mut app, &ctx);
    assert!(!ctx.input(|i| i.pointer.any_down()), "the button press has ended");
    assert_eq!(armed_key.trace_source, before_key.trace_source, "arming does not override the existing eye hold or trace View");
    assert_eq!(armed.pixels, before.pixels, "sampling original pixels does not change the displayed tracing result");
    assert_eq!(pixel_at(&armed, &armed_key, point), [0, 0, 0, 255]);
    assert_eq!(pixel_at(&armed, &armed_key, control), [255; 4]);
    press_armed_canvas(&mut app, &ctx, point);
    assert_eq!(app.session.tool_id(), "selection");
    let (restored, restored_key) = capture_canvas(&mut app, &ctx);
    assert!(restored_key.trace_source.is_none());
    assert_eq!(restored.pixels, before.pixels, "the valid Preview-off pick leaves the displayed result unchanged");
    assert_eq!(settings(&app), stored, "the stored View and trace settings are unchanged");
    let state = app.session.active().unwrap();
    assert_eq!((state.uid, state.revision), (uid, revision));
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    click_control(&mut app, &ctx, "Trace");
    assert_eq!(
        settings(&app)["params"]["ignoreColor"],
        "#255395",
        "the source color is sampled while the canvas still shows its black tracing result"
    );
}

/// Cancelling while Cmd/Ctrl borrows Selection also clears the parked eyedropper's one-shot.
#[test]
fn cancelling_a_borrowed_sampler_clears_the_restored_eyedropper() {
    fn loan_frame(app: &mut VectorcraftApp, ctx: &egui::Context, mut events: Vec<Event>) -> egui::FullOutput {
        events.insert(0, Event::ModifiersChanged(egui::Modifiers { ctrl: true, command: true, ..Default::default() }));
        frame(app, ctx, events)
    }

    fn borrow_and_release(app: &mut VectorcraftApp, ctx: &egui::Context, point: Point, stale: bool) {
        let modifiers = vectorcraft_tools::Mods { cmd: true, ..Default::default() };
        assert_eq!(app.session.lent_tool_id(&modifiers), Some("selection"), "exercise the real engine loan");
        let at = canvas::Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), ctx.zoom_factor()).to_screen(point);
        assert!(app.canvas_rect.unwrap().contains(at));
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() },
        };
        loan_frame(app, ctx, vec![Event::PointerMoved(at), press(true)]).textures_delta.clear();
        assert_eq!(app.session.tool_id(), "selection", "the eyedropper is now parked in Session::lender");
        if stale {
            app.run("select.none", json!({})).unwrap();
        }
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        loan_frame(app, ctx, vec![]).textures_delta.clear();
        assert_eq!(app.session.tool_id(), "selection", "cancellation respects the borrowed gesture");
        assert!(app.ui.trace_ignore_sampler.is_none(), "the real frame ends the UI-owned sampler");
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document);
        assert_eq!(state.selection, selection);
        assert_eq!(state.revision, revision);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(app.session.paint, paint, "cancelling a parked sampler does not paint");
        loan_frame(app, ctx, vec![press(false)]).textures_delta.clear();
        frame(app, ctx, vec![Event::ModifiersChanged(egui::Modifiers::NONE)]).textures_delta.clear();
        assert_eq!(app.session.tool_id(), "eyedropper", "ordinary engine loan restoration is retained");
    }

    fn ordinary_sample(app: &mut VectorcraftApp, ctx: &egui::Context, point: Point) -> (usize, usize, u64) {
        app.run("select.none", json!({})).unwrap();
        app.run("paint.setFill", json!({"color": "#e020c0"})).unwrap();
        assert!(app.session.fill_active);
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let old_paint = app.session.paint.clone();
        let view = app.view_info();
        let down = PointerEvent::new(PointerKind::Down, point.x, point.y).with_mods(vectorcraft_tools::Mods { shift: true, ..Default::default() });
        canvas::dispatch(app, &down, view);
        canvas::dispatch(app, &PointerEvent::new(PointerKind::Up, point.x, point.y), view);
        frame(app, ctx, vec![]).textures_delta.clear();
        assert_eq!(app.session.tool_id(), "eyedropper", "a normal sample does not consume a stale one-shot or switch tools");
        assert_ne!(app.session.paint, old_paint, "the normal color sample actually runs");
        assert_eq!(app.session.paint.fill, vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK));
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document, "with no selection a normal sample changes only paint defaults");
        assert_eq!(state.selection, selection);
        (state.history.undo.len() - history.0, state.history.redo.len().abs_diff(history.1), state.revision - revision)
    }

    for stale in [false, true] {
        // The same ordinary loan is the control for legitimate selection/default/history work.
        let (mut control, control_ctx) = fixture(true, [37, 83, 149], false);
        let control_point = source_point(&control, 0.25);
        let control_target = selected(&control).id;
        let control_revision = control.session.active().unwrap().revision;
        let control_document = control.session.active().unwrap().doc.clone();
        let control_history = {
            let state = control.session.active().unwrap();
            (state.history.undo.len(), state.history.redo.len())
        };
        control.select_tool("eyedropper");
        borrow_and_release(&mut control, &control_ctx, control_point, stale);
        let expected_selection: Vec<bool> = control.session.active().unwrap().selection.objects.iter().map(|id| *id == control_target).collect();
        let expected_revision_delta = control.session.active().unwrap().revision - control_revision;
        assert_eq!(control.session.active().unwrap().doc, control_document);
        assert_eq!((control.session.active().unwrap().history.undo.len(), control.session.active().unwrap().history.redo.len()), control_history);
        let expected_paint = control.session.paint.clone();
        let expected_sample_effects = ordinary_sample(&mut control, &control_ctx, control_point);

        let (mut app, ctx) = fixture(true, [37, 83, 149], false);
        let point = source_point(&app, 0.25);
        let target = selected(&app).id;
        let state = app.session.active().unwrap();
        let (document, revision) = (state.doc.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        click_control(&mut app, &ctx, SAMPLE_TIP);
        assert_eq!(app.session.tool_id(), "eyedropper");
        assert!(!app.session.tool_options()["imageTraceIgnoreColor"].is_null());
        borrow_and_release(&mut app, &ctx, point, stale);
        assert!(app.session.tool_options()["imageTraceIgnoreColor"].is_null(), "stale={stale}: the returned lender must not stay armed");
        assert!(app.session.tool_options()["stop"].is_null(), "stale={stale}: no ghost return-tool action");
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, document);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(state.selection.objects.iter().map(|id| *id == target).collect::<Vec<_>>(), expected_selection);
        assert_eq!(state.revision - revision, expected_revision_delta);
        assert_eq!(app.session.paint, expected_paint, "only the ordinary borrowed gesture's default changes are permitted");
        assert_eq!(ordinary_sample(&mut app, &ctx, point), expected_sample_effects, "the next real click follows the ordinary eyedropper path");
    }
}
