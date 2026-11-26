use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::ZOOM;
use crate::addons::dialogs::tests as host;
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
