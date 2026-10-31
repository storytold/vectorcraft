use super::*;
use egui::{Event, PointerButton, Pos2, Rect, vec2};

/// One headless frame of the panel (and its menu) with `events`: the texts drawn, with where.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

/// Click the panel's `label` (the first one drawn).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let texts = frame(app, ctx, vec![]);
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(at), press(true)]);
    frame(app, ctx, vec![press(false)]);
}

fn traced_app() -> VectorcraftApp {
    traced_app_with_ink(0)
}

/// A quick trace, then the Image Trace button.
fn trace_and_open(app: &mut VectorcraftApp, preset: &str) {
    trace_with(app, preset);
    open_settings(app);
}

fn traced_app_with_ink(ink: u8) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [ink, ink, ink, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false})).unwrap();
    app.run("imageTrace.make", json!({"preset": "Default"})).unwrap();
    app
}

#[test]
fn a_large_image_traces_and_the_status_bar_says_it_is_slower() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let img = image::RgbaImage::from_fn(1601, 12, |x, _| image::Rgba(if x < 800 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "wide.png", "dataBase64": data, "link": false})).unwrap();
    trace_with(&mut app, "Default");
    assert!(target(&app).2.is_some(), "the image is traced");
    assert!(app.ui.status.contains("1601 × 12") && app.ui.status.contains("more slowly"), "{}", app.ui.status);
}

#[test]
fn a_large_image_traced_as_a_flat_logo_shows_the_size_it_was_traced_at() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let img = image::RgbaImage::from_fn(1602, 1602, |x, _| image::Rgba(if x < 800 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "logo.png", "dataBase64": data, "link": false})).unwrap();
    trace_with(&mut app, "Flat Logo");
    assert!(target(&app).2.is_some(), "the image is traced: {}", app.ui.status);
    assert_eq!(app.ui.status, "Flat Logo traced the image at 801 × 801 pixels, averaged down from 1602 × 1602.");
}

#[test]
fn sketch_quick_trace_reveals_all_adjustments_and_keeps_the_source() {
    let mut app = traced_app();
    app.run("imageTrace.release", json!({})).unwrap();
    app.ui.image_trace_advanced = false;
    trace_and_open(&mut app, "Sketched Art");
    assert!(crate::panel_docking::showing(&app.ui, "imageTrace"));
    assert!(app.ui.image_trace_advanced);
    assert_eq!(selected_trace(&app).unwrap().0, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, vec![]);
    for label in ["Threshold", "Paths", "Corners", "Noise", "Method:", "Fills", "Strokes", "Ignore Color"] {
        assert!(texts.iter().any(|(t, _)| t == label), "missing {label}: {texts:?}");
    }
    let n = first_selected(&app).unwrap();
    assert!(is_trace(&n));
    assert!(matches!(n.children().unwrap()[0].kind, NodeKind::Image(_)));
    // Opening controls again neither closes the flyout nor adds a document undo step.
    let steps = app.session.active().unwrap().history.undo.len();
    open_settings(&mut app);
    assert!(crate::panel_docking::showing(&app.ui, "imageTrace"));
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
    // Optional local visual QA; no reference images or generated captures are committed.
    if let Some(path) = std::env::var_os("VECTORCRAFT_TRACE_CAPTURE") {
        let mut ready = false;
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(340.0, 820.0)).wgpu().build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !ready {
                    egui_extras::install_image_loaders(ui.ctx());
                    crate::theme::install_fonts(ui.ctx());
                    crate::theme::apply(ui.ctx(), crate::theme::Brightness::MediumDark);
                    ready = true;
                    return;
                }
                show(app, ui);
            },
            app,
        );
        harness.run_steps(4);
        harness.render().unwrap().save(path).unwrap();
    }
}

#[test]
fn a_quick_trace_leaves_the_panel_as_it_was() {
    let mut app = traced_app();
    app.run("imageTrace.release", json!({})).unwrap();
    app.ui.open_panel = None;
    app.ui.image_trace_advanced = false;
    trace_with(&mut app, "Sketched Art");
    assert!(is_trace(&first_selected(&app).unwrap()));
    assert!(app.ui.open_panel.is_none());
    assert!(!app.ui.image_trace_advanced);
}

#[test]
fn quick_trace_activates_the_existing_floating_panel() {
    let mut app = traced_app();
    app.run("window.panel.float", json!({"panel": "imageTrace"})).unwrap();
    let before = app.ui.docking.clone().unwrap();
    trace_and_open(&mut app, "Sketched Art");
    let after = app.ui.docking.as_ref().unwrap();
    assert_eq!(after.floating.len(), before.floating.len());
    assert!(after.floating.iter().any(|g| crate::panel_docking::window_panels(g) == ["imageTrace"]));
    assert!(app.ui.open_panel.is_none(), "the existing floating group owns the panel");
}

#[test]
fn adjusting_sketch_threshold_changes_the_vector_result_and_can_be_undone() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    let rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("trace-slider", "Threshold")))).unwrap();
    let at = rail.left_center() + vec2(20.0, 0.0);
    assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 2, "source and ink path");
    for pressed in [true, false] {
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }],
        );
    }
    assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 1, "only the source remains below the ink's luminance");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 2);
    frame(&mut app, &ctx, vec![]);
    let state: TraceUi = pstate(&ctx, "image-trace");
    assert_eq!(state.params["threshold"], 150);
}

#[test]
fn with_preview_off_a_slider_change_waits_for_trace() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    assert!(app.ui.image_trace_preview, "on at first");
    click(&mut app, &ctx, "Preview");
    assert!(!app.ui.image_trace_preview);
    let steps = app.session.active().unwrap().history.undo.len();
    let rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("trace-slider", "Threshold")))).unwrap();
    let at = rail.left_center() + vec2(20.0, 0.0);
    for pressed in [true, false] {
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }],
        );
    }
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "no trace while Preview is off");
    assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 2, "source and ink path, as before");
    let draft: TraceUi = pstate(&ctx, "image-trace");
    assert_ne!(draft.params["threshold"], target(&app).2.unwrap()["params"]["threshold"], "the change waits in the panel");
    click(&mut app, &ctx, "Trace");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "Trace applies it as one undo step");
    assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 1, "only the source remains below the ink's luminance");
    assert_eq!(target(&app).2.unwrap()["params"], pstate::<TraceUi>(&ctx, "image-trace").params);
    assert!(!app.ui.image_trace_preview, "Trace leaves Preview off");
}

#[test]
fn turning_preview_on_traces_the_changes_that_waited() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let strokes = |app: &VectorcraftApp| target(app).2.map(|t| t["params"]["strokes"].clone());
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let steps = undo(&app);
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Preview");
    assert!(app.ui.image_trace_preview);
    assert_eq!(undo(&app), steps, "nothing waited, so nothing is traced");
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    assert_eq!(strokes(&app), Some(json!(false)));
    assert_eq!(undo(&app), steps);
    click(&mut app, &ctx, "Preview");
    assert_eq!(strokes(&app), Some(json!(true)), "traced again with strokes");
    assert_eq!(undo(&app), steps + 1);
    // A change of view keeps the settings that wait, and Preview turned on with `ui.set` traces them.
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Fills");
    assert_eq!(target(&app).2.unwrap()["params"]["fills"], json!(true));
    click(&mut app, &ctx, "Tracing Result");
    click(&mut app, &ctx, "Outlines");
    assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Outlines));
    assert_eq!(undo(&app), steps + 2, "the view changes without a trace");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["fills"], json!(false), "the change still waits");
    app.ui.image_trace_preview = true;
    frame(&mut app, &ctx, vec![]);
    assert_eq!(target(&app).2.unwrap()["params"]["fills"], json!(false), "traced with the waiting change");
    assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Outlines), "in the view chosen");
    assert_eq!(undo(&app), steps + 3);
}

#[test]
fn preview_turned_on_with_nothing_selected_traces_when_the_object_is_selected_again() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let a = first_selected(&app).unwrap().id;
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "the change waits");
    app.run("select.none", json!({})).unwrap();
    let steps = undo(&app);
    click(&mut app, &ctx, "Preview");
    assert!(app.ui.image_trace_preview);
    assert_eq!(undo(&app), steps, "nothing to trace with nothing selected");
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(true), "traced with the change that waited");
    assert_eq!(undo(&app), steps + 1);
}

#[test]
fn settings_that_wait_stay_with_their_object() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    assert_ne!(a, b);
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "the copy has the same settings");
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["strokes"], json!(true), "the change waits for A");
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["strokes"], json!(false), "the panel shows B's own settings");
    click(&mut app, &ctx, "Trace");
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "B is traced with its own settings");
}

#[test]
fn settings_that_wait_come_back_when_their_object_is_selected_again() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    let shown = |ctx: &egui::Context| pstate::<TraceUi>(ctx, "image-trace").params["strokes"].clone();
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown(&ctx), json!(false), "B's own settings");
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown(&ctx), json!(true), "A's change still waits");
    let steps = undo(&app);
    click(&mut app, &ctx, "Trace");
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(true), "A is traced with its change");
    assert_eq!(undo(&app), steps + 1);
    // Preview turned on while B is selected: A is traced with its change when it is selected again.
    let a = first_selected(&app).unwrap().id;
    click(&mut app, &ctx, "Strokes");
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "Preview");
    assert!(app.ui.image_trace_preview);
    let steps = undo(&app);
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "traced with the change that waited");
    assert_eq!(undo(&app), steps + 1);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(undo(&app), steps + 1, "traced once");
}

#[test]
fn settings_that_wait_are_kept_when_a_plain_image_is_traced_in_between() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let a = first_selected(&app).unwrap().id;
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    let img = image::RgbaImage::from_fn(20, 20, |_, y| image::Rgba(if y < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "plain.png", "dataBase64": data, "link": false})).unwrap();
    assert!(target(&app).1, "a plain image is selected");
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "Strokes");
    click(&mut app, &ctx, "Trace");
    assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "the image is traced with the panel's settings");
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["strokes"], json!(true), "A's change still waits");
}

#[test]
fn settings_that_wait_are_dropped_when_their_object_changed_since() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    // A's settings as they were when the change was made, other than its settings now.
    let mut st: TraceUi = pstate(&ctx, "image-trace");
    let key = (app.session.active().unwrap().uid, a);
    let draft = st.drafts.get_mut(&key).expect("the change waits for A");
    assert_eq!(draft.1["strokes"], json!(true));
    draft.2["params"]["threshold"] = json!(1);
    set_pstate(&ctx, "image-trace", st);
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let shown = pstate::<TraceUi>(&ctx, "image-trace");
    assert_eq!(shown.params, target(&app).2.unwrap()["params"], "the panel shows A's settings as they are now");
    assert!(shown.drafts.is_empty(), "the change is dropped");
}

#[test]
fn preview_is_kept_with_the_preferences_and_reset_to_default_leaves_it() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    click(&mut app, &ctx, "Preview");
    let menu_frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| menu(app, ui));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    };
    let texts = menu_frame(&mut app, vec![]);
    // Menu items draw their label after the check mark's column.
    let at = texts.iter().find(|(t, _)| t.trim() == "Reset to Default").map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{texts:?}"));
    for pressed in [true, false] {
        menu_frame(
            &mut app,
            vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }],
        );
    }
    assert!(pstate::<TraceUi>(&ctx, "image-trace").params.is_null(), "the panel's settings were reset");
    assert!(!app.ui.image_trace_preview, "Reset to Default leaves Preview");
    let saved: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(!saved.image_trace_preview);
    let old: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
    assert!(old.image_trace_preview, "on for preferences saved before Preview existed");
}

#[test]
fn sketch_adjustment_sliders_retrace_on_release() {
    let mut app = traced_app();
    trace_and_open(&mut app, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for (label, key) in [("Threshold", "threshold"), ("Paths", "paths"), ("Corners", "corners"), ("Noise", "noise")] {
        frame(&mut app, &ctx, vec![]);
        let rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("trace-slider", label)))).unwrap();
        let at = rail.left_center() + vec2(20.0, 0.0);
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let before = target(&app).2.unwrap()["params"][key].clone();
        let preset_before = pstate::<TraceUi>(&ctx, "image-trace").preset;
        let steps = app.session.active().unwrap().history.undo.len();
        frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "{label} waits for release");
        let draft: TraceUi = pstate(&ctx, "image-trace");
        assert_ne!(draft.params[key], before, "{label} updates while held");
        assert_eq!(draft.preset, preset_before, "the preset row stays in place during {label}");
        frame(&mut app, &ctx, vec![press(false)]);
        let stored = target(&app).2.unwrap();
        assert_ne!(
            stored["params"][key],
            before,
            "{label} reaches the tracer; rail {rail:?}, draft {:?}",
            pstate::<TraceUi>(&ctx, "image-trace").params
        );
        assert_eq!(stored["preset"], "Custom");
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step for {label}");
        frame(&mut app, &ctx, vec![]);
        let state: TraceUi = pstate(&ctx, "image-trace");
        assert_eq!(state.params, stored["params"], "{label} survives the next frame");
    }
}

/// Holding the eye button next to View down lists the selected Image Trace object for the
/// canvas to draw with its source image; releasing it shows the object's View again. The
/// document, its View and the undo history stay as they were. With a plain image selected the
/// button is disabled.
#[test]
fn holding_the_eye_button_shows_the_source_image_until_released() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    let id = first_selected(&app).unwrap().id;
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let revision = |app: &VectorcraftApp| app.session.active().unwrap().revision;
    let (steps, rev) = (undo(&app), revision(&app));
    let at = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("trace-hold-source"))).unwrap().center();
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
    assert_eq!(app.ui.trace_hold_source.as_deref(), Some(&vec![id]), "held: the object shows its source image");
    frame(&mut app, &ctx, vec![]);
    assert!(app.ui.trace_hold_source.is_some(), "still held on the next frame");
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.ui.trace_hold_source, None, "released: the object shows its View again");
    assert_eq!(undo(&app), steps, "no undo step");
    assert_eq!(revision(&app), rev, "the document is unchanged");
    assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Result), "its View is unchanged");
    app.run("imageTrace.release", json!({})).unwrap();
    assert!(matches!(first_selected(&app).unwrap().kind, NodeKind::Image(_)));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
    assert_eq!(app.ui.trace_hold_source, None, "a plain image has no source image to show");
    frame(&mut app, &ctx, vec![press(false)]);
}

/// With another object first in the selection, the eye button and the panel menu's Release
/// act on the selected Image Trace object.
#[test]
fn the_trace_controls_work_when_another_object_is_selected_first() {
    let mut app = traced_app();
    let trace_id = first_selected(&app).unwrap().id;
    let made = app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 10, "height": 10})).unwrap();
    let rect = made["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [rect, trace_id.0]})).unwrap();
    assert_eq!(first_selected(&app).unwrap().id.0, rect, "the rectangle comes first");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    let at = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("trace-hold-source"))).unwrap().center();
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
    assert_eq!(app.ui.trace_hold_source.as_deref(), Some(&vec![trace_id]), "the eye button shows the source image");
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.ui.trace_hold_source, None);

    let menu_frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    };
    let texts = menu_frame(&mut app, vec![]);
    let at = texts.iter().find(|(t, _)| t.trim() == "Release").map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no Release in {texts:?}"));
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    menu_frame(&mut app, vec![Event::PointerMoved(at), press(true)]);
    menu_frame(&mut app, vec![press(false)]);
    let st = app.session.active().unwrap();
    assert!(st.doc.node(trace_id).is_none(), "Release puts the source image back");
}

/// With another object first in the selection, the panel's View menu and Expand act on the
/// selected Image Trace object.
#[test]
fn view_and_expand_work_when_another_object_is_selected_first() {
    let mut app = traced_app();
    let trace_id = first_selected(&app).unwrap().id;
    let made = app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 10, "height": 10})).unwrap();
    let rect = made["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [rect, trace_id.0]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    click(&mut app, &ctx, "Tracing Result");
    click(&mut app, &ctx, "Outlines");
    let view = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(trace_id).unwrap().trace_view();
    assert_eq!(view(&app), TraceView::Outlines, "the View menu sets the Image Trace object's view");
    click(&mut app, &ctx, "Expand");
    let st = app.session.active().unwrap();
    assert!(!is_trace(st.doc.node(trace_id).unwrap()), "Expand keeps only the traced paths");
}

/// With another object first in the selection, Trace traces the selected image, the View
/// dropdown shows the selected Image Trace object's view, and Preview traces it again.
#[test]
fn trace_view_and_preview_work_when_another_object_is_selected_first() {
    let mut app = traced_app();
    app.run("imageTrace.release", json!({})).unwrap();
    let image = first_selected(&app).unwrap().id;
    let made = app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 10, "height": 10})).unwrap();
    let rect = made["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [rect, image.0]})).unwrap();
    assert_eq!(first_selected(&app).unwrap().id.0, rect, "the rectangle comes first");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    click(&mut app, &ctx, "Trace");
    let st = app.session.active().unwrap();
    let trace_id = st.selection.objects.iter().copied().find(|id| st.doc.node(*id).is_some_and(is_trace));
    let trace_id = trace_id.unwrap_or_else(|| panic!("Trace makes an Image Trace object: {}", app.ui.status));

    // The panel last showed Tracing Result; the object is drawn as Outlines.
    frame(&mut app, &ctx, vec![]);
    app.run("imageTrace.setView", json!({"view": "outlines"})).unwrap();
    app.run("select.set", json!({"ids": [rect, trace_id.0]})).unwrap();
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Outlines") && !texts.iter().any(|(t, _)| t == "Tracing Result"), "{texts:?}");

    // Tracing again replaces the object, so look it up in the selection.
    let strokes = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let n = st.selection.objects.iter().find_map(|id| st.doc.node(*id).filter(|n| is_trace(n)));
        n.and_then(|n| n.extras().trace.as_deref().cloned()).map(|t| t["params"]["strokes"].clone())
    };
    assert_eq!(strokes(&app), Some(json!(false)));
    click(&mut app, &ctx, "Strokes");
    assert_eq!(strokes(&app), Some(json!(true)), "Preview traces the object again");
}

#[test]
fn properties_can_reopen_the_trace_controls() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut at = None;
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::properties::show(&mut app, ui));
        let texts = crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>());
        // The first Image Trace text is the section heading, the last is the button.
        at = texts.iter().rev().find(|(t, _)| t == "Image Trace").map(|(_, r)| r.center());
        out.textures_delta.clear();
    }
    let at = at.unwrap();
    for pressed in [true, false] {
        let events =
            vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }];
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::properties::show(&mut app, ui));
        out.textures_delta.clear();
    }
    assert!(crate::panel_docking::showing(&app.ui, "imageTrace"));
}

/// #1358: in Properties the View row fits the panel, so the panel doesn't widen and clip its
/// labels (the row took the panel's whole width for its menu, beside its label).
#[test]
fn the_view_row_fits_the_properties_panel() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // Where the panel's content ends after a few frames.
    let right = |app: &mut VectorcraftApp| {
        let mut right = 0.0;
        for _ in 0..3 {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                crate::panels::properties::show(app, ui);
                right = ui.min_rect().right();
            });
            out.textures_delta.clear();
        }
        right
    };
    let traced = right(&mut app);
    app.run("select.none", json!({})).unwrap();
    assert_eq!(traced, right(&mut app));
}

#[test]
fn advanced_is_a_section_that_stays_as_it_was_left() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(app.ui.image_trace_advanced && has(&texts, "Advanced") && has(&texts, "Corners"), "open at first: {texts:?}");
    click(&mut app, &ctx, "Advanced");
    assert!(!app.ui.image_trace_advanced, "the header closes it");
    assert!(!has(&frame(&mut app, &ctx, vec![]), "Corners"));
    // Remembered with the UI preferences, across launches.
    let saved: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(!saved.image_trace_advanced);
    let old: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
    assert!(old.image_trace_advanced, "open for preferences saved before the section existed");
    click(&mut app, &ctx, "Advanced");
    assert!(app.ui.image_trace_advanced && has(&frame(&mut app, &ctx, vec![]), "Corners"));
}

#[test]
fn the_view_dropdown_shows_and_sets_the_objects_view() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "View:") && texts.iter().any(|(t, _)| t == "Tracing Result"), "{texts:?}");
    let steps = app.session.active().unwrap().history.undo.len();
    click(&mut app, &ctx, "Tracing Result");
    click(&mut app, &ctx, "Source Image");
    assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Source));
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step, no new trace");
    // Another object's view is shown when it is selected.
    app.run("imageTrace.setView", json!({"view": "outlines"})).unwrap();
    assert!(frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Outlines"));
}

#[test]
fn the_palette_shows_in_color_mode_and_traces_again() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    let palette = |app: &VectorcraftApp| target(app).2.map(|t| t["params"]["palette"].clone());
    assert!(!has(&frame(&mut app, &ctx, vec![]), "Palette:"), "Black and White has no palette");
    app.run("imageTrace.make", json!({"preset": "6 Colors"})).unwrap();
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Palette:") && has(&texts, "Limited") && !has(&texts, "Library:"), "{texts:?}");
    // Document Library traces again with the document's swatches, and names them.
    click(&mut app, &ctx, "Limited");
    click(&mut app, &ctx, "Document Library");
    assert_eq!(palette(&app), Some(json!("documentLibrary")));
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Library:") && has(&texts, "Document Swatches"), "{texts:?}");
    // Full Tone's Colors is how many, Less (0%) … More (100%).
    click(&mut app, &ctx, "Document Library");
    click(&mut app, &ctx, "Full Tone");
    assert_eq!(palette(&app), Some(json!("fullTone")));
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "50") && !has(&texts, "Library:"), "{texts:?}");
}

#[test]
fn document_library_colors_lists_the_documents_color_groups() {
    let mut app = traced_app();
    app.run("swatch.newGroup", json!({"name": "Brand", "colors": ["#000000", "#ffffff"]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "6 Colors", "params": {"palette": "documentLibrary"}})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    let stored = |app: &VectorcraftApp, key: &str| target(app).2.map(|t| t["params"][key].clone());
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Colors:") && has(&texts, "All"), "{texts:?}");
    // The list holds All and the document's groups; a group traces again with its colors.
    click(&mut app, &ctx, "All");
    assert!(has(&frame(&mut app, &ctx, vec![]), "   Brand"));
    let steps = app.session.active().unwrap().history.undo.len();
    click(&mut app, &ctx, "   Brand");
    assert_eq!(stored(&app, "colorGroup"), Some(json!("Brand")));
    assert_eq!(target(&app).2.unwrap()["preset"], "Custom");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);
    assert!(has(&frame(&mut app, &ctx, vec![]), "Brand"));
    // All again.
    click(&mut app, &ctx, "Brand");
    click(&mut app, &ctx, "   All");
    assert_eq!(stored(&app, "colorGroup"), Some(Value::Null));
    // Another library clears the group, and the row goes.
    click(&mut app, &ctx, "All");
    click(&mut app, &ctx, "   Brand");
    let lib = swatches::colour_libraries(&app).into_iter().find(|l| l.submenu.is_none()).unwrap();
    click(&mut app, &ctx, "Document Swatches");
    click(&mut app, &ctx, &format!("   {}", library_panel::library_name(&lib.id, &lib.name)));
    assert_eq!((stored(&app, "library"), stored(&app, "colorGroup")), (Some(json!(lib.id)), Some(Value::Null)));
    assert!(!has(&frame(&mut app, &ctx, vec![]), "Colors:"));
    // Another palette clears the group too, and traces again (the object stores the settings of
    // a trace that was made).
    click(&mut app, &ctx, library_panel::library_name(&lib.id, &lib.name));
    click(&mut app, &ctx, "   Document Swatches");
    click(&mut app, &ctx, "All");
    click(&mut app, &ctx, "   Brand");
    assert_eq!(stored(&app, "colorGroup"), Some(json!("Brand")));
    click(&mut app, &ctx, "Document Library");
    click(&mut app, &ctx, "Limited");
    assert_eq!((stored(&app, "palette"), stored(&app, "colorGroup")), (Some(json!("limited")), Some(Value::Null)));
    // Another mode clears it too, and traces again.
    click(&mut app, &ctx, "Limited");
    click(&mut app, &ctx, "Document Library");
    click(&mut app, &ctx, "All");
    click(&mut app, &ctx, "   Brand");
    assert_eq!(stored(&app, "colorGroup"), Some(json!("Brand")));
    click(&mut app, &ctx, "Color");
    click(&mut app, &ctx, "Grayscale");
    assert_eq!((stored(&app, "mode"), stored(&app, "colorGroup")), (Some(json!("grayscale")), Some(Value::Null)));
}

#[test]
fn a_color_group_the_active_document_lacks_is_all() {
    let mut app = traced_app();
    app.run("swatch.newGroup", json!({"name": "Brand", "colors": ["#000000", "#ffffff"]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "6 Colors", "params": {"palette": "documentLibrary"}})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    click(&mut app, &ctx, "All");
    click(&mut app, &ctx, "   Brand");
    assert_eq!(target(&app).2.map(|t| t["params"]["colorGroup"].clone()), Some(json!("Brand")));
    let shows_brand = |app: &mut VectorcraftApp| frame(app, &ctx, vec![]).iter().any(|(t, _)| t == "Brand");
    // Without the group the row shows All; undoing its removal shows it again.
    app.run("swatch.ungroup", json!({"name": "Brand"})).unwrap();
    assert!(!shows_brand(&mut app));
    app.run("edit.undo", json!({})).unwrap();
    assert!(shows_brand(&mut app));
    // Another document without the group, with an image that is not traced yet.
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false})).unwrap();
    let texts = frame(&mut app, &ctx, vec![]);
    let has = |s: &str| texts.iter().any(|(t, _)| t == s);
    assert!(has("Colors:") && has("All") && !has("Brand"), "{texts:?}");
    // Back in the first document, its Image Trace object shows its group again.
    app.session.set_active(0);
    assert!(shows_brand(&mut app));
    app.session.set_active(1);
    assert!(!shows_brand(&mut app));
    // Trace uses every document swatch.
    click(&mut app, &ctx, "Trace");
    let stored = target(&app).2.unwrap_or_else(|| panic!("not traced: {}", app.ui.status));
    assert!(stored["params"]["colorGroup"].as_str().is_none_or(str::is_empty), "{stored}");
    assert_eq!(stored["params"]["palette"], "documentLibrary");
}

#[test]
fn create_strokes_traces_again_and_keeps_one_kind_on() {
    let mut app = traced_app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let create = |app: &VectorcraftApp| target(app).2.map(|t| (t["params"]["fills"].clone(), t["params"]["strokes"].clone()));
    let texts = frame(&mut app, &ctx, vec![]);
    for s in ["Create:", "Fills", "Strokes", "Stroke", "10"] {
        assert!(texts.iter().any(|(t, _)| t == s), "{s}: {texts:?}");
    }
    // Fills alone can't be turned off.
    click(&mut app, &ctx, "Fills");
    assert_eq!(create(&app), Some((json!(true), json!(false))));
    click(&mut app, &ctx, "Strokes");
    assert_eq!(create(&app), Some((json!(true), json!(true))), "traced again with strokes");
    click(&mut app, &ctx, "Fills");
    assert_eq!(create(&app), Some((json!(false), json!(true))));
    click(&mut app, &ctx, "Strokes");
    assert_eq!(create(&app), Some((json!(false), json!(true))), "Strokes alone stays on");
}

/// Click at `at`, then draw a frame.
fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(at), press(true)]);
    frame(app, ctx, vec![press(false)]);
    frame(app, ctx, vec![]);
}

/// Choose `item` from Manage Presets, next to the Preset dropdown. Whether it was enabled.
fn manage(app: &mut VectorcraftApp, ctx: &egui::Context, item: &str) -> bool {
    frame(app, ctx, vec![]);
    let button = ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("trace-manage-presets"))).unwrap();
    click_at(app, ctx, button.center());
    let texts = frame(app, ctx, vec![]);
    let at = texts.iter().find(|(t, _)| t.trim() == item).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{item}` in {texts:?}"));
    let state = |app: &VectorcraftApp| (app.ui.dialog.as_ref().map(|d| (d.kind.clone(), d.fields.clone())), app.session.prefs.trace_presets.clone());
    let before = state(app);
    click_at(app, ctx, at);
    // Escape closes the menu when the item was disabled.
    frame(app, ctx, vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    before != state(app)
}

/// The preset the panel shows.
fn shown_preset(ctx: &egui::Context) -> String {
    pstate::<TraceUi>(ctx, "image-trace").preset
}

#[test]
fn manage_presets_saves_renames_and_deletes_a_preset_of_the_panel_settings() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // A built-in preset can't be renamed or deleted.
    assert!(!manage(&mut app, &ctx, "Rename…"));
    assert!(!manage(&mut app, &ctx, "Delete"));
    // Save as New Preset… asks for a name, and the panel then shows the preset.
    assert!(manage(&mut app, &ctx, "Save as New Preset…"));
    let d = app.ui.dialog.clone().unwrap();
    assert_eq!((d.kind.as_str(), d.str("name")), (crate::dialogs::trace_presets::KIND, "Tracing Preset 1".to_string()));
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Heavy Ink"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let texts = frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Heavy Ink");
    assert!(texts.iter().any(|(t, _)| t == "Heavy Ink"), "{texts:?}");
    let saved = &app.session.prefs.trace_presets;
    assert_eq!((saved.len(), saved[0].name.as_str(), saved[0].params.threshold), (1, "Heavy Ink", 150));
    // It follows the built-in presets in the preset list (the Preset dropdown, Control bar and
    // Properties).
    let list_ctx = egui::Context::default();
    crate::theme::install_fonts(&list_ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 1200.0));
    let mut out = list_ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
        preset_list(&mut app, ui, "Heavy Ink");
    });
    out.textures_delta.clear();
    let shapes: Vec<_> = out.shapes.iter().map(|c| c.shape.clone()).collect();
    let listed: Vec<String> = crate::tests_removeanchors::shapes_text(&shapes).into_iter().map(|(t, _)| t).collect();
    assert_eq!(listed.iter().position(|t| t == "Heavy Ink"), Some(presets(&mut app, &list_ctx).len() - 1), "{listed:?}");
    assert_eq!(listed.len(), presets(&mut app, &list_ctx).len(), "{listed:?}");
    // Rename… starts from its name, and the panel follows the new one.
    assert!(manage(&mut app, &ctx, "Rename…"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "Heavy Ink");
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Dark Ink"));
    crate::dialogs::confirm(&mut app).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Dark Ink");
    assert_eq!(app.session.prefs.trace_presets[0].name, "Dark Ink");
    // Tracing with it gives the object its name.
    click(&mut app, &ctx, "Trace");
    assert_eq!(selected_trace(&app).unwrap().0, "Dark Ink");
    // Delete: the settings stay, as Custom.
    assert!(manage(&mut app, &ctx, "Delete"));
    assert!(app.session.prefs.trace_presets.is_empty());
    assert_eq!(shown_preset(&ctx), "Custom");
    assert!(!manage(&mut app, &ctx, "Delete"));
}

#[test]
fn renaming_another_saved_preset_with_the_same_settings_keeps_the_panel_preset() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    let save = |app: &mut VectorcraftApp, p: Value| {
        app.session.execute("imageTrace.presets.save", &p).unwrap();
        frame(app, &ctx, vec![]);
        shown_preset(&ctx)
    };
    assert_eq!(save(&mut app, json!({ "name": "A", "preset": "Sketched Art" })), "A");
    assert_eq!(save(&mut app, json!({ "name": "B", "preset": "Sketched Art" })), "B");
    // A, with the same settings as B, is renamed: the panel still shows B.
    assert_eq!(save(&mut app, json!({ "name": "A", "newName": "C" })), "B");
    // B itself is renamed: the panel follows it.
    assert_eq!(save(&mut app, json!({ "name": "B", "newName": "E" })), "E");
}

/// A saved preset renamed or deleted while the settings that wait for another object name it: that object, selected
/// again, shows the preset's new name, or Custom.
#[test]
fn settings_that_wait_follow_a_renamed_or_deleted_preset() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art", "params": {"threshold": 40}})).unwrap();
    trace_and_open(&mut app, "Sketched Art");
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "Preview");
    // Ink is chosen for A with Preview off, so it waits for A.
    let ink = presets(&mut app, &ctx).iter().find(|p| p.0 == "Ink").unwrap().1.clone();
    let mut st: TraceUi = pstate(&ctx, "image-trace");
    st.preset = "Ink".into();
    st.params = ink;
    set_pstate(&ctx, "image-trace", st);
    let select = |app: &mut VectorcraftApp, id: NodeId| {
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        frame(app, &ctx, vec![]);
        shown_preset(&ctx)
    };
    assert_eq!(select(&mut app, b), "Sketched Art");
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "newName": "Dark Ink"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(select(&mut app, a), "Dark Ink", "A's waiting settings follow the rename");
    assert_eq!(select(&mut app, b), "Sketched Art");
    app.session.execute("imageTrace.presets.delete", &json!({"name": "Dark Ink"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(select(&mut app, a), "Custom", "A's waiting settings name a deleted preset");
    assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["threshold"], json!(40), "and keep their settings");
}

#[test]
fn a_refused_preset_name_keeps_the_dialog_open() {
    let mut app = traced_app_with_ink(100);
    for name in ["A", "B"] {
        app.session.execute("imageTrace.presets.save", &json!({ "name": name })).unwrap();
    }
    crate::dialogs::trace_presets::open(&mut app, &json!({"rename": "A"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("b"));
    assert!(crate::dialogs::confirm(&mut app).is_err());
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("name")).as_deref(), Some("b"), "the name entered stays, to be changed");
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("C"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let names: Vec<String> = app.session.prefs.trace_presets.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["C", "B"]);
}

#[test]
fn selecting_an_object_traced_with_a_deleted_preset_shows_custom() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Keep", "preset": "Sketched Art"})).unwrap();
    trace_and_open(&mut app, "Keep");
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Keep");
    // A, traced again with a built-in preset, is shown while Keep is deleted.
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "Sketched Art"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.session.execute("imageTrace.presets.delete", &json!({"name": "Keep"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Sketched Art");
    // B still records Keep.
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    assert_eq!(selected_trace(&app).unwrap().0, "Keep");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Custom");
    // A built-in preset named in another case or in brackets shows under its own name, and Trace
    // with the settings unchanged keeps it.
    app.run("imageTrace.make", json!({"preset": "[sketched art]"})).unwrap();
    assert_eq!(selected_trace(&app).unwrap().0, "[sketched art]");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Sketched Art");
    click(&mut app, &ctx, "Trace");
    assert_eq!(selected_trace(&app).unwrap().0, "Sketched Art");
}

/// An object traced with a saved preset that was renamed since shows the new name when it is selected again, and
/// when the panel opens again.
#[test]
fn selecting_an_object_traced_with_a_renamed_preset_shows_the_new_name() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art"})).unwrap();
    trace_and_open(&mut app, "Ink");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Ink");
    app.session.execute("imageTrace.presets.save", &json!({"name": "ink", "newName": "Dark Ink"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Dark Ink");
    app.run("select.set", json!({"ids": []})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("select.all", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(selected_trace(&app).unwrap().0, "Ink", "the object still records the old name");
    assert_eq!(shown_preset(&ctx), "Dark Ink");
    let reopened = egui::Context::default();
    crate::theme::install_fonts(&reopened);
    frame(&mut app, &reopened, vec![]);
    assert_eq!(shown_preset(&reopened), "Dark Ink");
    // Dark Ink is deleted, and another preset with the same settings is made under that name: the object shows
    // Custom, also after that preset is renamed.
    app.session.execute("imageTrace.presets.delete", &json!({"name": "Dark Ink"})).unwrap();
    app.session.execute("imageTrace.presets.save", &json!({"name": "Dark Ink", "preset": "Sketched Art"})).unwrap();
    let shown = |app: &mut VectorcraftApp| {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(app, &ctx, vec![]);
        shown_preset(&ctx)
    };
    assert_eq!(shown(&mut app), "Custom");
    app.session.execute("imageTrace.presets.save", &json!({"name": "Dark Ink", "newName": "Other Ink"})).unwrap();
    assert_eq!(shown(&mut app), "Custom");
}

/// The Control bar's and Properties' Preset dropdowns show an object traced with a saved preset under the name the
/// panel shows: the new name after a rename, and Custom once the preset's settings changed or it was deleted.
#[test]
fn the_control_bar_and_properties_show_a_renamed_or_changed_preset_as_the_panel_does() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art"})).unwrap();
    trace_with(&mut app, "Ink");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = |app: &mut VectorcraftApp, properties: bool| {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            if properties {
                crate::panels::properties::show(app, ui);
            } else {
                ui.horizontal(|ui| control_bar(app, ui));
            }
        });
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    };
    let shows = |app: &mut VectorcraftApp, name: &str| {
        for properties in [false, true] {
            let t = texts(app, properties);
            assert!(t.iter().any(|(s, _)| s == name), "{} shows {name}: {t:?}", if properties { "Properties" } else { "Control bar" });
        }
    };
    shows(&mut app, "Ink");
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "newName": "Dark Ink"})).unwrap();
    shows(&mut app, "Dark Ink");
    app.session.execute("imageTrace.presets.save", &json!({"name": "Dark Ink", "preset": "Silhouettes"})).unwrap();
    shows(&mut app, "Custom");
    app.session.execute("imageTrace.presets.delete", &json!({"name": "Dark Ink"})).unwrap();
    shows(&mut app, "Custom");
}

/// An object traced with a preset whose name another saved preset had earlier in the session, renamed since: the
/// panel can't tell which of the two the object was traced with, and shows Custom.
#[test]
fn an_object_traced_with_a_reused_preset_name_that_was_renamed_shows_custom() {
    let mut app = traced_app_with_ink(100);
    let save = |app: &mut VectorcraftApp, p: Value| {
        app.session.execute("imageTrace.presets.save", &p).unwrap();
    };
    save(&mut app, json!({"name": "A", "preset": "Sketched Art"}));
    save(&mut app, json!({"name": "A", "newName": "B"}));
    save(&mut app, json!({"name": "A", "preset": "Sketched Art"}));
    trace_and_open(&mut app, "A");
    save(&mut app, json!({"name": "A", "newName": "C"}));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(selected_trace(&app).unwrap().0, "A");
    assert_eq!(shown_preset(&ctx), "Custom");
}

/// An object traced with a preset that got its name from a rename, renamed again since: the panel shows the new name.
#[test]
fn an_object_traced_between_two_renames_shows_the_last_name() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "A", "preset": "Sketched Art"})).unwrap();
    app.session.execute("imageTrace.presets.save", &json!({"name": "A", "newName": "B"})).unwrap();
    trace_and_open(&mut app, "B");
    app.session.execute("imageTrace.presets.save", &json!({"name": "B", "newName": "C"})).unwrap();
    let shown = |app: &mut VectorcraftApp| {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(app, &ctx, vec![]);
        shown_preset(&ctx)
    };
    assert_eq!(selected_trace(&app).unwrap().0, "B");
    assert_eq!(shown(&mut app), "C");
    // Renamed back to B and on to D, it is still the same preset.
    app.session.execute("imageTrace.presets.save", &json!({"name": "C", "newName": "B"})).unwrap();
    app.session.execute("imageTrace.presets.save", &json!({"name": "B", "newName": "D"})).unwrap();
    assert_eq!(shown(&mut app), "D");
}

/// Presets renamed between two frames while the object is not selected: the object, selected again, shows its own
/// preset's new name, not the name a rename of another preset gave.
#[test]
fn renames_between_frames_leave_a_reselected_object_on_its_own_preset() {
    let mut app = traced_app_with_ink(100);
    for name in ["A", "B"] {
        app.session.execute("imageTrace.presets.save", &json!({"name": name, "preset": "Sketched Art"})).unwrap();
    }
    trace_and_open(&mut app, "A");
    let a = first_selected(&app).unwrap().id;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "A");
    app.run("select.set", json!({"ids": []})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.session.execute("imageTrace.presets.save", &json!({"name": "B", "newName": "C"})).unwrap();
    app.session.execute("imageTrace.presets.save", &json!({"name": "A", "newName": "B"})).unwrap();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "B");
}

/// Saved presets whose names differ only by brackets are different presets: an object traced with
/// a deleted one shows Custom.
#[test]
fn a_saved_preset_in_brackets_is_not_another_saved_preset() {
    let mut app = traced_app_with_ink(100);
    for name in ["Ink", "[Ink]"] {
        app.session.execute("imageTrace.presets.save", &json!({"name": name, "preset": "Sketched Art"})).unwrap();
    }
    trace_and_open(&mut app, "[Ink]");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "[Ink]");
    app.session.execute("imageTrace.presets.delete", &json!({"name": "[Ink]"})).unwrap();
    app.run("select.set", json!({"ids": []})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("select.all", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(selected_trace(&app).unwrap().0, "[Ink]");
    assert_eq!(shown_preset(&ctx), "Custom");
}

/// A saved preset whose settings change outside the panel while the panel shows it with other
/// settings: the panel shows Custom, and Trace records Custom with the panel's settings.
#[test]
fn a_saved_preset_changed_elsewhere_leaves_the_panel_settings_as_custom() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art"})).unwrap();
    trace_and_open(&mut app, "Ink");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Ink");
    let before = pstate::<TraceUi>(&ctx, "image-trace").params;
    // A save with the same settings keeps the name.
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Ink");
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "params": {"threshold": 40}})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Custom");
    assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params, before, "the panel's settings stay");
    // A rename that also changes the settings.
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink 2", "preset": "Sketched Art"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Ink 2");
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink 2", "newName": "Ink 3", "params": {"threshold": 40}})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(shown_preset(&ctx), "Custom");
    // The object, traced with Ink before its settings changed, shows Custom when the panel opens
    // again.
    assert_eq!(selected_trace(&app).unwrap().0, "Ink");
    let reopened = egui::Context::default();
    crate::theme::install_fonts(&reopened);
    frame(&mut app, &reopened, vec![]);
    assert_eq!(shown_preset(&reopened), "Custom");
}

#[test]
fn the_preset_list_is_read_once_for_each_change_of_the_saved_presets() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let reads = || PRESET_READS.with(std::cell::Cell::get);
    frame(&mut app, &ctx, vec![]);
    let start = reads();
    for _ in 0..5 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(reads(), start, "frames with the same saved presets read the list again");
    // A preset saved outside the panel is read once, and the panel shows it.
    app.session.execute("imageTrace.presets.save", &json!({"name": "A", "preset": "Sketched Art"})).unwrap();
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    assert_eq!((reads(), shown_preset(&ctx)), (start + 1, "A".to_string()));
    // The rename's reply names the old name, and the panel follows the new one.
    let r = app.session.execute("imageTrace.presets.save", &json!({"name": "a", "newName": "B"})).unwrap();
    assert_eq!(r["renamedFrom"], "A");
    frame(&mut app, &ctx, vec![]);
    assert_eq!((reads(), shown_preset(&ctx)), (start + 2, "B".to_string()));
    // Preferences that replace the saved presets are read again too.
    let mut prefs = app.session.prefs.clone();
    prefs.trace_presets.clear();
    app.session.apply_prefs(prefs);
    frame(&mut app, &ctx, vec![]);
    assert_eq!((reads(), shown_preset(&ctx)), (start + 3, "Custom".to_string()));
    // A preset saved with the panel's settings and renamed before the next frame is shown under
    // its new name.
    app.session.execute("imageTrace.presets.save", &json!({"name": "C", "preset": "Sketched Art"})).unwrap();
    app.session.execute("imageTrace.presets.save", &json!({"name": "C", "newName": "D"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!((reads(), shown_preset(&ctx)), (start + 4, "D".to_string()));
}

#[test]
fn panel_and_menu_draw_headless() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    for _ in 0..2 {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui);
            menu(&mut app, ui);
        });
        out.textures_delta.clear();
    }
}

#[test]
fn ignore_color_rejects_signs_in_hex() {
    assert_eq!(ignore_color(&json!({"ignoreColor": "#+f+f+f"})), [255; 3]);
    assert_eq!(ignore_color(&json!({"ignoreColor": "+0+0+0"})), [255; 3]);
    assert_eq!(ignore_color(&json!({"ignoreColor": "#ff00ff"})), [255, 0, 255]);
}

/// #3098: a trace the panel starts off the UI thread is the panel's own once it lands: its counts
/// show, and nothing waits for the object it replaced, so Undo with Preview on shows that object's
/// own settings and traces nothing.
#[test]
fn a_trace_off_the_ui_thread_is_adopted_when_it_lands() {
    let mut app = traced_app();
    app.trace_runs.off_the_ui_thread = true;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    app.adopt_context(&ctx);
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let strokes = |app: &VectorcraftApp| target(app).2.map(|t| t["params"]["strokes"].clone());
    let steps = undo(&app);
    click(&mut app, &ctx, "Preview");
    click(&mut app, &ctx, "Strokes");
    click(&mut app, &ctx, "Trace");
    assert_eq!(app.trace_runs.running.len(), 1);
    assert_eq!(undo(&app), steps, "not traced yet");
    let t0 = std::time::Instant::now();
    while !app.trace_runs.running.is_empty() && t0.elapsed() < std::time::Duration::from_secs(30) {
        crate::trace_runs::poll(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(undo(&app), steps + 1, "one undo step");
    assert_eq!(strokes(&app), Some(json!(true)));
    let st: TraceUi = pstate(&ctx, "image-trace");
    assert!(st.info.is_some(), "its counts show");
    assert_eq!(st.object, target_key(&app));
    assert!(st.drafts.is_empty(), "nothing waits for the object it replaced");
    app.ui.image_trace_preview = true;
    frame(&mut app, &ctx, vec![]);
    app.run("edit.undo", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(app.trace_runs.running.is_empty(), "Undo traces nothing");
    assert_eq!(strokes(&app), Some(json!(false)), "the object's own settings");
    assert_eq!(undo(&app), steps);
}
