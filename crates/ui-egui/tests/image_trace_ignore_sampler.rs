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
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let pixels: Vec<u8> = (0..400).flat_map(|i| if i % 20 < 10 { [rgb[0], rgb[1], rgb[2], 255] } else { [255; 4] }).collect();
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
fn miss_escape_and_explicit_tool_choice_do_not_sample_or_paint() {
    for exit in ["outside", "escape", "rectangle", "eyedropper"] {
        let (mut app, ctx) = fixture(true, [37, 83, 149], false);
        let old_color = settings(&app)["params"]["ignoreColor"].clone();
        let state = app.session.active().unwrap();
        let (document, selection, revision) = (state.doc.clone(), state.selection.clone(), state.revision);
        let history = (state.history.undo.len(), state.history.redo.len());
        let paint = app.session.paint.clone();
        click_control(&mut app, &ctx, SAMPLE_TIP);
        assert_eq!(app.session.tool_id(), "eyedropper");
        match exit {
            "outside" => {
                let view = app.view_info();
                canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, -100.0, -100.0), view);
            }
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
        let expected = if exit == "outside" || exit == "escape" { "selection" } else { exit };
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
fn actual_canvas_shows_original_while_armed_then_restores_the_stored_view() {
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
    let source_hash = blob.content_key();
    let stored = settings(&app);
    let (before, before_key) = capture_canvas(&mut app, &ctx);
    assert_eq!(pixel_at(&before, &before_key, point), [0, 0, 0, 255]);
    assert_eq!(pixel_at(&before, &before_key, control), [255; 4]);
    click_control(&mut app, &ctx, SAMPLE_TIP);
    let (armed, armed_key) = capture_canvas(&mut app, &ctx);
    assert!(!ctx.input(|i| i.pointer.any_down()), "the source remains visible after releasing the button");
    assert_eq!(armed_key.trace_source.as_deref().map(Vec::as_slice), Some([target].as_slice()));
    assert_eq!(pixel_at(&armed, &armed_key, point), [37, 83, 149, 255]);
    assert_eq!(pixel_at(&armed, &armed_key, control), [255; 4]);
    app.select_tool("selection");
    let (restored, restored_key) = capture_canvas(&mut app, &ctx);
    assert!(restored_key.trace_source.is_none());
    assert_eq!(restored.pixels, before.pixels, "the exact stored result returns");
    let state = app.session.active().unwrap();
    assert_eq!((state.uid, state.revision), (uid, revision));
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    if let Some(dir) = std::env::var_os("VECTORCRAFT_TRACE_SAMPLER_EVIDENCE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, image) in [("before.png", &before), ("armed.png", &armed), ("restored.png", &restored)] {
            std::fs::write(dir.join(name), image.to_png().unwrap()).unwrap();
        }
        let binding = json!({"doc": uid, "target": target.0, "revision": revision, "storedTrace": stored, "sourceContentKey": source_hash,
            "point": [point.x,point.y], "controlPoint": [control.x,control.y], "width": before.width, "height": before.height,
            "before": pixel_at(&before,&before_key,point), "armed": pixel_at(&armed,&armed_key,point), "restored": pixel_at(&restored,&restored_key,point),
            "sourceIdsWhileArmed": [target.0], "undoRedoLengths": [history.0,history.1], "documentUnchanged": true,
            "view": {"zoom":before_key.zoom,"center":[before_key.cx,before_key.cy],"rotation":before_key.rot,"pixelsPerPoint":before_key.ppp}});
        std::fs::write(dir.join("binding.json"), serde_json::to_vec_pretty(&binding).unwrap()).unwrap();
    }
}
