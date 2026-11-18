use super::*;
use crate::panels::tests_appearance::{frame_raw, text_rect};
use vectorcraft_engine::Session;

fn app() -> (VectorcraftApp, Value) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({ "width": 500, "height": 400 })).unwrap();
    let id = app.run("text.create", json!({ "x": 10, "y": 10, "text": "Ab\nCd\nEf\nGh", "font": "Preflight UI Missing 917", "size": 12, "area": { "width": 100, "height": 20 } })).unwrap()["id"].clone();
    app.run("object.setProps", json!({ "ids": [id], "name": "Delivery headline" })).unwrap();
    (app, id)
}

fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    frame_raw(
        ctx,
        app,
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 1000.0))),
            events,
            ..Default::default()
        },
        show,
    )
}

fn text(rows: &[(String, egui::Rect)]) -> String {
    rows.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn preflight_panel_rechecks_edits_undo_and_document_switches() {
    let (mut app, id) = app();
    let ctx = egui::Context::default();
    let first = text(&frame(&ctx, &mut app, 1.0, vec![]));
    assert!(first.contains("Preflight UI Missing 917") && first.contains("Text past the frame"), "{first}");
    app.run("text.areaOptions", json!({ "ids": [id], "height": 150 })).unwrap();
    let fixed = text(&frame(&ctx, &mut app, 1.1, vec![]));
    assert!(!fixed.contains("Text past the frame"), "revision rechecks before the timer: {fixed}");
    app.run("edit.undo", json!({})).unwrap();
    assert!(text(&frame(&ctx, &mut app, 1.2, vec![])).contains("Text past the frame"));
    app.run("file.new", json!({ "width": 500, "height": 400 })).unwrap();
    let clean = text(&frame(&ctx, &mut app, 1.3, vec![]));
    assert!(!clean.contains("Preflight UI Missing 917") && !clean.contains("Needs attention"), "{clean}");
}

#[test]
fn clicking_a_preflight_finding_selects_and_zooms_to_its_artwork() {
    let (mut app, id) = app();
    app.run("select.set", json!({ "ids": [] })).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let ctx = egui::Context::default();
    let rows = frame(&ctx, &mut app, 1.0, vec![]);
    let at = text_rect(&rows, "Delivery headline\nText past the frame will be left out").center();
    let event = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut app, 1.1, vec![egui::Event::PointerMoved(at), event(true)]);
    frame(&ctx, &mut app, 1.2, vec![event(false)]);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id.as_u64().unwrap())]);
    assert!(app.view().unwrap().fitted);
    assert!(app.view().unwrap().zoom > 1.0);
}

#[test]
fn clicking_a_preflight_finding_on_a_locked_artboard_reports_the_problem() {
    let (mut app, id) = app();
    app.run("select.set", json!({ "ids": [] })).unwrap();
    app.run("artboard.setProps", json!({ "index": 0, "locked": true })).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let before_view = *app.view().unwrap();
    let before_doc = app.session.active().unwrap().doc.clone();
    app.ui.status.clear();
    let ctx = egui::Context::default();
    let rows = frame(&ctx, &mut app, 1.0, vec![]);
    let at = text_rect(&rows, "Delivery headline\nText past the frame will be left out").center();
    let event = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut app, 1.1, vec![egui::Event::PointerMoved(at), event(true)]);
    frame(&ctx, &mut app, 1.2, vec![event(false)]);
    assert!(app.ui.status.contains("could not be selected"), "{}", app.ui.status);
    assert!(app.session.active().unwrap().selection.objects.is_empty());
    assert_eq!(app.view().unwrap().zoom, before_view.zoom);
    assert_eq!(*app.session.active().unwrap().doc, *before_doc);
    app.run("artboard.setProps", json!({ "index": 0, "locked": false })).unwrap();
    locate(&mut app, &json!({ "ids": [id] }));
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
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let report = app.session.execute("document.preflight", &json!({})).unwrap();
    let finding = report["findings"].as_array().unwrap().iter().find(|f| f["code"] == "missingFont").unwrap();
    assert!(finding["ids"].as_array().unwrap().contains(&locked));
    locate(&mut app, finding);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(free.as_u64().unwrap())]);
    assert!(app.view().unwrap().fitted);
    assert!(app.view().unwrap().zoom > 1.0);
    assert!(app.ui.status.contains("could not be selected"), "{}", app.ui.status);
}

#[test]
fn preflight_panel_does_not_present_stale_results_during_a_drag() {
    let (mut app, _) = app();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, 1.0, vec![]);
    app.session.begin_interaction("Preflight test").unwrap();
    let shown = text(&frame(&ctx, &mut app, 1.1, vec![]));
    assert!(shown.contains("Preflight updates when the edit finishes."), "{shown}");
    assert!(!shown.contains("Passed") && !shown.contains("Needs attention"), "{shown}");
    app.session.cancel_interaction().unwrap();
}

#[test]
fn preflight_window_menu_and_empty_state_are_available() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("window.panel", json!({ "panel": ID })).unwrap();
    let ctx = egui::Context::default();
    let shown = text(&frame(&ctx, &mut app, 1.0, vec![]));
    assert!(shown.contains("Open a document to check it before delivery."), "{shown}");
    assert!(crate::state::all_panels().any(|(id, name)| id == ID && name == "Production Preflight"));
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
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, 1.0, vec![]);
    let rev = app.session.active().unwrap().revision;
    std::fs::remove_file(&path).unwrap();
    assert!(!text(&frame(&ctx, &mut app, 1.1, vec![])).contains("Linked file is missing"));
    assert!(text(&frame(&ctx, &mut app, 3.1, vec![])).contains("Linked file is missing"));
    assert_eq!(rev, app.session.active().unwrap().revision);
    std::fs::write(&path, &bytes).unwrap();
    let rows = frame(&ctx, &mut app, 3.2, vec![]);
    let at = text_rect(&rows, "Recheck").center();
    let event = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut app, 3.3, vec![egui::Event::PointerMoved(at), event(true)]);
    let after = text(&frame(&ctx, &mut app, 3.4, vec![event(false)]));
    assert!(!after.contains("Linked file is missing"), "{after}");
    std::fs::remove_file(&path).unwrap();
}

/// An optional visual QA artifact, never a checked-in snapshot or an asset shipped by the app.
#[test]
#[ignore = "set PREFLIGHT_PREVIEW_PATH to write an offscreen GPU render for visual review"]
fn preflight_panel_preview() {
    let (app, _) = app();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(340.0, 480.0))
        .with_accessibility_check(false)
        .with_context_setup(|ctx| {
            crate::theme::install_fonts(ctx);
            crate::theme::apply(ctx, crate::theme::Brightness::Dark);
        })
        .build_ui_state(
            |ui, app| {
                ui.heading("Production Preflight");
                ui.separator();
                show(app, ui);
            },
            app,
        );
    harness.run_steps(3);
    let path = std::env::var("PREFLIGHT_PREVIEW_PATH").expect("PREFLIGHT_PREVIEW_PATH");
    harness.render().unwrap().save(path).unwrap();
}
