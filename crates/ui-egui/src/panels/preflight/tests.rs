use super::*;
// PLAN: assumed `crate::tests::panel_harness::Panel` (object-panels-w8, op.A15)
use crate::tests::panel_harness::Panel;
use vectorcraft_engine::Session;

fn app() -> (VectorcraftApp, Value) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({ "width": 500, "height": 400 })).unwrap();
    let id = app.run("text.create", json!({ "x": 10, "y": 10, "text": "Ab\nCd\nEf\nGh", "font": "Preflight UI Missing 917", "size": 12, "area": { "width": 100, "height": 20 } })).unwrap()["id"].clone();
    app.run("object.setProps", json!({ "ids": [id], "name": "Delivery headline" })).unwrap();
    (app, id)
}

/// Cancelling a highlighted output must keep the custom threshold and the report made for it.
/// A deliberate choice still takes that output's default threshold.
#[test]
fn cancelling_output_choices_preserves_custom_ppi_and_the_diagnostic_report() {
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers};

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.type_into("Minimum PPI:", "240");
    let (doc, undo, key, report) = {
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert_eq!((app.ui.preflight_panel.options.output, app.ui.preflight_panel.options.min_ppi), (0, 240.0));
        assert_eq!(app.ui.preflight_panel.cache.report["minPpi"], 240.0);
        (st.doc.clone(), st.history.undo.len(), app.ui.preflight_panel.cache.key, app.ui.preflight_panel.cache.report.clone())
    };
    let output_name = if panel.shows("Output") { "Output" } else { "Output:" };
    panel.click_role(output_name, Role::COMBO_BOX);
    panel.key(Key::ArrowDown, Modifiers::NONE);
    panel.key(Key::ArrowDown, Modifiers::NONE);
    panel.key(Key::Escape, Modifiers::NONE);
    panel.frame();
    {
        let app = panel.app();
        assert_eq!((app.ui.preflight_panel.options.output, app.ui.preflight_panel.options.min_ppi), (0, 240.0));
        assert_eq!(app.ui.preflight_panel.cache.key, key);
        assert_eq!(app.ui.preflight_panel.cache.report, report);
        assert_eq!(*app.session.active().unwrap().doc, *doc);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    }
    panel.click_role(output_name, Role::COMBO_BOX);
    panel.key(Key::ArrowDown, Modifiers::NONE);
    panel.key(Key::ArrowDown, Modifiers::NONE);
    panel.key(Key::Enter, Modifiers::NONE);
    panel.frame();
    {
        let app = panel.app();
        assert_eq!((app.ui.preflight_panel.options.output, app.ui.preflight_panel.options.min_ppi), (2, 96.0));
        assert_eq!(app.ui.preflight_panel.cache.report["output"], "screen");
        assert_eq!(app.ui.preflight_panel.cache.report["minPpi"], 96.0);
    }
    panel.type_into("Minimum PPI:", "120");
    panel.click_role(output_name, Role::COMBO_BOX);
    panel.key(Key::ArrowUp, Modifiers::NONE);
    panel.key(Key::Escape, Modifiers::NONE);
    panel.frame();
    {
        let app = panel.app();
        assert_eq!((app.ui.preflight_panel.options.output, app.ui.preflight_panel.options.min_ppi), (2, 120.0));
    }
    panel.click_role(output_name, Role::COMBO_BOX);
    panel.key(Key::ArrowUp, Modifiers::NONE);
    panel.key(Key::Enter, Modifiers::NONE);
    panel.frame();
    {
        let app = panel.app();
        assert_eq!((app.ui.preflight_panel.options.output, app.ui.preflight_panel.options.min_ppi), (1, 300.0));
    }
    assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
}

/// The accessible name of the row of the first finding with `code`, as the list words it.
fn row(app: &mut VectorcraftApp, code: &str) -> String {
    let report = app.session.execute("document.preflight", &json!({})).unwrap();
    let finding = report["findings"].as_array().unwrap().iter().find(|f| f["code"] == code).unwrap();
    format!("{}, {}, Warning", finding["name"].as_str().unwrap(), reason(code))
}

/// Whole-app frames in a window wide enough for a canvas that zooms in on the art, where the canvas, once laid out,
/// takes the view request a zoom to the selection made.
fn app_frame(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.get_or_insert_with(crate::testing::ui_context).clone();
    for _ in 0..2 {
        let time = ctx.input(|i| i.time) + 0.05;
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2400.0, 1600.0)));
        crate::testing::app_frame(app, &ctx, egui::RawInput { screen_rect, time: Some(time), focused: true, ..Default::default() });
    }
}

fn big_canvas(app: &mut VectorcraftApp) {
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
}

#[test]
fn preflight_panel_rechecks_edits_undo_and_document_switches() {
    let (mut app, id) = app();
    let (font, overflow) = (row(&mut app, "missingFont"), row(&mut app, "overflow"));
    let mut panel = Panel::new(app, show);
    panel.frame();
    assert!(panel.shows(&font) && panel.shows(&overflow));
    panel.app().run("text.areaOptions", json!({ "ids": [id], "height": 150 })).unwrap();
    panel.frame();
    assert!(!panel.shows(&overflow), "revision rechecks before the timer");
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.frame();
    assert!(panel.shows(&overflow));
    panel.app().run("file.new", json!({ "width": 500, "height": 400 })).unwrap();
    panel.frame();
    assert!(!panel.shows(&font));
}

#[test]
fn clicking_a_preflight_finding_selects_and_zooms_to_its_artwork() {
    let (mut app, id) = app();
    app.run("select.set", json!({ "ids": [] })).unwrap();
    big_canvas(&mut app);
    let overflow = row(&mut app, "overflow");
    let mut panel = Panel::new(app, show);
    panel.frame();
    panel.click(&overflow);
    app_frame(&mut panel.app());
    let app = panel.app();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id.as_u64().unwrap())]);
    assert!(app.view().unwrap().fitted);
    assert!(app.view().unwrap().zoom > 1.0);
}

#[test]
fn clicking_a_preflight_finding_on_a_locked_artboard_reports_the_problem() {
    let (mut app, id) = app();
    app.run("select.set", json!({ "ids": [] })).unwrap();
    app.run("artboard.setProps", json!({ "index": 0, "locked": true })).unwrap();
    big_canvas(&mut app);
    let before_view = *app.view().unwrap();
    let before_doc = app.session.active().unwrap().doc.clone();
    app.ui.status.clear();
    let overflow = row(&mut app, "overflow");
    let mut panel = Panel::new(app, show);
    panel.frame();
    panel.click(&overflow);
    {
        let app = panel.app();
        assert!(app.ui.status.contains("could not be selected"), "{}", app.ui.status);
        assert!(app.session.active().unwrap().selection.objects.is_empty());
        assert_eq!(app.view().unwrap().zoom, before_view.zoom);
        assert_eq!(*app.session.active().unwrap().doc, *before_doc);
    }
    panel.app().run("artboard.setProps", json!({ "index": 0, "locked": false })).unwrap();
    locate(&mut panel.app(), &json!({ "ids": [id] }));
    app_frame(&mut panel.app());
    let app = panel.app();
    assert!(app.ui.status.is_empty(), "successful navigation clears its previous warning: {}", app.ui.status);
    assert!(app.view().unwrap().fitted);
}

#[test]
fn preflight_navigation_reports_partial_selection_and_zooms_to_selectable_artwork() {
    let (mut app, locked) = app();
    app.run("artboard.new", json!({ "x": 600, "y": 0, "width": 500, "height": 400 })).unwrap();
    let free = app.run("text.create", json!({ "x": 610, "y": 30, "text": "Free", "font": "Preflight UI Missing 917" })).unwrap()["id"].clone();
    app.run("select.set", json!({ "ids": [] })).unwrap();
    app.run("artboard.setProps", json!({ "index": 0, "locked": true })).unwrap();
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let report = app.session.execute("document.preflight", &json!({})).unwrap();
    let finding = report["findings"].as_array().unwrap().iter().find(|f| f["code"] == "missingFont").unwrap();
    assert!(finding["ids"].as_array().unwrap().contains(&locked));
    locate(&mut app, finding);
    app_frame(&mut app);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(free.as_u64().unwrap())]);
    assert!(app.view().unwrap().fitted);
    assert!(app.view().unwrap().zoom > 1.0);
    assert!(app.ui.status.contains("could not be selected"), "{}", app.ui.status);
}

#[test]
fn preflight_panel_does_not_present_stale_results_during_a_drag() {
    let (mut app, _) = app();
    let overflow = row(&mut app, "overflow");
    let mut panel = Panel::new(app, show);
    panel.frame();
    assert!(panel.shows(&overflow));
    panel.app().session.begin_interaction("Preflight test").unwrap();
    panel.frame();
    assert!(panel.shows("Preflight updates when the edit finishes."));
    assert!(!panel.shows(&overflow));
    panel.app().session.cancel_interaction().unwrap();
    panel.frame();
    assert!(panel.shows(&overflow));
}

#[test]
fn preflight_panel_rechecks_missing_files_on_its_timer_and_on_request() {
    let dir = vectorcraft_testkit::temp_dir("ui-preflight-timer");
    let path = dir.join("picture.png");
    let image = craft_image_codecs::Image::from_raw(2, 2, craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, vec![255; 16])
        .unwrap();
    let bytes = craft_image_codecs::encode(&image, craft_image_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({ "width": 500, "height": 400 })).unwrap();
    app.run("file.place", json!({ "path": path.to_string_lossy(), "at": [100, 100] })).unwrap();
    let mut panel = Panel::new(app, show);
    panel.frame();
    let rev = panel.app().session.active().unwrap().revision;
    std::fs::remove_file(&path).unwrap();
    let missing = row(&mut panel.app(), "missingLink");
    panel.advance(0.1);
    panel.frame();
    assert!(!panel.shows(&missing));
    // PLAN: assumed Panel::advance(&mut self, seconds: f64)
    panel.advance(2.0);
    panel.frame();
    assert!(panel.shows(&missing));
    assert_eq!(rev, panel.app().session.active().unwrap().revision);
    std::fs::write(&path, &bytes).unwrap();
    panel.click("Recheck");
    assert!(!panel.shows(&missing));
    std::fs::remove_file(&path).unwrap();
}
