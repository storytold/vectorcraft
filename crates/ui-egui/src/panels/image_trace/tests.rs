use super::*;
use crate::panels::first_selected;
use crate::panels::swatches::DOCUMENT_SWATCHES as DOCUMENT_SWATCHES_NAME;
use crate::tests::panel_harness::Panel;
use craft::behavior::Mixed;
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;

/// The name of the eye button next to View.
const EYE: &str = "Press and hold to view the source image";

/// The name of the button that opens the preset row's menu.
const MANAGE: &str = "Manage presets";

/// The Image Trace panel over `app`.
fn panel_over(app: VectorcraftApp) -> Panel {
    Panel::new(app, show)
}

/// The panel's settings as it shows them.
fn shown(panel: &Panel) -> TraceUi {
    panel.app().ui.image_trace_panel.shown.clone()
}

/// The preset the panel shows.
fn shown_preset(panel: &Panel) -> String {
    panel.app().ui.image_trace_panel.shown.preset.clone()
}

/// Runs the engine command `id` with `params` outside the panel, then a frame.
fn execute(panel: &mut Panel, id: &str, params: Value) {
    panel.app().session.execute(id, &params).unwrap();
    panel.frame();
}

/// Selects `id`, then a frame: the preset the panel shows.
fn select(panel: &mut Panel, id: NodeId) -> String {
    panel.app().run("select.set", json!({"ids": [id.0]})).unwrap();
    panel.frame();
    shown_preset(panel)
}

/// The undo steps of the active document.
fn undo(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The trace setting `key` the target object stores.
fn param(app: &VectorcraftApp, key: &str) -> Option<Value> {
    find_target(app).stored.map(|t| t["params"][key].clone())
}

/// The children of the first selected object: the source image and the traced paths.
fn children(app: &VectorcraftApp) -> usize {
    first_selected(app).unwrap().children().unwrap().len()
}

/// The number a field's value text shows, its unit left out.
fn number(text: &str) -> Option<f64> {
    text.trim_end_matches('%').trim_end_matches("px").trim().parse().ok()
}

/// Chooses `preset` from the panel's Preset picker.
fn choose(panel: &mut Panel, preset: &str) {
    panel.click("Preset");
    panel.click(preset);
}

/// The presets of the buttons above the Preset row that show as on.
fn selected_preset_buttons(panel: &Panel) -> Vec<&'static str> {
    preset_buttons().into_iter().filter(|(label, _)| panel.toggled(label) == Some(Mixed::Same(true))).map(|(_, name)| name).collect()
}

fn traced_app() -> VectorcraftApp {
    traced_app_with_ink(0)
}

/// A quick trace, then the Image Trace button.
fn trace_and_open(app: &mut VectorcraftApp, preset: &str) {
    trace_with(app, preset);
    open_settings(app);
}

/// Places `img` as an unlinked PNG named `name` in the active document.
fn place_png(app: &mut VectorcraftApp, name: &str, img: image::RgbaImage) {
    let (w, h) = img.dimensions();
    let raw = craft_image_codecs::Image::from_raw(w, h, craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, img.into_raw())
        .unwrap();
    let png = craft_image_codecs::encode(&raw, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
    app.run("file.place", json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&png), "link": false})).unwrap();
}

/// An image whose left half has the luminance `ink`, traced with Default.
fn traced_app_with_ink(ink: u8) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [ink, ink, ink, 255] } else { [255, 255, 255, 255] }));
    place_png(&mut app, "half.png", img);
    app.run("imageTrace.make", json!({"preset": "Default"})).unwrap();
    app
}

/// Revert reloads the saved file even when its trace has the same node id and stored settings.
/// A preview-off draft belongs to the discarded source pixels, not the reloaded object.
#[test]
fn native_file_revert_discards_pending_settings_for_reused_trace_ids() {
    for frame_without_selection in [false, true] {
        let mut app = traced_app_with_ink(0);
        let old = find_target(&app);
        let id = old.object.unwrap().1;
        let old_source = source_image(app.session.active().unwrap().doc.node(id).unwrap()).unwrap().key.clone();
        let dir = vectorcraft_testkit::TempDir::new("trace-native-revert");
        let path = dir.join("saved.vectorcraft");
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
        app.run("object.move", json!({"ids": [id.0], "dx": 1, "dy": 0})).unwrap();

        // Another saved copy supplies different pixels with the same trace identity and settings.
        let mut saved = traced_app_with_ink(100);
        let restored = find_target(&saved);
        assert_eq!(restored.object.unwrap().1, id);
        assert_eq!(old.stored.as_ref().unwrap()["params"], restored.stored.as_ref().unwrap()["params"]);
        saved.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        saved.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();

        let mut panel = panel_over(app);
        panel.click("Preview");
        panel.click("Strokes");
        assert!(!panel.app().ui.image_trace_preview);
        assert_eq!(shown(&panel).params["strokes"], json!(true));
        assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)), "the changed setting remains a panel draft");
        let uid = panel.app().session.active().unwrap().uid;
        let reverts = panel.app().session.active().unwrap().reverts;
        panel.app().run("file.revert", json!({"confirmed": true})).unwrap();
        {
            let app = panel.app();
            let st = app.session.active().unwrap();
            assert_eq!(st.uid, uid);
            assert_eq!(st.reverts, reverts + 1, "the native Revert replaced the drawing contents");
            assert!(st.selection.objects.is_empty());
            assert_ne!(source_image(st.doc.node(id).unwrap()).unwrap().key, old_source, "Revert read the other saved pixels");
        }
        if frame_without_selection {
            panel.frame();
        }
        select(&mut panel, id);
        assert_eq!(shown(&panel).params, restored.stored.unwrap()["params"], "the saved trace's own settings are adopted");
        assert!(shown(&panel).drafts.is_empty(), "the discarded source's draft is gone");
        let before = undo(&panel.app());
        panel.click("Preview");
        assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)));
        assert_eq!(undo(&panel.app()), before, "Preview must not apply the discarded source's draft");
    }
}

#[test]
fn closing_a_drawing_discards_only_its_own_trace_draft() {
    for park_closed_draft in [true, false] {
        let mut panel = panel_over(traced_app());
        let remaining = shown(&panel).object.unwrap();
        panel.click("Preview");
        panel.click("Strokes");
        assert_eq!(shown(&panel).params["strokes"], json!(true));

        let dir = vectorcraft_testkit::TempDir::new("trace-native-close");
        let path = dir.join("second.vectorcraft");
        {
            let mut app = panel.app();
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [100, 100, 100, 255] } else { [255, 255, 255, 255] }));
            place_png(&mut app, "second.png", img);
            app.run("imageTrace.make", json!({"preset": "Default"})).unwrap();
            app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
            app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
            assert!(!app.session.active().unwrap().is_dirty());
        }
        panel.frame();
        let closed = shown(&panel).object.unwrap();
        assert_ne!(remaining.0, closed.0);
        panel.click("Strokes");
        assert_eq!(shown(&panel).params["strokes"], json!(true));
        assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)));
        if park_closed_draft {
            panel.app().run("select.none", json!({})).unwrap();
            panel.frame();
            assert!(shown(&panel).drafts.contains_key(&closed));
        }
        assert!(shown(&panel).drafts.contains_key(&remaining));

        panel.app().run("file.close", json!({})).unwrap();
        assert_eq!(panel.app().session.documents().len(), 1);
        assert_eq!(panel.app().session.active().unwrap().uid, remaining.0);
        let after_close = shown(&panel);
        assert!(!after_close.drafts.contains_key(&closed), "File Close releases the closed drawing's draft");
        assert_ne!(after_close.object, Some(closed), "File Close releases the displayed draft's binding too");
        assert!(after_close.drafts.contains_key(&remaining), "File Close preserves the open drawing's draft");

        select(&mut panel, remaining.2);
        assert!(shown(&panel).drafts.is_empty(), "the next frame does not put back the closed drawing's draft");
        assert_eq!(shown(&panel).params["strokes"], json!(true));
        assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)));
        panel.click("Preview");
        assert_eq!(param(&panel.app(), "strokes"), Some(json!(true)));
    }
}

fn scroll_settings(panel: &mut Panel, dy: f32) {
    panel.scroll_at(egui::pos2(30.0, 30.0), dy);
    panel.advance(1.0);
    panel.frame();
}

#[test]
fn preview_trace_and_expand_stay_reachable_in_a_compact_panel() {
    let mut app = traced_app();
    app.run("imageTrace.release", json!({})).unwrap();
    app.ui.image_trace_advanced = true;
    let source = first_selected(&app).unwrap();
    assert!(matches!(source.kind, NodeKind::Image(_)));
    let mut panel = panel_over(app);
    assert!(panel.rect_role("Image Trace", craft::ui::contract::Role::GROUP).is_positive());
    for size in [egui::vec2(148.0, 480.0), egui::vec2(240.0, 170.0), egui::vec2(148.0, 170.0)] {
        panel.room(size);
        panel.frame();
        panel.frame();
        let room = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        for label in ["Preview", "Trace", "Expand"] {
            assert!(panel.in_view(label), "{label} is clipped at {size:?}");
            let rect = panel.rect(label);
            assert!(room.contains_rect(rect), "the whole {label} control {rect:?} fits at {size:?}");
        }
        assert!(panel.enabled("Trace"));
        assert!(!panel.enabled("Expand"));
    }
    let preview = panel.app().ui.image_trace_preview;
    panel.click("Preview");
    assert_ne!(panel.app().ui.image_trace_preview, preview);

    let action = panel.rect("Trace");
    let first_preset = panel.rect("Auto Color").top();
    scroll_settings(&mut panel, -1000.0);
    assert!(!panel.shows("Auto Color") || panel.rect("Auto Color").top() < first_preset, "the settings scroll");
    assert_eq!(panel.rect("Trace"), action, "scrolling settings leaves the action in place");
    let ignore = shown(&panel).params["ignoreWhite"].clone();
    panel.click("Ignore Color");
    assert_ne!(shown(&panel).params["ignoreWhite"], ignore, "scrolling reaches the end of Advanced");

    panel.click("Trace");
    assert!(selected_trace(&panel.app()).is_some());
    assert!(panel.enabled("Expand"));
    panel.click("Expand");
    assert!(selected_trace(&panel.app()).is_none());
    assert!(!is_trace(&first_selected(&panel.app()).unwrap()));
    panel.app().run("edit.undo", json!({})).unwrap();
    assert!(selected_trace(&panel.app()).is_some());

    // A single action row needs less height than the wrapped footer in the narrow panel.
    panel.room(egui::vec2(240.0, 100.0));
    panel.frame();
    panel.frame();
    scroll_settings(&mut panel, 1000.0);
    let action = panel.rect("Trace");
    let first_preset = panel.rect("Auto Color").top();
    scroll_settings(&mut panel, -1000.0);
    assert!(!panel.shows("Auto Color") || panel.rect("Auto Color").top() < first_preset, "the short settings viewport scrolls");
    assert!(panel.in_view("Trace"));
    assert_eq!(panel.rect("Trace"), action, "a short panel still pins the footer when the actions fit on one line");

    // Shorten the panel while the settings-only body remains scrolled to Advanced.
    panel.room(egui::vec2(148.0, 68.0));
    panel.frame();
    panel.frame();
    let preview = panel.app().ui.image_trace_preview;
    panel.click("Preview");
    assert_ne!(panel.app().ui.image_trace_preview, preview, "Preview starts in view in a very short panel");
    let ignore = shown(&panel).params["ignoreWhite"].clone();
    scroll_settings(&mut panel, -1000.0);
    panel.click("Ignore Color");
    assert_ne!(shown(&panel).params["ignoreWhite"], ignore, "a very short panel still scrolls to the last setting");

    panel.room(egui::vec2(240.0, 100.0));
    panel.frame();
    panel.room(egui::vec2(148.0, 68.0));
    panel.frame();
    panel.frame();
    let preview = panel.app().ui.image_trace_preview;
    panel.click("Preview");
    assert_ne!(panel.app().ui.image_trace_preview, preview, "shrinking a second time starts at the actions");
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
    let panel = panel_over(app);
    for label in ["Threshold", "Paths", "Corners", "Noise", "Method:", "Fills", "Strokes", "Ignore Color"] {
        assert!(panel.shows(label), "missing {label}");
    }
    let mut app = panel.app();
    let n = first_selected(&app).unwrap();
    assert!(is_trace(&n));
    assert!(matches!(n.children().unwrap()[0].kind, NodeKind::Image(_)));
    // Opening controls again neither closes the flyout nor adds a document undo step.
    let steps = undo(&app);
    open_settings(&mut app);
    assert!(crate::panel_docking::showing(&app.ui, "imageTrace"));
    assert_eq!(undo(&app), steps);
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
    let floating = |app: &VectorcraftApp| -> Vec<Vec<String>> {
        app.ui.dock_workspace.layout().floating.iter().map(crate::panel_docking::window_panels).collect()
    };
    let before = floating(&app);
    trace_and_open(&mut app, "Sketched Art");
    let after = floating(&app);
    assert_eq!(after.len(), before.len());
    assert!(after.iter().any(|panels| panels == &["imageTrace"]));
    assert!(app.ui.open_panel.is_none(), "the existing floating group owns the panel");
}

#[test]
fn adjusting_sketch_threshold_changes_the_vector_result_and_can_be_undone() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let mut panel = panel_over(app);
    assert_eq!(children(&panel.app()), 2, "source and ink path");
    panel.type_into("Threshold", "50");
    assert_eq!(children(&panel.app()), 1, "only the source remains below the ink's luminance");
    panel.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(children(&panel.app()), 2);
    panel.frame();
    assert_eq!(shown(&panel).params["threshold"], 150);
}

#[test]
fn with_preview_off_a_slider_change_waits_for_trace() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let mut panel = panel_over(app);
    assert!(panel.app().ui.image_trace_preview, "on at first");
    panel.click("Preview");
    assert!(!panel.app().ui.image_trace_preview);
    let steps = undo(&panel.app());
    panel.type_into("Threshold", "50");
    assert_eq!(undo(&panel.app()), steps, "no trace while Preview is off");
    assert_eq!(children(&panel.app()), 2, "source and ink path, as before");
    assert_ne!(Some(shown(&panel).params["threshold"].clone()), param(&panel.app(), "threshold"), "the change waits in the panel");
    panel.click("Trace");
    assert_eq!(undo(&panel.app()), steps + 1, "Trace applies it as one undo step");
    assert_eq!(children(&panel.app()), 1, "only the source remains below the ink's luminance");
    let stored = find_target(&panel.app()).stored.unwrap();
    assert_eq!(stored["params"], shown(&panel).params);
    assert!(!panel.app().ui.image_trace_preview, "Trace leaves Preview off");
}

#[test]
fn turning_preview_on_traces_the_changes_that_waited() {
    let mut panel = panel_over(traced_app());
    let steps = undo(&panel.app());
    panel.click("Preview");
    panel.click("Preview");
    assert!(panel.app().ui.image_trace_preview);
    assert_eq!(undo(&panel.app()), steps, "nothing waited, so nothing is traced");
    panel.click("Preview");
    panel.click("Strokes");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)));
    assert_eq!(undo(&panel.app()), steps);
    panel.click("Preview");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(true)), "traced again with strokes");
    assert_eq!(undo(&panel.app()), steps + 1);
    // A change of view keeps the settings that wait, and Preview turned on with `ui.set` traces them.
    panel.click("Preview");
    panel.click("Fills");
    assert_eq!(param(&panel.app(), "fills"), Some(json!(true)));
    panel.click("View");
    panel.click("Outlines");
    assert_eq!(selected_trace(&panel.app()).map(|t| t.1), Some(TraceView::Outlines));
    assert_eq!(undo(&panel.app()), steps + 2, "the view changes without a trace");
    panel.frame();
    assert_eq!(shown(&panel).params["fills"], json!(false), "the change still waits");
    panel.app().ui.image_trace_preview = true;
    panel.frame();
    assert_eq!(param(&panel.app(), "fills"), Some(json!(false)), "traced with the waiting change");
    assert_eq!(selected_trace(&panel.app()).map(|t| t.1), Some(TraceView::Outlines), "in the view chosen");
    assert_eq!(undo(&panel.app()), steps + 3);
}

#[test]
fn preview_turned_on_with_nothing_selected_traces_when_the_object_is_selected_again() {
    let app = traced_app();
    let a = first_selected(&app).unwrap().id;
    let mut panel = panel_over(app);
    panel.click("Preview");
    panel.click("Strokes");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)), "the change waits");
    panel.app().run("select.none", json!({})).unwrap();
    let steps = undo(&panel.app());
    panel.click("Preview");
    assert!(panel.app().ui.image_trace_preview);
    assert_eq!(undo(&panel.app()), steps, "nothing to trace with nothing selected");
    select(&mut panel, a);
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(true)), "traced with the change that waited");
    assert_eq!(undo(&panel.app()), steps + 1);
}

#[test]
fn settings_that_wait_stay_with_their_object() {
    let mut app = traced_app();
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    assert_ne!(a, b);
    assert_eq!(param(&app, "strokes"), Some(json!(false)), "the copy has the same settings");
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let mut panel = panel_over(app);
    panel.click("Preview");
    panel.click("Strokes");
    assert_eq!(shown(&panel).params["strokes"], json!(true), "the change waits for A");
    select(&mut panel, b);
    assert_eq!(shown(&panel).params["strokes"], json!(false), "the panel shows B's own settings");
    panel.click("Trace");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)), "B is traced with its own settings");
}

#[test]
fn settings_that_wait_come_back_when_their_object_is_selected_again() {
    let mut app = traced_app();
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let mut panel = panel_over(app);
    let strokes = |panel: &Panel| shown(panel).params["strokes"].clone();
    panel.click("Preview");
    panel.click("Strokes");
    select(&mut panel, b);
    assert_eq!(strokes(&panel), json!(false), "B's own settings");
    select(&mut panel, a);
    assert_eq!(strokes(&panel), json!(true), "A's change still waits");
    let steps = undo(&panel.app());
    panel.click("Trace");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(true)), "A is traced with its change");
    assert_eq!(undo(&panel.app()), steps + 1);
    // Preview turned on while B is selected: A is traced with its change when it is selected again.
    let a = first_selected(&panel.app()).unwrap().id;
    panel.click("Strokes");
    select(&mut panel, b);
    panel.click("Preview");
    assert!(panel.app().ui.image_trace_preview);
    let steps = undo(&panel.app());
    select(&mut panel, a);
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)), "traced with the change that waited");
    assert_eq!(undo(&panel.app()), steps + 1);
    panel.frame();
    assert_eq!(undo(&panel.app()), steps + 1, "traced once");
}

#[test]
fn settings_that_wait_are_kept_when_a_plain_image_is_traced_in_between() {
    let app = traced_app();
    let a = first_selected(&app).unwrap().id;
    let mut panel = panel_over(app);
    panel.click("Preview");
    panel.click("Strokes");
    let img = image::RgbaImage::from_fn(20, 20, |_, y| image::Rgba(if y < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    place_png(&mut panel.app(), "plain.png", img);
    assert!(find_target(&panel.app()).is_image, "a plain image is selected");
    panel.frame();
    panel.click("Strokes");
    panel.click("Trace");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)), "the image is traced with the panel's settings");
    select(&mut panel, a);
    assert_eq!(shown(&panel).params["strokes"], json!(true), "A's change still waits");
}

#[test]
fn settings_that_wait_are_dropped_when_their_object_changed_since() {
    let mut app = traced_app();
    let a = first_selected(&app).unwrap().id;
    app.run("edit.duplicate", json!({})).unwrap();
    let b = first_selected(&app).unwrap().id;
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let mut panel = panel_over(app);
    panel.click("Preview");
    panel.click("Strokes");
    select(&mut panel, b);
    // A's settings as they were when the change was made, other than its settings now.
    {
        let mut app = panel.app();
        let doc = app.session.active().unwrap();
        let key = (doc.uid, doc.reverts, a);
        let draft = app.ui.image_trace_panel.shown.drafts.get_mut(&key).expect("the change waits for A");
        assert_eq!(draft.1["strokes"], json!(true));
        draft.2["params"]["threshold"] = json!(1);
    }
    select(&mut panel, a);
    let st = shown(&panel);
    assert_eq!(st.params, find_target(&panel.app()).stored.unwrap()["params"], "the panel shows A's settings as they are now");
    assert!(st.drafts.is_empty(), "the change is dropped");
}

#[test]
fn preview_is_kept_with_the_preferences_and_reset_to_default_leaves_it() {
    let mut panel = panel_over(traced_app());
    panel.click("Preview");
    let mut app = panel.app();
    app.run("ui.imageTrace.reset", json!({})).unwrap();
    assert!(app.ui.image_trace_panel.shown.params.is_null(), "the panel's settings were reset");
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
    let mut panel = panel_over(app);
    for (label, key) in [("Threshold", "threshold"), ("Paths", "paths"), ("Corners", "corners"), ("Noise", "noise")] {
        let before = param(&panel.app(), key).unwrap();
        let to = if before == json!(90) { 10 } else { 90 };
        let steps = undo(&panel.app());
        panel.type_into(label, &to.to_string());
        let stored = find_target(&panel.app()).stored.unwrap();
        assert_ne!(stored["params"][key], before, "{label} reaches the tracer");
        assert_eq!(stored["preset"], "Custom");
        assert_eq!(undo(&panel.app()), steps + 1, "one undo step for {label}");
        panel.frame();
        assert_eq!(shown(&panel).params, stored["params"], "{label} survives the next frame");
    }
}

/// Holding the eye button next to View down lists the selected Image Trace object for the
/// canvas to draw with its source image; releasing it shows the object's View again. The
/// document, its View and the undo history stay as they were. With a plain image selected the
/// button is disabled.
#[test]
fn holding_the_eye_button_shows_the_source_image_until_released() {
    let app = traced_app();
    let id = first_selected(&app).unwrap().id;
    let revision = |app: &VectorcraftApp| app.session.active().unwrap().revision;
    let (steps, rev) = (undo(&app), revision(&app));
    let mut panel = panel_over(app);
    panel.press(EYE);
    assert_eq!(panel.app().ui.trace_hold_source.as_deref(), Some(&vec![id]), "held: the object shows its source image");
    panel.frame();
    assert!(panel.app().ui.trace_hold_source.is_some(), "still held on the next frame");
    panel.release();
    assert_eq!(panel.app().ui.trace_hold_source, None, "released: the object shows its View again");
    assert_eq!(undo(&panel.app()), steps, "no undo step");
    assert_eq!(revision(&panel.app()), rev, "the document is unchanged");
    assert_eq!(selected_trace(&panel.app()).map(|t| t.1), Some(TraceView::Result), "its View is unchanged");
    panel.app().run("imageTrace.release", json!({})).unwrap();
    assert!(matches!(first_selected(&panel.app()).unwrap().kind, NodeKind::Image(_)));
    panel.frame();
    panel.press(EYE);
    assert_eq!(panel.app().ui.trace_hold_source, None, "a plain image has no source image to show");
    panel.release();
}

/// A divider between whole percents (as a canvas drag or an agent sets it) stays as it is while
/// the panel shows, with no undo step; an edit of the Divider slider sets it as one undo step.
#[test]
fn the_divider_slider_keeps_an_exact_divider_and_a_drag_is_one_undo_step() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = traced_app();
    app.run("imageTrace.setView", json!({"view": "split", "split": 0.3025})).unwrap();
    let steps = undo(&app);
    let mut panel = panel_over(app);
    panel.frame();
    assert_eq!(first_selected(&panel.app()).unwrap().trace_split(), 0.3025, "the panel leaves the divider as it is");
    assert_eq!(undo(&panel.app()), steps, "no undo step");
    panel.type_into("Divider", "80");
    let split = first_selected(&panel.app()).unwrap().trace_split();
    assert!(split > 0.6, "the edit moved the divider: {split}");
    assert_eq!(undo(&panel.app()), steps + 1, "one undo step for the edit");
}

/// Properties shows the Divider slider of the selected Image Trace object in the Split view only,
/// with its name and value for screen readers.
#[test]
fn properties_shows_the_divider_slider_in_the_split_view() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut panel = Panel::new(traced_app(), crate::panels::properties::show);
    assert!(!panel.shows("Divider"), "no divider in the Tracing Result view");
    panel.app().run("imageTrace.setView", json!({"view": "split", "split": 0.4})).unwrap();
    panel.frame();
    assert_eq!(panel.value("Divider").as_deref().and_then(number), Some(40.0));
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
    let mut panel = panel_over(app);
    panel.press(EYE);
    assert_eq!(panel.app().ui.trace_hold_source.as_deref(), Some(&vec![trace_id]), "the eye button shows the source image");
    panel.release();
    assert_eq!(panel.app().ui.trace_hold_source, None);
    panel.app().run("imageTrace.release", json!({})).unwrap();
    assert!(panel.app().session.active().unwrap().doc.node(trace_id).is_none(), "Release puts the source image back");
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
    let mut panel = panel_over(app);
    panel.click("View");
    panel.click("Outlines");
    let view = panel.app().session.active().unwrap().doc.node(trace_id).unwrap().trace_view();
    assert_eq!(view, TraceView::Outlines, "the View menu sets the Image Trace object's view");
    panel.click("Expand");
    assert!(!is_trace(panel.app().session.active().unwrap().doc.node(trace_id).unwrap()), "Expand keeps only the traced paths");
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
    let mut panel = panel_over(app);
    panel.click("Trace");
    let trace_id = {
        let app = panel.app();
        let st = app.session.active().unwrap();
        let found = st.selection.objects.iter().copied().find(|id| st.doc.node(*id).is_some_and(is_trace));
        found.unwrap_or_else(|| panic!("Trace makes an Image Trace object: {}", app.ui.status))
    };

    // The panel last showed Tracing Result; the object is drawn as Outlines.
    panel.app().run("imageTrace.setView", json!({"view": "outlines"})).unwrap();
    panel.app().run("select.set", json!({"ids": [rect, trace_id.0]})).unwrap();
    panel.frame();
    assert_eq!(panel.value("View").as_deref(), Some("Outlines"));

    // Tracing again replaces the object, so look it up in the selection.
    let strokes = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let n = st.selection.objects.iter().find_map(|id| st.doc.node(*id).filter(|n| is_trace(n)));
        n.and_then(|n| n.extras().trace.as_deref().cloned()).map(|t| t["params"]["strokes"].clone())
    };
    assert_eq!(strokes(&panel.app()), Some(json!(false)));
    panel.click("Strokes");
    assert_eq!(strokes(&panel.app()), Some(json!(true)), "Preview traces the object again");
}

#[test]
fn advanced_is_a_section_that_stays_as_it_was_left() {
    let mut panel = panel_over(traced_app());
    assert!(panel.app().ui.image_trace_advanced && panel.shows("Advanced") && panel.shows("Corners"), "open at first");
    // The section's region and heading share its name: the header's disclosure is the button.
    panel.click_role("Advanced", craft::ui::contract::Role::BUTTON);
    assert!(!panel.app().ui.image_trace_advanced, "the header closes it");
    // Once the section has drawn itself closed (it animates), as production read it on a later frame.
    panel.advance(1.0);
    panel.frame();
    assert!(!panel.shows("Corners"));
    // Remembered with the UI preferences, across launches.
    let saved: crate::state::UiState = serde_json::from_value(serde_json::to_value(&panel.app().ui).unwrap()).unwrap();
    assert!(!saved.image_trace_advanced);
    let old: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
    assert!(old.image_trace_advanced, "open for preferences saved before the section existed");
    panel.click_role("Advanced", craft::ui::contract::Role::BUTTON);
    panel.frame();
    assert!(panel.app().ui.image_trace_advanced && panel.shows("Corners"));
}

#[test]
fn the_palette_shows_in_color_mode_and_traces_again() {
    let mut panel = panel_over(traced_app());
    assert!(!panel.shows("Palette:"), "Black and White has no palette");
    panel.app().run("imageTrace.make", json!({"preset": "6 Colors"})).unwrap();
    panel.frame();
    assert_eq!(panel.value("Palette:").as_deref(), Some("Limited"));
    assert!(!panel.shows("Library:"));
    // Document Library traces again with the document's swatches, and names them.
    panel.click("Palette:");
    panel.click("Document Library");
    assert_eq!(param(&panel.app(), "palette"), Some(json!("documentLibrary")));
    assert_eq!(panel.value("Library:").as_deref(), Some(DOCUMENT_SWATCHES_NAME));
    // Full Tone's Colors is how many, Less (0%) … More (100%).
    panel.click("Palette:");
    panel.click("Full Tone");
    assert_eq!(param(&panel.app(), "palette"), Some(json!("fullTone")));
    assert_eq!(panel.value("Colors").as_deref().and_then(number), Some(50.0));
    assert!(!panel.shows("Library:"));
}

#[test]
fn document_library_colors_lists_the_documents_color_groups() {
    let mut app = traced_app();
    app.run("swatch.newGroup", json!({"name": "Brand", "colors": ["#000000", "#ffffff"]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "6 Colors", "params": {"palette": "documentLibrary"}})).unwrap();
    let (lib, lib_label) = RowsData::of(&app)
        .libraries
        .iter()
        .find_map(|e| e.value().filter(|id| id.as_str() != DOCUMENT_SWATCHES).map(|id| (id.clone(), e.label().as_str().to_owned())))
        .unwrap();
    let mut panel = panel_over(app);
    let pick = |panel: &mut Panel, control: &str, entry: &str| {
        panel.click(control);
        panel.click(entry);
    };
    assert_eq!(panel.value("Colors:").as_deref(), Some("All"));
    // The list holds All and the document's groups; a group traces again with its colors.
    let steps = undo(&panel.app());
    pick(&mut panel, "Colors:", "Brand");
    assert_eq!(param(&panel.app(), "colorGroup"), Some(json!("Brand")));
    assert_eq!(find_target(&panel.app()).stored.unwrap()["preset"], "Custom");
    assert_eq!(undo(&panel.app()), steps + 1);
    assert_eq!(panel.value("Colors:").as_deref(), Some("Brand"));
    // All again.
    pick(&mut panel, "Colors:", "All");
    assert_eq!(param(&panel.app(), "colorGroup"), Some(Value::Null));
    // Another library clears the group, and the row goes.
    pick(&mut panel, "Colors:", "Brand");
    pick(&mut panel, "Library:", &lib_label);
    assert_eq!(
        {
            let app = panel.app();
            (param(&app, "library"), param(&app, "colorGroup"))
        },
        (Some(json!(lib)), Some(Value::Null))
    );
    assert!(!panel.shows("Colors:"));
    // Another palette clears the group too, and traces again (the object stores the settings of
    // a trace that was made).
    pick(&mut panel, "Library:", DOCUMENT_SWATCHES_NAME);
    pick(&mut panel, "Colors:", "Brand");
    assert_eq!(param(&panel.app(), "colorGroup"), Some(json!("Brand")));
    pick(&mut panel, "Palette:", "Limited");
    assert_eq!(
        {
            let app = panel.app();
            (param(&app, "palette"), param(&app, "colorGroup"))
        },
        (Some(json!("limited")), Some(Value::Null))
    );
    // Another mode clears it too, and traces again.
    pick(&mut panel, "Palette:", "Document Library");
    pick(&mut panel, "Colors:", "Brand");
    assert_eq!(param(&panel.app(), "colorGroup"), Some(json!("Brand")));
    pick(&mut panel, "Mode:", "Flat Logo");
    assert_eq!(
        {
            let app = panel.app();
            (param(&app, "mode"), param(&app, "colorGroup"))
        },
        (Some(json!("logo")), Some(Value::Null))
    );
}

#[test]
fn a_color_group_the_active_document_lacks_is_all() {
    let mut app = traced_app();
    app.run("swatch.newGroup", json!({"name": "Brand", "colors": ["#000000", "#ffffff"]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "6 Colors", "params": {"palette": "documentLibrary"}})).unwrap();
    let mut panel = panel_over(app);
    panel.click("Colors:");
    panel.click("Brand");
    assert_eq!(param(&panel.app(), "colorGroup"), Some(json!("Brand")));
    let shows_brand = |panel: &mut Panel| {
        panel.frame();
        panel.value("Colors:").as_deref() == Some("Brand")
    };
    // Without the group the row shows All; undoing its removal shows it again.
    panel.app().run("swatch.ungroup", json!({"name": "Brand"})).unwrap();
    assert!(!shows_brand(&mut panel));
    panel.app().run("edit.undo", json!({})).unwrap();
    assert!(shows_brand(&mut panel));
    // Another document without the group, with an image that is not traced yet.
    {
        let mut app = panel.app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
        place_png(&mut app, "half.png", img);
    }
    panel.frame();
    assert_eq!(panel.value("Colors:").as_deref(), Some("All"));
    // Back in the first document, its Image Trace object shows its group again.
    panel.app().session.set_active(0);
    assert!(shows_brand(&mut panel));
    panel.app().session.set_active(1);
    assert!(!shows_brand(&mut panel));
    // Trace uses every document swatch.
    panel.click("Trace");
    let app = panel.app();
    let stored = find_target(&app).stored.unwrap_or_else(|| panic!("not traced: {}", app.ui.status));
    assert!(stored["params"]["colorGroup"].as_str().is_none_or(str::is_empty), "{stored}");
    assert_eq!(stored["params"]["palette"], "documentLibrary");
}

#[test]
fn create_strokes_traces_again_and_keeps_one_kind_on() {
    let mut panel = panel_over(traced_app());
    let create = |panel: &Panel| {
        let app = panel.app();
        (param(&app, "fills"), param(&app, "strokes"))
    };
    for s in ["Create:", "Fills", "Strokes", "Stroke"] {
        assert!(panel.shows(s), "{s}");
    }
    assert_eq!(panel.value("Stroke").as_deref().and_then(number), Some(10.0));
    // Fills alone can't be turned off.
    panel.click("Fills");
    assert_eq!(create(&panel), (Some(json!(true)), Some(json!(false))));
    panel.click("Strokes");
    assert_eq!(create(&panel), (Some(json!(true)), Some(json!(true))), "traced again with strokes");
    panel.click("Fills");
    assert_eq!(create(&panel), (Some(json!(false)), Some(json!(true))));
    panel.click("Strokes");
    assert_eq!(create(&panel), (Some(json!(false)), Some(json!(true))), "Strokes alone stays on");
}

#[test]
fn manage_presets_saves_renames_and_deletes_a_preset_of_the_panel_settings() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let suggested = app.session.new_trace_preset_name();
    let mut panel = panel_over(app);
    // Save as New Preset… offers a name, and the panel then shows the preset.
    panel.click(MANAGE);
    panel.click("Save as New Preset…");
    assert_eq!(panel.value("Name").as_deref(), Some(suggested.as_str()));
    panel.click("OK");
    assert_eq!(shown_preset(&panel), suggested);
    {
        let app = panel.app();
        let saved = &app.session.prefs.trace_presets;
        assert_eq!((saved.len(), saved[0].name.as_str(), saved[0].params.threshold), (1, suggested.as_str(), 150));
    }
    // It follows the built-in presets in the preset list (the Preset row, Control bar and
    // Properties).
    let listed = presets(&mut panel.app());
    assert_eq!(listed.presets.last().map(|p| p.0.as_str()), Some(suggested.as_str()));
    // Rename… starts from its name, and the panel follows the new one.
    panel.click(MANAGE);
    panel.click("Rename…");
    assert_eq!(panel.value("Name").as_deref(), Some(suggested.as_str()));
    panel.type_into("Name", "Dark Ink");
    assert_eq!(shown_preset(&panel), "Dark Ink");
    assert_eq!(panel.app().session.prefs.trace_presets[0].name, "Dark Ink");
    // Tracing with it gives the object its name.
    panel.click("Trace");
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "Dark Ink");
    // Delete: the settings stay, as Custom.
    panel.click(MANAGE);
    panel.click("Delete");
    assert!(panel.app().session.prefs.trace_presets.is_empty());
    assert_eq!(shown_preset(&panel), "Custom");
}

#[test]
fn renaming_another_saved_preset_with_the_same_settings_keeps_the_panel_preset() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let mut panel = panel_over(app);
    let mut save = |p: Value| {
        execute(&mut panel, "imageTrace.presets.save", p);
        shown_preset(&panel)
    };
    assert_eq!(save(json!({ "name": "A", "preset": "Sketched Art" })), "A");
    assert_eq!(save(json!({ "name": "B", "preset": "Sketched Art" })), "B");
    // A, with the same settings as B, is renamed: the panel still shows B.
    assert_eq!(save(json!({ "name": "A", "newName": "C" })), "B");
    // B itself is renamed: the panel follows it.
    assert_eq!(save(json!({ "name": "B", "newName": "E" })), "E");
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
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let mut panel = panel_over(app);
    panel.click("Preview");
    // Ink is chosen for A with Preview off, so it waits for A.
    {
        let mut app = panel.app();
        let ink = presets(&mut app).presets.iter().find(|p| p.0 == "Ink").unwrap().1.clone();
        let st = &mut app.ui.image_trace_panel.shown;
        st.preset = "Ink".into();
        st.params = ink;
    }
    assert_eq!(select(&mut panel, b), "Sketched Art");
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "Ink", "newName": "Dark Ink"}));
    assert_eq!(select(&mut panel, a), "Dark Ink", "A's waiting settings follow the rename");
    assert_eq!(select(&mut panel, b), "Sketched Art");
    execute(&mut panel, "imageTrace.presets.delete", json!({"name": "Dark Ink"}));
    assert_eq!(select(&mut panel, a), "Custom", "A's waiting settings name a deleted preset");
    assert_eq!(shown(&panel).params["threshold"], json!(40), "and keep their settings");
}

#[test]
fn a_refused_preset_name_keeps_the_dialog_open() {
    let mut app = traced_app_with_ink(100);
    for name in ["A", "B"] {
        app.session.execute("imageTrace.presets.save", &json!({ "name": name })).unwrap();
    }
    trace_and_open(&mut app, "A");
    let mut panel = panel_over(app);
    assert_eq!(shown_preset(&panel), "A");
    panel.click(MANAGE);
    panel.click("Rename…");
    panel.type_into("Name", "b");
    assert_eq!(panel.value("Name").as_deref(), Some("b"), "the name entered stays, to be changed");
    panel.type_into("Name", "C");
    assert!(!panel.shows("Name"), "the accepted name closes the field");
    let names: Vec<String> = panel.app().session.prefs.trace_presets.iter().map(|p| p.name.clone()).collect();
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
    let mut panel = panel_over(app);
    assert_eq!(shown_preset(&panel), "Keep");
    // A, traced again with a built-in preset, is shown while Keep is deleted.
    panel.app().run("select.set", json!({"ids": [a.0]})).unwrap();
    panel.app().run("imageTrace.make", json!({"preset": "Sketched Art"})).unwrap();
    panel.frame();
    execute(&mut panel, "imageTrace.presets.delete", json!({"name": "Keep"}));
    assert_eq!(shown_preset(&panel), "Sketched Art");
    // B still records Keep.
    panel.app().run("select.set", json!({"ids": [b.0]})).unwrap();
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "Keep");
    panel.frame();
    assert_eq!(shown_preset(&panel), "Custom");
    // A built-in preset named in another case or in brackets shows under its own name, and Trace
    // with the settings unchanged keeps it.
    panel.app().run("imageTrace.make", json!({"preset": "[sketched art]"})).unwrap();
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "[sketched art]");
    panel.frame();
    assert_eq!(shown_preset(&panel), "Sketched Art");
    panel.click("Trace");
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "Sketched Art");
}

/// An object traced with a saved preset that was renamed since shows the new name when it is selected again, and
/// on a later frame.
#[test]
fn selecting_an_object_traced_with_a_renamed_preset_shows_the_new_name() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art"})).unwrap();
    trace_and_open(&mut app, "Ink");
    let mut panel = panel_over(app);
    assert_eq!(shown_preset(&panel), "Ink");
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "ink", "newName": "Dark Ink"}));
    assert_eq!(shown_preset(&panel), "Dark Ink");
    panel.app().run("select.set", json!({"ids": []})).unwrap();
    panel.frame();
    panel.app().run("select.all", json!({})).unwrap();
    panel.frame();
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "Ink", "the object still records the old name");
    assert_eq!(shown_preset(&panel), "Dark Ink");
    panel.frame();
    assert_eq!(shown_preset(&panel), "Dark Ink");
    // Dark Ink is deleted, and another preset with the same settings is made under that name: the object shows
    // Custom, also after that preset is renamed. Each is read from the panel opened anew, whose run-only state starts
    // over, as production read it from a new context.
    let reopened = |panel: &mut Panel| {
        panel.app().ui.image_trace_panel = Default::default();
        panel.frame();
        shown_preset(panel)
    };
    panel.app().session.execute("imageTrace.presets.delete", &json!({"name": "Dark Ink"})).unwrap();
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "Dark Ink", "preset": "Sketched Art"})).unwrap();
    assert_eq!(reopened(&mut panel), "Custom");
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "Dark Ink", "newName": "Other Ink"})).unwrap();
    assert_eq!(reopened(&mut panel), "Custom");
}

/// The Control bar's and Properties' Preset dropdowns show an object traced with a saved preset under the name the
/// panel shows: the new name after a rename, and Custom once the preset's settings changed or it was deleted.
#[test]
fn the_control_bar_and_properties_show_a_renamed_or_changed_preset_as_the_panel_does() {
    let bar: fn(&mut VectorcraftApp, &mut Ui) = |app, ui| {
        control_bar(app, ui);
    };
    let properties: fn(&mut VectorcraftApp, &mut Ui) = crate::panels::properties::show;
    for (host, draw) in [("Control bar", bar), ("Properties", properties)] {
        let mut app = traced_app_with_ink(100);
        app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art"})).unwrap();
        trace_with(&mut app, "Ink");
        let mut panel = Panel::new(app, draw);
        let shows = |panel: &mut Panel, change: Option<(&str, Value)>, name: &str| {
            if let Some((id, params)) = change {
                execute(panel, id, params);
            }
            assert_eq!(panel.value("Preset").as_deref(), Some(name), "{host}");
        };
        shows(&mut panel, None, "Ink");
        shows(&mut panel, Some(("imageTrace.presets.save", json!({"name": "Ink", "newName": "Dark Ink"}))), "Dark Ink");
        shows(&mut panel, Some(("imageTrace.presets.save", json!({"name": "Dark Ink", "preset": "Silhouettes"}))), "Custom");
        shows(&mut panel, Some(("imageTrace.presets.delete", json!({"name": "Dark Ink"}))), "Custom");
    }
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
    let panel = panel_over(app);
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "A");
    assert_eq!(shown_preset(&panel), "Custom");
}

/// An object traced with a preset that got its name from a rename, renamed again since: the panel shows the new name.
#[test]
fn an_object_traced_between_two_renames_shows_the_last_name() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "A", "preset": "Sketched Art"})).unwrap();
    app.session.execute("imageTrace.presets.save", &json!({"name": "A", "newName": "B"})).unwrap();
    trace_and_open(&mut app, "B");
    app.session.execute("imageTrace.presets.save", &json!({"name": "B", "newName": "C"})).unwrap();
    let mut panel = panel_over(app);
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "B");
    assert_eq!(shown_preset(&panel), "C");
    // Renamed back to B and on to D, it is still the same preset.
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "C", "newName": "B"})).unwrap();
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "B", "newName": "D"}));
    assert_eq!(shown_preset(&panel), "D");
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
    let mut panel = panel_over(app);
    assert_eq!(shown_preset(&panel), "A");
    panel.app().run("select.set", json!({"ids": []})).unwrap();
    panel.frame();
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "B", "newName": "C"})).unwrap();
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "A", "newName": "B"})).unwrap();
    assert_eq!(select(&mut panel, a), "B");
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
    let mut panel = panel_over(app);
    assert_eq!(shown_preset(&panel), "[Ink]");
    panel.app().session.execute("imageTrace.presets.delete", &json!({"name": "[Ink]"})).unwrap();
    panel.app().run("select.set", json!({"ids": []})).unwrap();
    panel.frame();
    panel.app().run("select.all", json!({})).unwrap();
    panel.frame();
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "[Ink]");
    assert_eq!(shown_preset(&panel), "Custom");
}

/// A saved preset whose settings change outside the panel while the panel shows it with other
/// settings: the panel shows Custom, and Trace records Custom with the panel's settings.
#[test]
fn a_saved_preset_changed_elsewhere_leaves_the_panel_settings_as_custom() {
    let mut app = traced_app_with_ink(100);
    app.session.execute("imageTrace.presets.save", &json!({"name": "Ink", "preset": "Sketched Art"})).unwrap();
    trace_and_open(&mut app, "Ink");
    let mut panel = panel_over(app);
    assert_eq!(shown_preset(&panel), "Ink");
    let before = shown(&panel).params;
    // A save with the same settings keeps the name.
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "Ink"}));
    assert_eq!(shown_preset(&panel), "Ink");
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "Ink", "params": {"threshold": 40}}));
    assert_eq!(shown_preset(&panel), "Custom");
    assert_eq!(shown(&panel).params, before, "the panel's settings stay");
    // A rename that also changes the settings.
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "Ink 2", "preset": "Sketched Art"}));
    assert_eq!(shown_preset(&panel), "Ink 2");
    execute(&mut panel, "imageTrace.presets.save", json!({"name": "Ink 2", "newName": "Ink 3", "params": {"threshold": 40}}));
    assert_eq!(shown_preset(&panel), "Custom");
    // The object, traced with Ink before its settings changed, shows Custom on a later frame.
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "Ink");
    panel.frame();
    assert_eq!(shown_preset(&panel), "Custom");
}

#[test]
fn the_preset_list_is_read_once_for_each_change_of_the_saved_presets() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Sketched Art");
    let mut panel = panel_over(app);
    let kept = |panel: &Panel| panel.app().canvas.trace_presets.as_ref().map(|(_, list)| list.clone()).unwrap();
    // The list after a change: read on the next frame, then kept by the frames after it.
    let settled = |panel: &mut Panel| {
        panel.frame();
        let list = kept(panel);
        panel.frame();
        panel.frame();
        assert!(Arc::ptr_eq(&list, &kept(panel)), "frames with the same saved presets read the list again");
        list
    };
    let start = settled(&mut panel);
    // A preset saved outside the panel is read once, and the panel shows it.
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "A", "preset": "Sketched Art"})).unwrap();
    let saved = settled(&mut panel);
    assert!(!Arc::ptr_eq(&saved, &start));
    assert_eq!(shown_preset(&panel), "A");
    // The rename's reply names the old name, and the panel follows the new one.
    let r = panel.app().session.execute("imageTrace.presets.save", &json!({"name": "a", "newName": "B"})).unwrap();
    assert_eq!(r["renamedFrom"], "A");
    let renamed = settled(&mut panel);
    assert!(!Arc::ptr_eq(&renamed, &saved));
    assert_eq!(shown_preset(&panel), "B");
    // Preferences that replace the saved presets are read again too.
    {
        let mut app = panel.app();
        let mut prefs = app.session.prefs.clone();
        prefs.trace_presets.clear();
        app.session.apply_prefs(prefs);
    }
    let replaced = settled(&mut panel);
    assert!(!Arc::ptr_eq(&replaced, &renamed));
    assert_eq!(shown_preset(&panel), "Custom");
    // A preset saved with the panel's settings and renamed before the next frame is shown under
    // its new name.
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "C", "preset": "Sketched Art"})).unwrap();
    panel.app().session.execute("imageTrace.presets.save", &json!({"name": "C", "newName": "D"})).unwrap();
    let both = settled(&mut panel);
    assert!(!Arc::ptr_eq(&both, &replaced));
    assert_eq!(shown_preset(&panel), "D");
}

#[test]
fn the_selected_preset_button_follows_the_preset_menu() {
    let mut app = traced_app_with_ink(100);
    trace_and_open(&mut app, "Default");
    let mut panel = panel_over(app);
    assert!(selected_preset_buttons(&panel).is_empty(), "Default has no button");
    choose(&mut panel, "Shades of Gray");
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "Shades of Gray");
    assert_eq!(selected_preset_buttons(&panel), ["Shades of Gray"]);
    choose(&mut panel, "6 Colors");
    assert_eq!(selected_trace(&panel.app()).unwrap().0, "6 Colors");
    assert!(selected_preset_buttons(&panel).is_empty(), "6 Colors has no button");
    // A changed setting makes the preset Custom, and no button shows as selected.
    panel.click(preset_label("Outline"));
    assert_eq!(selected_preset_buttons(&panel), ["Outline"]);
    panel.click("Preview");
    panel.click("Fills");
    assert_eq!(shown_preset(&panel), "Custom");
    assert!(selected_preset_buttons(&panel).is_empty());
}

#[test]
fn a_preset_button_with_no_image_selected_changes_the_panel_settings_as_the_preset_menu_does() {
    let blank = || {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app
    };
    let mut by_button = panel_over(blank());
    let steps = undo(&by_button.app());
    by_button.click("High Color");
    assert_eq!(undo(&by_button.app()), steps, "nothing is traced");
    assert!(find_target(&by_button.app()).stored.is_none(), "no Image Trace object");
    assert_eq!(selected_preset_buttons(&by_button), ["High Fidelity Photo"]);

    let mut by_menu = panel_over(blank());
    choose(&mut by_menu, "High Fidelity Photo");
    assert_eq!(undo(&by_menu.app()), steps);
    let (button, menu) = (shown(&by_button), shown(&by_menu));
    assert_eq!((button.preset, button.params), (menu.preset, menu.params));
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
    let steps = undo(&app);
    let mut panel = panel_over(app);
    panel.click("Preview");
    panel.click("Strokes");
    panel.click("Trace");
    assert_eq!(panel.app().trace_runs.running.len(), 1);
    assert_eq!(undo(&panel.app()), steps, "not traced yet");
    {
        let mut app = panel.app();
        let t0 = std::time::Instant::now();
        while !app.trace_runs.running.is_empty() && t0.elapsed() < std::time::Duration::from_secs(30) {
            crate::trace_runs::poll(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(undo(&app), steps + 1, "one undo step");
        assert_eq!(param(&app, "strokes"), Some(json!(true)));
        let st = &app.ui.image_trace_panel.shown;
        assert!(st.info.is_some(), "its counts show");
        assert_eq!(st.object, find_target(&app).object_key());
        assert!(st.drafts.is_empty(), "nothing waits for the object it replaced");
        app.ui.image_trace_preview = true;
    }
    panel.frame();
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.frame();
    panel.frame();
    assert!(panel.app().trace_runs.running.is_empty(), "Undo traces nothing");
    assert_eq!(param(&panel.app(), "strokes"), Some(json!(false)), "the object's own settings");
    assert_eq!(undo(&panel.app()), steps);
}

/// The object Trace acts on as the panel found it before [`find_target`]: every selected object put
/// in paint order, the first image or Image Trace object among them taken, else the first selected
/// object.
fn target_node_sorting_everything(app: &VectorcraftApp) -> Option<Node> {
    let st = app.session.active()?;
    let traceable = |n: &&Node| is_trace(n) || matches!(n.kind, NodeKind::Image(_));
    st.selection
        .in_paint_order(&st.doc)
        .into_iter()
        .find_map(|id| st.doc.node(id).filter(traceable).cloned())
        .or_else(|| first_selected(app).map(|n| (*n).clone()))
}

/// [`Target`] from [`target_node_sorting_everything`] and a scan of the whole selection for Image
/// Trace objects, as the panel read them on every frame before.
fn target_sorting_everything(app: &VectorcraftApp) -> Target {
    let document = app.session.active().map(|st| (st.uid, st.reverts));
    let any_trace = app.session.active().is_some_and(|st| st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(is_trace)));
    let Some(n) = target_node_sorting_everything(app) else { return Target { document, any_trace, ..Target::default() } };
    Target {
        document,
        object: app.session.active().map(|st| (st.uid, n.id)),
        is_trace: is_trace(&n),
        is_image: matches!(n.kind, NodeKind::Image(_)),
        stored: n.extras().trace.as_deref().cloned(),
        any_trace,
    }
}

/// One frame's lookups as the panel made them before [`target`]: the target, then the scan for
/// Image Trace objects, then the target again for its document and id.
fn frame_lookups_sorting_everything(app: &VectorcraftApp) -> Target {
    let mut t = target_sorting_everything(app);
    t.object = app.session.active().zip(target_node_sorting_everything(app)).map(|(st, n)| (st.uid, n.id));
    t
}

/// Select `ids` as they are, repeats and ids the document lacks included, as a new revision.
fn select_as_is(app: &mut VectorcraftApp, ids: &[NodeId]) {
    let st = app.session.active_mut().unwrap();
    st.selection.objects = ids.to_vec();
    st.revision += 1;
}

/// A traced document with every kind of object the lookup tells apart, by name: the Image Trace
/// object (`trace`) and its image (`trace image`), plain images at the back (`back`) and front
/// (`front`) of the layer, a rectangle (`rect`), an image (`deep`) in a group (`inner`) in a group
/// (`outer`), a clipping group named Image Trace (`clip`) with an image (`clipped`), a hidden image
/// (`hidden`), a locked one (`locked`), and an image (`other`) and an Image Trace object (`other
/// trace`) on a second layer.
fn lookup_fixture() -> (VectorcraftApp, std::collections::HashMap<&'static str, NodeId>) {
    let mut app = traced_app();
    let rect = app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let layer = doc.layers[0].id;
    let trace: Node = doc.layers[0].children().unwrap().iter().find(|n| is_trace(n)).map(|n| (**n).clone()).unwrap();
    let image: Node = (*trace.children().unwrap()[0]).clone();
    let mut ids = std::collections::HashMap::from([("trace", trace.id), ("trace image", image.id), ("rect", NodeId(rect))]);
    let back = doc.reid(&image);
    ids.insert("back", doc.insert(Some(layer), 0, back).unwrap());
    let front = doc.reid(&image);
    ids.insert("front", doc.insert(Some(layer), usize::MAX, front).unwrap());
    let deep = doc.reid(&image);
    ids.insert("deep", deep.id);
    let inner = Node::group(doc.alloc_id(), vec![Arc::new(deep)]);
    ids.insert("inner", inner.id);
    let outer = Node::group(doc.alloc_id(), vec![Arc::new(inner)]);
    ids.insert("outer", doc.insert(Some(layer), 1, outer).unwrap());
    let clipped = doc.reid(&image);
    ids.insert("clipped", clipped.id);
    let mut clip = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clipped)], clip: true });
    clip.name = Some("Image Trace".into());
    ids.insert("clip", doc.insert(Some(layer), 2, clip).unwrap());
    let mut hidden = doc.reid(&image);
    hidden.visible = false;
    ids.insert("hidden", doc.insert(Some(layer), 3, hidden).unwrap());
    let mut locked = doc.reid(&image);
    locked.locked = true;
    ids.insert("locked", doc.insert(Some(layer), 4, locked).unwrap());
    let second = doc.add_layer(Some("Second"));
    let other = doc.reid(&image);
    ids.insert("other", doc.insert(Some(second), usize::MAX, other).unwrap());
    let other_trace = doc.reid(&trace);
    ids.insert("other trace", doc.insert(Some(second), 0, other_trace).unwrap());
    st.revision += 1;
    (app, ids)
}

/// The panel's lookup gives what putting the whole selection in paint order gave, field for field,
/// for every kind of selection: images in either selection order, an Image Trace object with its
/// own image, a clipping group named Image Trace, nested groups, hidden and locked images, two
/// layers, repeated ids, ids the document lacks, and nothing selected. Selected ancestors with
/// their descendants have separate canonical-root expectations below. The result kept for a
/// revision follows each change of the selection, an edit, Undo and a switch of document.
#[test]
fn the_target_lookup_matches_sorting_the_whole_selection() {
    let (mut app, ids) = lookup_fixture();
    let stale = NodeId(u64::MAX);
    let cases: &[&[&str]] = &[
        &[],
        &["rect"],
        &["back", "front"],
        &["front", "back"],
        &["rect", "front"],
        &["trace", "trace image"],
        &["trace image", "trace"],
        &["rect", "trace image"],
        &["clip"],
        &["rect", "clipped"],
        &["outer"],
        &["inner", "rect"],
        &["deep", "back"],
        &["hidden", "locked"],
        &["locked", "rect"],
        &["other", "front"],
        &["other trace", "trace"],
        &["rect", "other trace", "other"],
        &["front", "front"],
        &["rect", "rect"],
        &["stale"],
        &["stale", "rect"],
        &["stale", "front"],
        &["rect", "stale"],
    ];
    let mut every: Vec<NodeId> = ids.values().copied().collect();
    every.sort();
    every.push(stale);
    let selections = cases.iter().map(|c| c.iter().map(|n| if *n == "stale" { stale } else { ids[n] }).collect::<Vec<_>>()).chain([every]);
    let mut objects = std::collections::HashSet::new();
    for sel in selections {
        select_as_is(&mut app, &sel);
        let expected = target_sorting_everything(&app);
        assert_eq!(frame_lookups_sorting_everything(&app), expected, "{sel:?}");
        assert_eq!(find_target(&app), expected, "{sel:?}");
        assert_eq!(*target(&mut app), expected, "looked up for the revision: {sel:?}");
        assert_eq!(*target(&mut app), expected, "kept for the revision: {sel:?}");
        objects.insert(expected.object.map(|(_, id)| id));
    }
    assert!(objects.len() >= 8, "the cases reach many different objects: {objects:?}");

    // An edit and Undo.
    app.run("select.set", json!({"ids": [ids["rect"].0, ids["trace"].0]})).unwrap();
    let before = target(&mut app);
    assert_eq!(before.object.map(|o| o.1), Some(ids["trace"]));
    app.run("imageTrace.make", json!({"preset": "Outline"})).unwrap();
    let traced = target(&mut app);
    assert_eq!(*traced, target_sorting_everything(&app));
    assert_ne!(traced.stored, before.stored, "the edit changed the stored settings");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(target(&mut app), before, "Undo");

    // Another document, then back once it is closed.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert_eq!(*target(&mut app), target_sorting_everything(&app));
    assert_eq!(target(&mut app).object, None, "nothing selected in the new document");
    app.run("file.close", json!({})).unwrap();
    assert_eq!(target(&mut app), before, "the first document again");

    // No document.
    let mut none = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert_eq!(find_target(&none), target_sorting_everything(&none));
    assert_eq!(*target(&mut none), Target::default());
}

/// Trace only accepts selected roots: a selected parent blocks its image descendant.
/// Missing implicit-selection ids are dropped, while explicit ids are validated.
#[test]
fn the_target_lookup_obeys_canonical_roots_and_explicit_id_validation() {
    let (mut app, ids) = lookup_fixture();
    for (parent, child) in [("clip", "clipped"), ("outer", "deep")] {
        select_as_is(&mut app, &[ids[parent], ids[child]]);
        let uid = app.session.active().unwrap().uid;
        let document = app.session.active().map(|st| (st.uid, st.reverts));
        let expected = Target { document, object: Some((uid, ids[parent])), ..Default::default() };
        assert_eq!(find_target(&app), expected, "the selected {parent} supplies metadata, not its pruned image");
        assert_eq!(*target(&mut app), expected);
        assert_eq!(*target(&mut app), expected, "the canonical result is also cached");
        assert!(vectorcraft_engine::cmd::image_trace_target(&app.session, &Value::Null).is_err());
        let document = app.session.active().unwrap().doc.clone();
        assert!(app.run("imageTrace.make", json!({})).is_err(), "Trace rejects that non-image root");
        assert_eq!(app.session.active().unwrap().doc, document);
        select_as_is(&mut app, &[ids[child]]);
        assert_eq!(vectorcraft_engine::cmd::image_trace_target(&app.session, &Value::Null).unwrap(), ids[child], "the image alone remains eligible");
    }
    let stale = NodeId(u64::MAX);
    select_as_is(&mut app, &[stale, ids["front"]]);
    assert_eq!(target(&mut app).object.map(|(_, id)| id), Some(ids["front"]), "implicit stale entries do not hide a valid image");
    assert_eq!(vectorcraft_engine::cmd::image_trace_target(&app.session, &Value::Null).unwrap(), ids["front"]);
    let explicit = json!({"ids": [stale.0, ids["front"].0]});
    let document = app.session.active().unwrap().doc.clone();
    assert!(vectorcraft_engine::cmd::image_trace_target(&app.session, &explicit).is_err(), "explicit missing ids are invalid");
    assert!(app.run("imageTrace.make", explicit).is_err());
    assert_eq!(app.session.active().unwrap().doc, document);
}

/// The panel looks the object Trace acts on up once for each revision of the document, however
/// many frames show it, and again once the revision moves.
#[test]
fn the_panel_looks_its_target_up_once_per_document_revision() {
    let mut app = traced_app();
    let trace_id = first_selected(&app).unwrap().id;
    let rect = app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [rect, trace_id.0]})).unwrap();
    let uid = app.session.active().unwrap().uid;
    let mut panel = panel_over(app);
    let kept = |panel: &Panel| panel.app().canvas.trace_target.as_ref().map(|(_, t)| t.clone()).unwrap();
    let revision = |panel: &Panel| panel.app().session.active().unwrap().revision;
    let (start, at) = (kept(&panel), revision(&panel));
    for _ in 0..3 {
        panel.frame();
    }
    assert_eq!(revision(&panel), at, "the frames change nothing");
    assert!(Arc::ptr_eq(&kept(&panel), &start), "the frames of one revision reuse its lookup");
    assert_eq!(start.object, Some((uid, trace_id)));

    panel.app().run("select.set", json!({"ids": [rect]})).unwrap();
    panel.frame();
    let moved = kept(&panel);
    assert!(!Arc::ptr_eq(&moved, &start), "a new selection is looked up");
    assert_eq!(moved.object, Some((uid, NodeId(rect))));
    panel.frame();
    assert!(Arc::ptr_eq(&kept(&panel), &moved), "once");
}
