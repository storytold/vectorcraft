//! Slices on the canvas and in the menus.

use egui::{Pos2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.session.paint.stroke = vectorcraft_color::Paint::None;
    app
}

/// One headless canvas frame on an 800 × 600 window; the texts it painted.
fn frame_texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::canvas::show(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_canvas_numbers_slices_until_they_are_hidden() {
    let mut app = app();
    let ctx = egui::Context::default();
    assert!(!frame_texts(&mut app, &ctx).iter().any(|t| t == "01"), "no slices, no numbers");
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap();
    app.run("object.slice.make", json!({})).unwrap();
    let texts = frame_texts(&mut app, &ctx);
    // The object slice and the four auto slices around it.
    for n in ["01", "02", "03", "04", "05"] {
        assert!(texts.iter().any(|t| t == n), "slice {n} numbered: {texts:?}");
    }
    let cached = app.canvas.slices.as_ref().map(|(_, s)| s.len());
    assert_eq!(cached, Some(5));
    // Show Slice Numbers off: outlines only.
    app.run("prefs.set", json!({"key": "showSliceNumbers", "value": false})).unwrap();
    assert!(!frame_texts(&mut app, &ctx).iter().any(|t| t == "01"));
    app.run("prefs.set", json!({"key": "showSliceNumbers", "value": true})).unwrap();
    app.run("view.slices.hide", json!({})).unwrap();
    assert!(!frame_texts(&mut app, &ctx).iter().any(|t| t == "01"), "hidden");
}

#[test]
fn slice_menu_items_follow_the_state() {
    let mut app = app();
    let en = |app: &VectorcraftApp, id: &str| crate::menus::enabled(app, id);
    assert!(!en(&app, "object.slice.make") && !en(&app, "object.slice.release") && !en(&app, "object.slice.deleteAll"));
    assert!(en(&app, "object.slice.fromGuides"));
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    assert!(en(&app, "object.slice.make") && !en(&app, "object.slice.release"));
    app.run("object.slice.make", json!({})).unwrap();
    for id in ["object.slice.release", "object.slice.duplicate", "object.slice.divide", "object.slice.options", "object.slice.deleteAll"] {
        assert!(en(&app, id), "{id}");
    }
    assert!(app.session.execute("object.slice.combine", &json!({})).is_err(), "one slice can't be combined");
    assert_eq!(crate::menus::checked(&app, "object.slice.clipToArtboard", &json!({})), Some(true));
    app.run("object.slice.clipToArtboard", json!({})).unwrap();
    assert_eq!(crate::menus::checked(&app, "object.slice.clipToArtboard", &json!({})), Some(false));
    assert_eq!(crate::menus::dynamic_label(&app, "view.slices.hide", "Hide Slices"), "Hide Slices");
    app.run("view.slices.hide", json!({})).unwrap();
    assert_eq!(crate::menus::dynamic_label(&app, "view.slices.hide", "Hide Slices"), "Show Slices");
    assert_eq!(crate::menus::checked(&app, "view.slices.lock", &json!({})), Some(false));
    app.run("view.slices.lock", json!({})).unwrap();
    assert_eq!(crate::menus::checked(&app, "view.slices.lock", &json!({})), Some(true));
}

#[test]
fn slice_tool_cursors_are_drawn_in_code() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        for c in [vectorcraft_tools::Cursor::Slice, vectorcraft_tools::Cursor::SliceSelect] {
            assert!(crate::cursors::paint(ui.painter(), c, egui::pos2(50.0, 50.0), 1.0), "{c:?}");
        }
    });
    out.textures_delta.clear();
    assert!(!out.shapes.is_empty());
}

#[test]
fn the_slice_tools_are_real_tools() {
    let mut app = app();
    for t in ["slice", "sliceSelection"] {
        app.run("tool.select", json!({"tool": t})).unwrap();
        assert_eq!(app.session.tool_id(), t);
    }
    // Double-clicking a slice opens Slice Options.
    let id = app.run("object.slice.create", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    let view = app.view_info();
    let ev = vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::DoubleClick, 20.0, 20.0);
    crate::canvas::dispatch(&mut app, &ev, view);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::slices::OPTIONS));
    assert_eq!(app.session.active().unwrap().selection.slices, vec![vectorcraft_doc::NodeId(id)]);
}
