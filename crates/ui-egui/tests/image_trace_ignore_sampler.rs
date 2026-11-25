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
    let raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1400.0))), ..Default::default() };
    // `before_input` already begins the host iteration; do not begin a second one.
    craft::ui::shell::embed::before_input(ctx, |out| app.frame_input(out));
    run_ui(app, ctx, raw)
}

thread_local! {
    /// The input of the first pass of each frame the last step ran, and the pass the step ended on.
    static FRAME_INPUT: std::cell::RefCell<Option<(Vec<egui::InputState>, u64)>> = const { std::cell::RefCell::new(None) };
}

/// The pointer moves and button events among `events`.
fn pointer_events(events: &[Event]) -> usize {
    events.iter().filter(|event| matches!(event, Event::PointerMoved(_) | Event::PointerButton { .. })).count()
}

/// One step of the app: the frame of `raw`, then empty frames at its time until all its pointer events arrived. A frame
/// can take more than one pass, and egui gives the frame's events to its first; the framework gives a pass one press,
/// as its last pointer event, and holds the events after it for the next frame ("Input pacing", one press per pass).
fn run_ui(app: &mut VectorcraftApp, ctx: &egui::Context, mut raw: RawInput) -> egui::FullOutput {
    let (sent, time, screen_rect) = (pointer_events(&raw.events), raw.time, raw.screen_rect);
    let mut frames = Vec::new();
    let mut arrived = 0;
    loop {
        let mut first = None;
        let out = ctx.run_ui(raw, |ui| {
            if first.is_none() {
                first = Some(ui.ctx().input(Clone::clone));
            }
            app.tick(ui.ctx());
            app.ui(ui);
        });
        let first = first.expect("a frame runs a pass");
        arrived += pointer_events(&first.events);
        frames.push(first);
        if arrived >= sent {
            FRAME_INPUT.with(|input| *input.borrow_mut() = Some((frames, ctx.cumulative_pass_nr())));
            return out;
        }
        assert!(frames.len() < 8, "the framework holds this step's pointer events");
        let mut out = out;
        out.textures_delta.clear();
        craft::ui::shell::embed::before_input(ctx, |out| app.frame_input(out));
        raw = RawInput { time, focused: true, screen_rect, ..Default::default() };
    }
}

/// The last step's frames' input, checked to be read right after the step.
fn with_frames<R>(ctx: &egui::Context, read: impl FnOnce(&[egui::InputState]) -> R) -> R {
    FRAME_INPUT.with(|input| {
        let input = input.borrow();
        let (frames, pass) = input.as_ref().expect("a frame ran");
        assert_eq!(*pass, ctx.cumulative_pass_nr(), "read right after the frame");
        read(frames)
    })
}

/// Reads egui's input as the last step's events left it: the button held, the pointer's place.
fn frame_input<R>(ctx: &egui::Context, read: impl FnOnce(&egui::InputState) -> R) -> R {
    with_frames(ctx, |frames| read(frames.last().expect("a frame ran")))
}

/// Whether egui saw `event` (a press, release or click) in a frame of the last step.
fn frame_event(ctx: &egui::Context, event: impl Fn(&egui::InputState) -> bool) -> bool {
    with_frames(ctx, |frames| frames.iter().any(event))
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
    craft::ui::shell::embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
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
    let view = app.canvas_view().unwrap();
    let at = view.to_screen(point);
    assert!(view.rect.contains(at));
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
    let view = app.canvas_view().unwrap();
    let at = view.to_screen(point);
    assert!(view.rect.contains(at));
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

fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) {
    let raw = RawInput {
        time: Some(time),
        events,
        focused: true,
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1400.0))),
        ..Default::default()
    };
    craft::ui::shell::embed::before_input(ctx, |out| app.frame_input(out));
    let mut out = run_ui(app, ctx, raw);
    out.textures_delta.clear();
    assert_eq!(ctx.time(), time, "the real UI frame uses the supplied gesture time");
}

fn check_next_ordinary_double_click(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, group: NodeId, next: f64, step: f64) {
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_at(app, ctx, next, vec![Event::PointerMoved(at), press(true)]);
    frame_at(app, ctx, next + step, vec![press(false)]);
    assert!(!frame_event(ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().selection.objects, vec![group], "the next ordinary first click selects the group");
    assert_eq!(app.session.active().unwrap().isolation, None);
    frame_at(app, ctx, next + 3.0 * step, vec![press(true)]);
    frame_at(app, ctx, next + 4.0 * step, vec![press(false)]);
    assert!(frame_event(ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().isolation, Some(group), "the next ordinary double-click still isolates the group");
}

fn check_sampler_double_click(sampler: bool) {
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
    let view = app.canvas_view().unwrap();
    let at = view.to_screen(point);
    assert!(view.rect.contains(at));
    let (delay, hold) = ctx.options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_duration));
    assert!(delay.is_finite() && delay > 0.0 && hold.is_finite() && hold > 0.0);
    let step = delay.min(hold) / 8.0;
    // Leave the panel-button click outside the interval; only these two canvas clicks count.
    let start = ctx.time() + delay + 1.0;
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_at(&mut app, &ctx, start, vec![Event::PointerMoved(at), press(true)]);
    frame_at(&mut app, &ctx, start + step, vec![press(false)]);
    assert!(!frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)), "the first canvas click is single");
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
        assert!(
            frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)),
            "egui recognizes the timed release as a double-click"
        );
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
        check_next_ordinary_double_click(&mut app, &ctx, at, group, next, step);
    } else {
        assert_eq!(app.session.active().unwrap().selection.objects, vec![group], "the ordinary first click really selects the group");
        frame_at(&mut app, &ctx, start + 3.0 * step, vec![press(true)]);
        frame_at(&mut app, &ctx, start + 4.0 * step, vec![press(false)]);
        assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
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
    assert!(!frame_input(&ctx, |i| i.pointer.any_down()), "the button press has ended");
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
        let view = app.canvas_view().unwrap();
        let at = view.to_screen(point);
        assert!(view.rect.contains(at));
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

fn review_gesture_group_fixture() -> (VectorcraftApp, egui::Context, Point, NodeId) {
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
    assert!(point.x < source_point(&app, 0.0).x - 40.0, "the gesture group is outside the sampled image");
    (app, ctx, point, group)
}

fn review_gesture_screen(app: &VectorcraftApp, point: Point) -> Pos2 {
    let view = app.canvas_view().unwrap();
    let at = view.to_screen(point);
    assert!(view.rect.contains(at), "the real canvas contains the gesture point");
    at
}

fn review_gesture_times(ctx: &egui::Context) -> (f64, f64) {
    let (delay, hold) = ctx.options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_duration));
    assert!(delay.is_finite() && delay > 0.0 && hold.is_finite() && hold > 0.0);
    (ctx.time() + delay + 1.0, delay.min(hold) / 8.0)
}

fn review_gesture_press(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, time: f64) {
    let event = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() };
    frame_at(app, ctx, time, vec![Event::PointerMoved(at), event]);
}

fn review_gesture_release(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, time: f64) {
    let event = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame_at(app, ctx, time, vec![event]);
}

fn review_gesture_state(app: &VectorcraftApp) -> impl PartialEq + std::fmt::Debug + use<> {
    let state = app.session.active().unwrap();
    (
        state.doc.clone(),
        state.selection.clone(),
        state.revision,
        (state.history.undo.len(), state.history.redo.len()),
        app.session.paint.clone(),
        state.isolation,
    )
}

fn review_gesture_observation(app: &VectorcraftApp, ctx: &egui::Context) -> (bool, bool, Vec<NodeId>, Option<NodeId>) {
    let state = app.session.active().unwrap();
    (
        frame_event(ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)),
        frame_event(ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)),
        state.selection.objects.clone(),
        state.isolation,
    )
}

fn review_gesture_ordinary_pair(app: &mut VectorcraftApp, ctx: &egui::Context, first: Pos2, second: Pos2, group: NodeId) {
    let (start, step) = review_gesture_times(ctx);
    review_gesture_press(app, ctx, first, start);
    review_gesture_release(app, ctx, first, start + step);
    assert!(!frame_event(ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().selection.objects, vec![group], "the ordinary first click selects the group");
    assert_eq!(app.session.active().unwrap().isolation, None);
    review_gesture_press(app, ctx, second, start + 3.0 * step);
    frame_at(app, ctx, start + 3.5 * step, vec![]);
    review_gesture_release(app, ctx, second, start + 4.0 * step);
    assert!(frame_event(ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().isolation, Some(group), "the ordinary held double-click isolates the group");
}

#[test]
fn review_followup_sampler_cancelled_mid_hold_keeps_release_owned() {
    let (mut control, control_ctx, control_point, control_group) = review_gesture_group_fixture();
    let control_at = review_gesture_screen(&control, control_point);
    review_gesture_ordinary_pair(&mut control, &control_ctx, control_at, control_at, control_group);

    let (mut app, ctx, point, _) = review_gesture_group_fixture();
    let source = selected(&app).id;
    click_control(&mut app, &ctx, SAMPLE_TIP);
    let at = review_gesture_screen(&app, point);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    review_gesture_release(&mut app, &ctx, at, start + step);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert!(app.ui.trace_ignore_sampler.is_some(), "the first canvas click is a sampler miss");
    assert_eq!(app.session.active().unwrap().selection.objects, vec![source]);
    assert_eq!(app.session.active().unwrap().isolation, None);
    review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    assert!(app.ui.trace_ignore_sampler.is_some(), "the second Down still belongs to the valid sampler");
    app.run("select.none", json!({})).unwrap();
    let expected = review_gesture_state(&app);
    frame_at(&mut app, &ctx, start + 3.5 * step, vec![]);
    assert!(frame_input(&ctx, |i| i.pointer.primary_down()));
    assert!(!frame_event(&ctx, |i| i.pointer.primary_pressed()));
    assert_eq!(app.session.tool_id(), "selection");
    assert!(app.ui.trace_ignore_sampler.is_none(), "the held frame cancelled the invalid binding");
    review_gesture_release(&mut app, &ctx, at, start + 4.0 * step);
    assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(review_gesture_state(&app), expected, "a mid-hold cancellation must retain ownership through release");
    assert_eq!(app.session.tool_id(), "selection");
}

#[test]
fn review_followup_consumed_prefix_keeps_two_fresh_clicks_single_then_double() {
    let (mut control, control_ctx, control_point, control_group) = review_gesture_group_fixture();
    let control_at = review_gesture_screen(&control, control_point);
    review_gesture_ordinary_pair(&mut control, &control_ctx, control_at, control_at, control_group);

    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    app.run("select.none", json!({})).unwrap();
    let at = review_gesture_screen(&app, point);
    let expected = review_gesture_state(&app);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    review_gesture_release(&mut app, &ctx, at, start + step);
    assert!(!frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(review_gesture_state(&app), expected, "the single stale prefix itself is consumed");
    assert_eq!(app.session.tool_id(), "selection");

    review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    review_gesture_release(&mut app, &ctx, at, start + 4.0 * step);
    let first = review_gesture_observation(&app, &ctx);
    // Always run the second fresh click, even when the first app-level oracle was wrong.
    review_gesture_press(&mut app, &ctx, at, start + 6.0 * step);
    review_gesture_release(&mut app, &ctx, at, start + 7.0 * step);
    let second = review_gesture_observation(&app, &ctx);
    assert!(first.0 && !first.1 && !second.0 && second.1, "expected raw Double then Triple: first={first:?}, second={second:?}");
    assert!(
        first.2 == vec![group] && first.3.is_none() && second.3 == Some(group),
        "fresh clicks must select without isolation, then isolate: first={first:?}, second={second:?}"
    );
}

fn review_gesture_enclose_source(app: &mut VectorcraftApp, source: NodeId) -> NodeId {
    let background = app.run("shape.rectangle", json!({"x": 50, "y": 30, "width": 300, "height": 240})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [background.clone()], "color": "#e020c0"})).unwrap();
    app.run("select.set", json!({"ids": [source.0, background]})).unwrap();
    let group = NodeId(app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [source.0]})).unwrap();
    group
}

fn review_gesture_original_pixels(app: &VectorcraftApp) {
    let trace = selected(app);
    let original = trace.children().unwrap()[0].as_ref();
    let NodeKind::Image(image) = &original.kind else { panic!("the fixture retains its original image") };
    let state = app.session.active().unwrap();
    let blob = &state.doc.images[&image.key];
    assert!(blob.sample_object(19.5, 10.0, (image.width, image.height), 1).is_none(), "the first point is really transparent");
    assert_eq!(
        blob.sample_object(5.0, 10.0, (image.width, image.height), 1),
        Some([37, 83, 149, 255]),
        "the second point has a distinct nonwhite source color"
    );
}

fn review_gesture_close_pair(app: &VectorcraftApp, ctx: &egui::Context, miss: Point, hit: Point) -> (Pos2, Pos2) {
    let miss = review_gesture_screen(app, miss);
    let hit = review_gesture_screen(app, hit);
    let distance = ctx.options(|o| o.input_options.max_click_dist);
    assert!(distance.is_finite() && distance > 0.0);
    assert!(miss.distance_sq(hit) < distance * distance, "the actual transformed miss/hit pair satisfies egui's click distance");
    (miss, hit)
}

fn review_gesture_completion_fixture() -> (VectorcraftApp, egui::Context, Point, Point, NodeId) {
    let (mut app, ctx) = fixture_with_transparent_edge(true, [37, 83, 149], false, true);
    let source = selected(&app).id;
    review_gesture_original_pixels(&app);
    app.session.prefs.double_click_to_isolate = true;
    let group = review_gesture_enclose_source(&mut app, source);
    // view.setZoom takes display percent; use its public 10% setting, then assert actual geometry.
    app.run("view.setZoom", json!({"zoom": 10.0, "center": [200, 150]})).unwrap();
    settle(&mut app, &ctx);
    let miss = source_point(&app, 0.975);
    let hit = source_point(&app, 0.25);
    (app, ctx, miss, hit, group)
}

#[test]
fn review_followup_valid_sampler_down_completes_without_lending_its_release() {
    let (mut control, control_ctx, control_miss, control_hit, control_group) = review_gesture_completion_fixture();
    control.run("select.none", json!({})).unwrap();
    settle(&mut control, &control_ctx);
    let (control_miss, control_hit) = review_gesture_close_pair(&control, &control_ctx, control_miss, control_hit);
    review_gesture_ordinary_pair(&mut control, &control_ctx, control_miss, control_hit, control_group);

    let (mut app, ctx, miss, hit, _) = review_gesture_completion_fixture();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    let (miss, hit) = review_gesture_close_pair(&app, &ctx, miss, hit);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, miss, start);
    review_gesture_release(&mut app, &ctx, miss, start + step);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert!(app.ui.trace_ignore_sampler.is_some(), "the transparent first click keeps the sampler armed");
    let expected = review_gesture_state(&app);
    review_gesture_press(&mut app, &ctx, hit, start + 3.0 * step);
    assert_eq!(app.session.tool_id(), "selection", "the opaque Down completes sampling and restores the tool");
    assert!(app.ui.trace_ignore_sampler.is_none());
    assert_eq!(review_gesture_state(&app), expected, "Preview off sampling changes only the panel draft");
    review_gesture_release(&mut app, &ctx, hit, start + 4.0 * step);
    assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(review_gesture_state(&app), expected, "a completed sampler still owns its matching release");
    assert_eq!(app.session.tool_id(), "selection");
    // A fix that merely cancels the valid Down must not pass without accepting its actual color.
    click_control(&mut app, &ctx, "Trace");
    assert_eq!(settings(&app)["params"]["ignoreColor"], "#255395", "Trace persists the real opaque source sample");
    assert_eq!(settings(&app)["params"]["ignoreWhite"], true);
}

fn review_preset_checkbox(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> bool {
    for _ in 0..3 {
        let mut output = frame(app, ctx, vec![]);
        let checked = output.platform_output.accesskit_update.as_ref().and_then(|update| {
            update.nodes.iter().find(|(_, node)| node.label() == Some(label) && node.role() == egui::accesskit::Role::CheckBox).and_then(
                |(_, node)| match node.toggled() {
                    Some(egui::accesskit::Toggled::True) => Some(true),
                    Some(egui::accesskit::Toggled::False) => Some(false),
                    _ => None,
                },
            )
        });
        output.textures_delta.clear();
        if let Some(checked) = checked {
            return checked;
        }
    }
    panic!("fixture: missing accessible Image Trace checkbox state: {label}");
}

fn review_preset_assert_shown(app: &mut VectorcraftApp, ctx: &egui::Context, preset: &str) {
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(drawn) => out.push(drawn.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
            _ => {}
        }
    }
    let mut output = frame(app, ctx, vec![]);
    let mut shown = Vec::new();
    output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut shown));
    output.textures_delta.clear();
    let word = preset.split(' ').next().unwrap_or(preset);
    let near: Vec<&String> = shown.iter().filter(|text| text.contains(word)).collect();
    // The framework's preset row marks a built-in preset after its name (ui-patterns `namedPresetRow.builtIn`).
    let built_in = format!("{preset} (built-in)");
    assert!(shown.iter().any(|text| *text == preset || *text == built_in), "fixture: the real Preset control must show {preset}; shown: {near:?}");
}

fn review_preset_click_opaque_source(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let point = source_point(app, 0.25);
    let source = selected(app);
    let original = source.children().and_then(|children| children.first()).unwrap();
    let NodeKind::Image(image) = &original.kind else { panic!("fixture: the live trace retains its original image") };
    assert_eq!(
        app.session.active().unwrap().doc.images[&image.key].sample_object(
            f64::from(image.width) * 0.25,
            f64::from(image.height) * 0.5,
            (image.width, image.height),
            1,
        ),
        Some([37, 83, 149, 255]),
        "fixture: the actual canvas point is an opaque original pixel"
    );
    let view = app.canvas_view().unwrap();
    let at = view.to_screen(point);
    assert!(view.rect.contains(at), "fixture: the source point lies on the actual canvas");
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(at), press(true)]).textures_delta.clear();
    frame(app, ctx, vec![press(false)]).textures_delta.clear();
    settle(app, ctx);
}

fn review_preset_enhanced_on_control() {
    // Legitimate control first: the real Enhanced pair retains Ignore Color and a valid pick.
    let (mut control, control_ctx) = fixture(true, [37, 83, 149], false);
    click_control(&mut control, &control_ctx, "Black and White");
    review_preset_assert_shown(&mut control, &control_ctx, "Black and White Logo");
    assert!(review_preset_checkbox(&mut control, &control_ctx, "Ignore Color"));
    let enhanced = review_preset_checkbox(&mut control, &control_ctx, "Enhanced");
    click_control(&mut control, &control_ctx, SAMPLE_TIP);
    assert_eq!(control.session.tool_id(), "eyedropper");
    assert!(control.ui.trace_ignore_sampler.is_some());
    click_control(&mut control, &control_ctx, "Enhanced");
    assert_ne!(review_preset_checkbox(&mut control, &control_ctx, "Enhanced"), enhanced, "fixture: the real Enhanced checkbox changed");
    assert!(review_preset_checkbox(&mut control, &control_ctx, "Ignore Color"), "Enhanced keeps this preset's Ignore Color on");
    assert!(control.ui.trace_ignore_sampler.is_some(), "an on-to-on settings change keeps the pick armed");
    assert_eq!(control.session.tool_id(), "eyedropper");
    assert!(!control.session.tool_options()["imageTraceIgnoreColor"].is_null());
    review_preset_click_opaque_source(&mut control, &control_ctx);
    assert!(control.ui.trace_ignore_sampler.is_none(), "the opaque pick completes normally");
    assert_eq!(control.session.tool_id(), "selection");
    click_control(&mut control, &control_ctx, "Trace");
    assert_eq!(settings(&control)["params"]["ignoreWhite"], true);
    assert_eq!(settings(&control)["params"]["ignoreColor"], "#255395", "the on-to-on control really accepted the source color");
}

#[test]
fn review_followup_preset_off_cancels_sampler_but_enhanced_on_keeps_it() {
    review_preset_enhanced_on_control();

    // Separate causal fixture: Preview off leaves the target/revision valid when a preset turns Ignore Color off.
    let (mut app, ctx) = fixture(true, [37, 83, 149], false);
    let presets = app.run("imageTrace.presets", json!({})).unwrap();
    let expected = presets["presets"].as_array().unwrap().iter().find(|preset| preset["name"] == "High Fidelity Photo").unwrap()["params"].clone();
    assert_eq!(expected["ignoreWhite"], false, "fixture: High Color's public preset disables Ignore Color");
    assert_ne!(expected["ignoreColor"], "#255395");
    click_control(&mut app, &ctx, "Black and White");
    assert!(review_preset_checkbox(&mut app, &ctx, "Ignore Color"));
    click_control(&mut app, &ctx, SAMPLE_TIP);
    assert!(app.ui.trace_ignore_sampler.is_some());
    assert_eq!(app.session.tool_id(), "eyedropper");
    let state = app.session.active().unwrap();
    let (document, revision) = (state.doc.clone(), state.revision);
    let history = (state.history.undo.len(), state.history.redo.len());
    let paint = app.session.paint.clone();
    click_control(&mut app, &ctx, "High Color");
    review_preset_assert_shown(&mut app, &ctx, "High Fidelity Photo");
    assert!(!review_preset_checkbox(&mut app, &ctx, "Ignore Color"), "the actual preset button made Ignore Color off");
    let state = app.session.active().unwrap();
    assert_eq!(state.doc, document, "Preview off changes the panel draft, not the document");
    assert_eq!(state.revision, revision, "the sampler target binding was not invalidated by a trace");
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    assert!(app.ui.trace_ignore_sampler.is_none(), "choosing an Ignore Color-off preset must cancel the armed sampler");
    assert_eq!(app.session.tool_id(), "selection", "cancelling the panel pick restores the previous tool");
    assert!(app.session.tool_options_of("eyedropper")["imageTraceIgnoreColor"].is_null());
    review_preset_click_opaque_source(&mut app, &ctx);
    assert!(app.ui.trace_ignore_sampler.is_none());
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(app.session.paint, paint);
    review_preset_assert_shown(&mut app, &ctx, "High Fidelity Photo");
    assert!(!review_preset_checkbox(&mut app, &ctx, "Ignore Color"));
    click_control(&mut app, &ctx, "Trace");
    assert_eq!(settings(&app)["preset"], "High Fidelity Photo", "the following opaque canvas click cannot turn the off preset into Custom");
    assert_eq!(settings(&app)["params"], expected, "the hidden pick cannot replace the off preset's ignored color");
}

#[test]
fn review_followup_release_position_survives_a_later_hover_move() {
    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    app.run("select.none", json!({})).unwrap();
    let at = review_gesture_screen(&app, point);
    let hover = at + vec2(80.0, 0.0);
    assert!(app.canvas_view().unwrap().rect.contains(hover));
    assert!(at.distance(hover) > ctx.options(|o| o.input_options.max_click_dist));
    let expected = review_gesture_state(&app);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    review_gesture_release(&mut app, &ctx, at, start + step);
    assert_eq!(review_gesture_state(&app), expected, "the stale prefix is consumed");
    review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    frame_at(
        &mut app,
        &ctx,
        start + 4.0 * step,
        vec![
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() },
            Event::PointerMoved(hover),
        ],
    );
    assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(frame_input(&ctx, |i| i.pointer.interact_pos()), Some(hover));
    assert_eq!(app.session.active().unwrap().selection.objects, vec![group]);
    assert_eq!(app.session.active().unwrap().isolation, None, "the first fresh click remains single");
    review_gesture_press(&mut app, &ctx, at, start + 6.0 * step);
    review_gesture_release(&mut app, &ctx, at, start + 7.0 * step);
    assert!(frame_event(&ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().isolation, Some(group), "the fresh pair uses release positions, not subsequent hover positions");
}

fn review_596866_button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn review_596866_owned_release_then_press(coalesced: bool) {
    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    app.run("select.none", json!({})).unwrap();
    let at = review_gesture_screen(&app, point);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(app.ui.trace_ignore_sampler.is_none());
    assert!(frame_input(&ctx, |i| i.pointer.primary_down()));
    let document = app.session.active().unwrap().doc.clone();
    let paint = app.session.paint.clone();
    let history = (app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len());
    if coalesced {
        frame_at(&mut app, &ctx, start + step, vec![review_596866_button(at, false), review_596866_button(at, true)]);
        assert!(frame_input(&ctx, |i| i.pointer.primary_released() && i.pointer.primary_pressed() && i.pointer.primary_down()));
    } else {
        review_gesture_release(&mut app, &ctx, at, start + step);
        assert_eq!(app.session.active().unwrap().selection.objects, Vec::<NodeId>::new(), "CONTROL: the owned Up remains consumed");
        review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    }
    let before_release = review_gesture_observation(&app, &ctx);
    review_gesture_release(&mut app, &ctx, at, start + 4.0 * step);
    let after_release = review_gesture_observation(&app, &ctx);
    assert!(after_release.0 && !after_release.1, "the owned prefix and one fresh click form egui's raw Double");
    assert!(
        before_release.2 == vec![group] && before_release.3.is_none() && after_release.2 == vec![group] && after_release.3.is_none(),
        "coalesced={coalesced}: the first fresh click must select without isolation: before={before_release:?}, after={after_release:?}"
    );
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(app.session.paint, paint);
    assert_eq!((app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len()), history);
    if !coalesced {
        review_gesture_press(&mut app, &ctx, at, start + 6.0 * step);
        review_gesture_release(&mut app, &ctx, at, start + 7.0 * step);
        assert_eq!(app.session.active().unwrap().isolation, Some(group), "CONTROL: the second fresh click still isolates");
    }
}

#[test]
fn review_596866_owned_up_then_new_down_in_one_frame_keeps_the_next_click_single() {
    review_596866_owned_release_then_press(false);
    review_596866_owned_release_then_press(true);
}

#[test]
fn review_596866_second_corrected_release_uses_its_release_position() {
    let (mut control, control_ctx, control_point, control_group) = review_gesture_group_fixture();
    let control_at = review_gesture_screen(&control, control_point);
    review_gesture_ordinary_pair(&mut control, &control_ctx, control_at, control_at, control_group);

    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    app.run("select.none", json!({})).unwrap();
    let at = review_gesture_screen(&app, point);
    let hover = review_gesture_screen(&app, Point::new(340.0, 250.0));
    assert!(at.distance(hover) > ctx.options(|o| o.input_options.max_click_dist));
    let expected = review_gesture_state(&app);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    review_gesture_release(&mut app, &ctx, at, start + step);
    assert_eq!(review_gesture_state(&app), expected, "the stale prefix is consumed");
    review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    review_gesture_release(&mut app, &ctx, at, start + 4.0 * step);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![group]);
    assert_eq!(app.session.active().unwrap().isolation, None, "the first fresh click is single");
    review_gesture_press(&mut app, &ctx, at, start + 6.0 * step);
    frame_at(&mut app, &ctx, start + 7.0 * step, vec![review_596866_button(at, false), Event::PointerMoved(hover)]);
    assert!(frame_event(&ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)));
    assert_eq!(frame_input(&ctx, |i| i.pointer.interact_pos()), Some(hover));
    assert_eq!(
        app.session.active().unwrap().isolation,
        Some(group),
        "the corrected Double belongs to its release on the group, not the later blank hover"
    );
}

fn review_596866_bound_point(app: &VectorcraftApp, target: NodeId) -> Point {
    let state = app.session.active().unwrap();
    let node = state.doc.node(target).unwrap();
    let original = node.children().and_then(|children| children.first()).map_or(node, |child| child.as_ref());
    let NodeKind::Image(image) = &original.kind else { panic!("the retained binding has a source image") };
    image.xf * Point::new(f64::from(image.width) * 0.25, f64::from(image.height) * 0.5)
}

fn review_596866_source_image(app: &VectorcraftApp, target: NodeId) -> NodeId {
    let state = app.session.active().unwrap();
    let node = state.doc.node(target).unwrap();
    let original = node.children().and_then(|children| children.first()).map_or(node, |child| child.as_ref());
    assert!(matches!(original.kind, NodeKind::Image(_)), "the retained binding has an original source image");
    original.id
}

fn review_596866_pick_binding(app: &mut VectorcraftApp, ctx: &egui::Context) -> NodeId {
    click_control(app, ctx, SAMPLE_TIP);
    let target = app.ui.trace_ignore_sampler.as_ref().unwrap().target;
    let source = review_596866_source_image(app, target);
    let point = review_596866_bound_point(app, target);
    let at = review_gesture_screen(app, point);
    let (start, step) = review_gesture_times(ctx);
    review_gesture_press(app, ctx, at, start);
    review_gesture_release(app, ctx, at, start + step);
    settle(app, ctx);
    assert!(app.ui.trace_ignore_sampler.is_none(), "an opaque original pixel completes the pick");
    source
}

fn review_596866_ignore_color(app: &VectorcraftApp, source: NodeId) -> Value {
    let doc = &app.session.active().unwrap().doc;
    assert!(matches!(doc.node(source).unwrap().kind, NodeKind::Image(_)), "retrace retains the original source image identity");
    let trace = doc.node(doc.parent_of(source).unwrap()).unwrap();
    assert_eq!(trace.children().unwrap().first().unwrap().id, source, "the source remains the trace's first child");
    trace.extras().trace.as_ref().expect("the source's current parent is an Image Trace")["params"]["ignoreColor"].clone()
}

fn review_596866_nested_trace_selection() -> (VectorcraftApp, egui::Context, NodeId, NodeId) {
    let (mut app, ctx) = fixture(true, [37, 83, 149], true);
    let first = selected(&app).id;
    let parent = review_gesture_enclose_source(&mut app, first);
    let bytes = {
        let state = app.session.active().unwrap();
        let original = state.doc.node(first).unwrap().children().unwrap()[0].as_ref();
        let NodeKind::Image(image) = &original.kind else { panic!("the first trace retains its input") };
        state.doc.images[&image.key].bytes.clone()
    };
    app.run(
        "file.place",
        json!({"name": "second-sampler-source.png", "dataBase64": vectorcraft_format::base64_encode(&bytes), "link": false, "at": [280, 150]}),
    )
    .unwrap();
    app.run("imageTrace.make", json!({"preset": "Default", "params": {"mode": "blackAndWhite", "threshold": 127, "noise": 1, "ignoreWhite": true}}))
        .unwrap();
    let second = selected(&app).id;
    assert_ne!(first, second);
    app.run("select.set", json!({"ids": [parent.0, first.0, second.0]})).unwrap();
    settle(&mut app, &ctx);
    let state = app.session.active().unwrap();
    assert_eq!(state.selection.objects, vec![parent, first, second], "CONTROL: the public selection keeps the ordinary ancestor and nested trace");
    assert!(state.doc.node(parent).unwrap().extras().trace.is_none(), "the selected ancestor is not itself traceable");
    assert_eq!(state.doc.paint_order([first, second]), vec![first, second], "the nested trace precedes the top-level trace");
    (app, ctx, first, second)
}

#[test]
fn review_596866_preview_retraces_the_retained_sampler_binding() {
    let (mut control, control_ctx) = fixture(true, [37, 83, 149], true);
    let control_source = review_596866_pick_binding(&mut control, &control_ctx);
    assert_eq!(review_596866_ignore_color(&control, control_source), "#255395", "CONTROL: a single trace receives its sampled color");

    let (mut app, ctx, first, second) = review_596866_nested_trace_selection();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    let bound = app.ui.trace_ignore_sampler.as_ref().unwrap().target;
    assert!(bound == first || bound == second);
    let other = if bound == first { second } else { first };
    let other_before = app.session.active().unwrap().doc.node(other).unwrap().clone();
    let bound_source = review_596866_source_image(&app, bound);
    assert_ne!(review_596866_ignore_color(&app, bound_source), "#255395", "CONTROL: the sample must change the bound trace's stored color");
    let point = review_596866_bound_point(&app, bound);
    let at = review_gesture_screen(&app, point);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    review_gesture_release(&mut app, &ctx, at, start + step);
    settle(&mut app, &ctx);
    let bound_color = review_596866_ignore_color(&app, bound_source);
    let other_after = app.session.active().unwrap().doc.node(other).cloned();
    assert!(
        bound_color == "#255395" && other_after.as_ref() == Some(&other_before),
        "Preview must update only retained sampler target {bound:?}: bound_color={bound_color}, other_changed_or_missing={}",
        other_after.as_ref() != Some(&other_before)
    );
}

fn review_596866_grid_visible(app: &VectorcraftApp) -> bool {
    vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&app.session.active().unwrap().doc).visible
}

fn review_596866_hidden_grid_fixture() -> (VectorcraftApp, egui::Context) {
    let (mut app, ctx) = fixture(true, [37, 83, 149], false);
    app.select_tool("perspectiveGrid");
    assert!(review_596866_grid_visible(&app), "CONTROL: a deliberate fresh tool choice shows the grid");
    app.run("perspective.grid.show", json!({"visible": false})).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.session.tool_id(), "perspectiveGrid");
    assert!(!review_596866_grid_visible(&app));
    (app, ctx)
}

#[test]
fn review_596866_successful_pick_preserves_a_hidden_perspective_grid() {
    let (mut control, _) = review_596866_hidden_grid_fixture();
    let revision = control.session.active().unwrap().revision;
    control.select_tool("perspectiveGrid");
    assert!(review_596866_grid_visible(&control), "CONTROL: explicitly choosing the tool again still shows the grid");
    assert!(control.session.active().unwrap().revision > revision);

    let (mut app, ctx) = review_596866_hidden_grid_fixture();
    let expected = review_gesture_state(&app);
    let source = review_596866_pick_binding(&mut app, &ctx);
    assert_eq!(app.session.tool_id(), "perspectiveGrid");
    let observed = review_gesture_state(&app);
    let visible = review_596866_grid_visible(&app);
    click_control(&mut app, &ctx, "Trace");
    assert_eq!(review_596866_ignore_color(&app, source), "#255395", "CONTROL: the valid Preview-off pick really changed the panel draft");
    assert!(!visible, "returning from the temporary sampler must not show the hidden grid");
    assert_eq!(observed, expected, "the Preview-off pick must not change document, selection, revision, history or paint");
}

fn review_596866_coalesced_fresh_suffix(dragged: bool) {
    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    click_control(&mut app, &ctx, SAMPLE_TIP);
    app.run("select.none", json!({})).unwrap();
    let at = review_gesture_screen(&app, point);
    let distance = ctx.options(|o| o.input_options.max_click_dist);
    let away = at + vec2(distance + 8.0, 0.0);
    assert!(away.distance(at) > distance && app.canvas_view().unwrap().rect.contains(away));
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(app.ui.trace_ignore_sampler.is_none() && frame_input(&ctx, |i| i.pointer.primary_down()));
    let mut events = vec![review_596866_button(at, false), review_596866_button(at, true)];
    if dragged {
        events.extend([Event::PointerMoved(away), Event::PointerMoved(at)]);
    }
    events.push(review_596866_button(at, false));
    frame_at(&mut app, &ctx, start + step, events);
    assert!(frame_event(&ctx, |i| i.pointer.primary_pressed()) && frame_event(&ctx, |i| i.pointer.primary_released()));
    assert!(!frame_input(&ctx, |i| i.pointer.primary_down()));
    assert!(frame_event(&ctx, |i| i.pointer.button_clicked(PointerButton::Primary)), "the old owned release really contributed a raw click");
    assert_eq!(
        frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)),
        !dragged,
        "only the valid fresh suffix is a second raw click"
    );
    assert_eq!(app.session.active().unwrap().selection.objects, vec![group], "the fresh Down still selects the group");
    assert_eq!(app.session.active().unwrap().isolation, None, "neither the first fresh click nor an out-and-back drag may isolate");
    // The ordinary tool receives its Up in the next empty frame; no document/history oracle
    // forbids legitimate drag completion work.
    frame_at(&mut app, &ctx, start + 2.0 * step, vec![]);
    assert_eq!(app.session.active().unwrap().isolation, None);
    review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    review_gesture_release(&mut app, &ctx, at, start + 4.0 * step);
    if dragged {
        assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
        assert_eq!(app.session.active().unwrap().selection.objects, vec![group]);
        assert_eq!(app.session.active().unwrap().isolation, None, "the invalid suffix must not advance ordinary click history");
        review_gesture_press(&mut app, &ctx, at, start + 6.0 * step);
        review_gesture_release(&mut app, &ctx, at, start + 7.0 * step);
    }
    assert!(frame_event(&ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().isolation, Some(group), "the second qualifying ordinary click still isolates: dragged={dragged}");
}

#[test]
fn review_596866_coalesced_fresh_release_qualifies_its_own_click() {
    review_596866_coalesced_fresh_suffix(false);
    review_596866_coalesced_fresh_suffix(true);
}

fn review_597048_key(pressed: bool) -> Event {
    Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() }
}

#[test]
fn review_597048_escape_returns_to_a_hidden_grid_without_fresh_activation() {
    let (mut control, _) = review_596866_hidden_grid_fixture();
    control.select_tool("perspectiveGrid");
    assert!(review_596866_grid_visible(&control), "CONTROL: explicit fresh selection still shows the grid");
    let (mut app, ctx) = review_596866_hidden_grid_fixture();
    let expected = review_gesture_state(&app);
    click_control(&mut app, &ctx, SAMPLE_TIP);
    assert_eq!(app.session.tool_id(), "eyedropper");
    frame(&mut app, &ctx, vec![review_597048_key(true)]).textures_delta.clear();
    frame(&mut app, &ctx, vec![review_597048_key(false)]).textures_delta.clear();
    assert!(app.ui.trace_ignore_sampler.is_none());
    assert_eq!(app.session.tool_id(), "perspectiveGrid");
    assert!(!review_596866_grid_visible(&app), "Escape returns without revealing the grid");
    assert_eq!(review_gesture_state(&app), expected, "cancellation changes no document, revision, history, selection or paint");
}

fn review_597048_down_pixel(later_move: bool) {
    let (mut app, ctx) = fixture(true, [37, 83, 149], false);
    let source = review_596866_source_image(&app, selected(&app).id);
    let pressed = review_gesture_screen(&app, source_point(&app, 0.25));
    let hover = review_gesture_screen(&app, source_point(&app, 0.75));
    assert_ne!(pressed, hover);
    let expected = review_gesture_state(&app);
    click_control(&mut app, &ctx, SAMPLE_TIP);
    let mut events = vec![Event::PointerMoved(pressed), review_596866_button(pressed, true)];
    if later_move {
        events.push(Event::PointerMoved(hover));
    }
    frame(&mut app, &ctx, events).textures_delta.clear();
    assert_eq!(frame_input(&ctx, |i| i.pointer.interact_pos()), Some(if later_move { hover } else { pressed }));
    assert_eq!(app.session.tool_id(), "selection");
    assert!(app.ui.trace_ignore_sampler.is_none());
    assert_eq!(review_gesture_state(&app), expected, "Preview off stores a panel draft only");
    frame(&mut app, &ctx, vec![review_596866_button(if later_move { hover } else { pressed }, false)]).textures_delta.clear();
    click_control(&mut app, &ctx, "Trace");
    assert_eq!(review_596866_ignore_color(&app, source), "#255395", "the sampled pixel belongs to Down, later_move={later_move}");
}

#[test]
fn review_597048_sampler_uses_down_pixel_before_a_later_move() {
    review_597048_down_pixel(false);
    review_597048_down_pixel(true);
}

fn review_597048_coalesced_pair(sampler_prefix: bool) {
    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    let at = review_gesture_screen(&app, point);
    if sampler_prefix {
        click_control(&mut app, &ctx, SAMPLE_TIP);
    }
    app.run("select.none", json!({})).unwrap();
    let (start, step) = review_gesture_times(&ctx);
    if sampler_prefix {
        let expected = review_gesture_state(&app);
        review_gesture_press(&mut app, &ctx, at, start);
        review_gesture_release(&mut app, &ctx, at, start + step);
        assert_eq!(review_gesture_state(&app), expected, "the stale prefix was consumed in the earlier frame");
    }
    frame_at(
        &mut app,
        &ctx,
        start + 3.0 * step,
        vec![
            Event::PointerMoved(at),
            review_596866_button(at, true),
            review_596866_button(at, false),
            review_596866_button(at, true),
            review_596866_button(at, false),
        ],
    );
    assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert_eq!(frame_event(&ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)), sampler_prefix);
    // The canvas gets the second release in the frame after the second press, which the framework asks for.
    frame_at(&mut app, &ctx, start + 3.0 * step, vec![]);
    assert_eq!(app.session.active().unwrap().isolation, Some(group), "two ordinary clicks isolate after an earlier-frame prefix");
    app.run("object.exitIsolation", json!({"all": true})).unwrap();
    review_gesture_press(&mut app, &ctx, at, start + 5.0 * step);
    review_gesture_release(&mut app, &ctx, at, start + 6.0 * step);
    assert!(frame_event(&ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)));
    assert_eq!(app.session.active().unwrap().isolation, None, "the next ordinary Triple must not inherit a corrected Double");
}

#[test]
fn review_597048_two_ordinary_clicks_in_one_frame_survive_sampler_correction() {
    review_597048_coalesced_pair(false);
    review_597048_coalesced_pair(true);
}

#[test]
fn review_597048_a_later_drag_release_does_not_replace_a_qualified_double_click() {
    let (mut control, control_ctx, point, group) = review_gesture_group_fixture();
    let at = review_gesture_screen(&control, point);
    review_gesture_ordinary_pair(&mut control, &control_ctx, at, at, group);
    let (mut app, ctx, point, group) = review_gesture_group_fixture();
    app.run("select.none", json!({})).unwrap();
    let at = review_gesture_screen(&app, point);
    let blank = review_gesture_screen(&app, Point::new(340.0, 250.0));
    let distance = ctx.options(|o| o.input_options.max_click_dist);
    let away = blank + vec2(distance + 8.0, 0.0);
    assert!(app.canvas_view().unwrap().rect.contains(away) && away.distance(blank) > distance);
    let (start, step) = review_gesture_times(&ctx);
    review_gesture_press(&mut app, &ctx, at, start);
    review_gesture_release(&mut app, &ctx, at, start + step);
    review_gesture_press(&mut app, &ctx, at, start + 3.0 * step);
    frame_at(
        &mut app,
        &ctx,
        start + 4.0 * step,
        vec![
            review_596866_button(at, false),
            Event::PointerMoved(blank),
            review_596866_button(blank, true),
            Event::PointerMoved(away),
            review_596866_button(away, false),
        ],
    );
    assert!(frame_event(&ctx, |i| i.pointer.button_double_clicked(PointerButton::Primary)));
    assert!(!frame_event(&ctx, |i| i.pointer.button_triple_clicked(PointerButton::Primary)), "the out-of-slop drag is not a third click");
    assert_eq!(frame_input(&ctx, |i| i.pointer.interact_pos()), Some(away));
    assert_eq!(app.session.active().unwrap().isolation, Some(group), "the qualified Double belongs to the earlier group release");
}
