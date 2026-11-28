use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::ZOOM;
use crate::addons::dialogs::tests as host;
use crate::tests::panel_harness::Panel as Harness;
use crate::{VectorcraftApp, menus};

/// An app with a window, where its dialogs open, and two rectangles without a stroke, both selected.
fn app() -> (VectorcraftApp, NodeId, NodeId) {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    let mut rect = |x, y, w, h| NodeId(app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap());
    let (a, b) = (rect(0, 0, 20, 10), rect(100, 100, 40, 40));
    app.run("select.all", json!({})).unwrap();
    (app, a, b)
}

fn bounds(app: &VectorcraftApp, id: NodeId) -> vectorcraft_geom::Rect {
    app.session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

/// Click the Arrange Tools item `label`.
fn click(app: &mut VectorcraftApp, label: &str) {
    let entries = menus::menu_entries(app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Arrange Tools"] && e.label == label).expect(label);
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &id, p);
}

/// Replace a numeric field's text and commit by moving focus, without confirming the dialog.
fn type_number(panel: &mut Harness, label: &str, text: &str) {
    panel.click(label);
    panel.key(egui::Key::A, egui::Modifiers::COMMAND);
    panel.type_text(text);
    panel.key(egui::Key::Tab, egui::Modifiers::NONE);
    panel.frame();
}

#[test]
fn real_grid_edits_reach_the_preview_and_confirmed_geometry() {
    let (app, a, b) = app();
    let mut panel = Harness::new(app, crate::tests::picks::dialogs);
    click(&mut panel.app(), "Arrange in Grid…");
    type_number(&mut panel, "Columns", "1");
    type_number(&mut panel, "V Spacing", "7");
    let id = host::dialog_id(&panel.app());
    assert_eq!(host::read(&panel.app(), id, |f| f.int("columns")), Some(1));
    let preview = {
        let app = panel.app();
        (bounds(&app, a), bounds(&app, b))
    };
    assert!((preview.0.center().x - preview.1.center().x).abs() < 1e-6);
    assert!((preview.1.y0 - preview.0.y1 - 7.0).abs() < 1e-6);
    panel.click("OK");
    panel.frame();
    let app = panel.app();
    assert_eq!((bounds(&app, a), bounds(&app, b)), preview);
    assert!(!app.any_dialog_open());
}

#[test]
fn real_size_keyboard_edits_keep_unticked_sides_unchanged() {
    let (app, a, b) = app();
    let mut panel = Harness::new(app, crate::tests::picks::dialogs);
    click(&mut panel.app(), "Set Size of Each…");
    panel.frame();
    panel.focus("Height");
    panel.key(egui::Key::Space, egui::Modifiers::NONE);
    // Width's number follows its checkbox in keyboard order; the number has no separate caption.
    panel.focus("Width");
    panel.key(egui::Key::Tab, egui::Modifiers::NONE);
    panel.key(egui::Key::A, egui::Modifiers::COMMAND);
    panel.type_text("30");
    panel.key(egui::Key::Tab, egui::Modifiers::NONE);
    panel.frame();
    let id = host::dialog_id(&panel.app());
    assert_eq!(host::read(&panel.app(), id, |f| (f.number("width"), f.bool("heightOn"))), (Some(30.0), Some(false)));
    panel.click("OK");
    panel.frame();
    let app = panel.app();
    let (first, second) = (bounds(&app, a), bounds(&app, b));
    assert_eq!((first.width(), first.height()), (30.0, 10.0));
    assert_eq!((second.width(), second.height()), (30.0, 40.0));
}

#[test]
fn real_opacity_edits_drive_validation_confirmation_and_cancel() {
    let (app, a, b) = app();
    let mut panel = Harness::new(app, crate::tests::picks::dialogs);
    click(&mut panel.app(), "Randomize Opacity…");
    type_number(&mut panel, "Minimum", "80");
    type_number(&mut panel, "Maximum", "20");
    assert!(!panel.enabled("OK"), "a minimum above the maximum disables confirmation");
    type_number(&mut panel, "Maximum", "80");
    assert!(panel.enabled("OK"));
    panel.click("OK");
    panel.frame();
    let opacity = |panel: &Harness, id| panel.app().session.doc().unwrap().doc.node(id).unwrap().opacity;
    assert!((opacity(&panel, a) - 0.8).abs() < 1e-6 && (opacity(&panel, b) - 0.8).abs() < 1e-6);
    click(&mut panel.app(), "Randomize Opacity…");
    type_number(&mut panel, "Minimum", "25");
    type_number(&mut panel, "Maximum", "25");
    assert!((opacity(&panel, a) - 0.25).abs() < 1e-6);
    panel.click("Cancel");
    panel.frame();
    assert!((opacity(&panel, a) - 0.8).abs() < 1e-6 && (opacity(&panel, b) - 0.8).abs() < 1e-6);
}

#[test]
fn set_size_of_each_sets_the_sides_ticked() {
    let (mut app, a, b) = app();
    click(&mut app, "Set Size of Each…");
    let id = host::dialog_id(&app);
    let sides = host::read(&app, id, |f| (f.number("width"), f.number("height")));
    assert_eq!(sides, (Some(20.0), Some(10.0)), "the first object's size");
    host::set(&app, id, "width", json!(30));
    host::set(&app, id, "heightOn", json!(false));
    assert!(host::confirm(&app, id));
    host::frame(&mut app);
    assert_eq!((bounds(&app, a).width(), bounds(&app, a).height()), (30.0, 10.0));
    assert_eq!((bounds(&app, b).width(), bounds(&app, b).height()), (30.0, 40.0));
}

#[test]
fn replace_opens_with_the_scale_strokes_preference_and_swap_runs_directly() {
    let (mut app, a, b) = app();
    app.run("prefs.set", json!({"key": "scaleStrokes", "value": true})).unwrap();
    click(&mut app, "Replace with Top Object…");
    let id = host::dialog_id(&app);
    let opened = host::read(&app, id, |f| (f.bool("scaleStrokes"), f.text("fit").map(str::to_owned)));
    assert_eq!(opened, (Some(true), Some("keep".to_owned())));
    host::cancel(&app, id);
    host::frame(&mut app);
    click(&mut app, "Swap Objects");
    assert!(!app.any_dialog_open());
    assert_eq!(
        (bounds(&app, a).center(), bounds(&app, b).center()),
        (vectorcraft_geom::Point::new(120.0, 120.0), vectorcraft_geom::Point::new(10.0, 5.0))
    );
}

#[test]
fn zoom_to_selection_fits_the_selection_in_the_view() {
    let (mut app, _, b) = app();
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 460.0));
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    assert!(menus::enabled(&app, ZOOM));
    // The view fits at once, in the canvas it was laid out in.
    app.run(ZOOM, json!({})).unwrap();
    let (canvas, view) = (app.view().unwrap().rect, app.canvas_view().unwrap());
    let selected = bounds(&app, b);
    let (min, max) = (
        view.to_screen(vectorcraft_geom::Point::new(selected.x0, selected.y0)),
        view.to_screen(vectorcraft_geom::Point::new(selected.x1, selected.y1)),
    );
    assert!(canvas.contains(min) && canvas.contains(max), "the selection lies in the canvas");
    assert!((view.to_screen(selected.center()) - canvas.center()).length() < 1.0, "and is centred");
    app.run("select.none", json!({})).unwrap();
    assert!(!menus::enabled(&app, ZOOM));
    assert!(app.run(ZOOM, json!({})).is_err());
}
