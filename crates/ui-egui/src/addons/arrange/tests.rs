use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::{GRID, OPACITY, REPLACE, SIZE, SORT, ZOOM};
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// An app with two rectangles without a stroke, both selected.
fn app() -> (VectorcraftApp, NodeId, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
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
fn the_plugins_submenu_lists_the_tools() {
    let (app, ..) = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Arrange Tools"] && e.label == label);
    for (label, id) in [
        ("Arrange in Grid…", "addon.arrange.grid"),
        ("Swap Objects", "addon.arrange.swap"),
        ("Swap Objects and Sizes", "addon.arrange.swap"),
        ("Replace with Top Object…", "addon.arrange.replace"),
        ("Set Size of Each…", "addon.arrange.size"),
        ("Group Overlapping Objects", "addon.arrange.groupOverlapping"),
        ("Randomize Opacity…", "addon.arrange.randomizeOpacity"),
        ("Sort Objects…", "addon.arrange.sort"),
        ("Zoom to Selection", ZOOM),
    ] {
        assert_eq!(under(label).and_then(|e| e.command.as_deref()), Some(id), "{label}");
    }
    assert!(entries.iter().any(|e| e.path == ["Object", "Plug-ins"] && e.label == "Install Plug-in…"), "the rest follows");
}

#[test]
fn arrange_in_grid_opens_its_dialog_and_runs_on_ok() {
    let (mut app, a, b) = app();
    click(&mut app, "Arrange in Grid…");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(GRID));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Arrange in Grid", "Layout:", "Columns:", "H Spacing:", "V Spacing:", "Center on Artboard", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["columns"], json!(2), "√2, rounded up");
    d.fields.insert("layout".into(), json!("row"));
    let n = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
    assert_eq!((bounds(&app, a).x0, bounds(&app, b).x0, bounds(&app, b).y0), (0.0, 30.0, 0.0));
}

#[test]
fn arrange_in_grid_previews_and_cancel_rolls_back() {
    let (mut app, _, b) = app();
    click(&mut app, "Arrange in Grid…");
    app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(bounds(&app, b).x0, 30.0, "previewed");
    crate::dialogs::cancel(&mut app);
    assert_eq!(bounds(&app, b).x0, 100.0, "rolled back");
}

#[test]
fn set_size_of_each_sets_the_sides_ticked() {
    let (mut app, a, b) = app();
    click(&mut app, "Set Size of Each…");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(SIZE));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Set Size of Each", "Width:", "Height:", "Constrain Width and Height Proportions", "Reference Point", "Scale Strokes & Effects"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!((&d.fields["width"], &d.fields["height"]), (&json!(20.0), &json!(10.0)), "the first object's size");
    d.fields.insert("width".into(), json!(30));
    d.fields.insert("__height".into(), json!(false));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((bounds(&app, a).width(), bounds(&app, a).height()), (30.0, 10.0));
    assert_eq!((bounds(&app, b).width(), bounds(&app, b).height()), (30.0, 40.0));
}

#[test]
fn replace_opens_with_the_scale_strokes_preference_and_swap_runs_directly() {
    let (mut app, a, b) = app();
    app.run("prefs.set", json!({"key": "scaleStrokes", "value": true})).unwrap();
    click(&mut app, "Replace with Top Object…");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), &d.fields["scaleStrokes"], &d.fields["fit"]), (REPLACE, &json!(true), &json!("keep")));
    crate::dialogs::cancel(&mut app);
    click(&mut app, "Swap Objects");
    assert!(app.ui.dialog.is_none());
    assert_eq!(
        (bounds(&app, a).center(), bounds(&app, b).center()),
        (vectorcraft_geom::Point::new(120.0, 120.0), vectorcraft_geom::Point::new(10.0, 5.0))
    );
}

#[test]
fn zoom_to_selection_fits_the_selection_in_the_view() {
    let (mut app, _, b) = app();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 460.0)));
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    assert!(menus::enabled(&app, ZOOM));
    app.run(ZOOM, json!({})).unwrap();
    let v = app.view().unwrap();
    // 40 × 40 into 500 × 400 (the window less the margin).
    assert_eq!((v.center, v.zoom), (vectorcraft_geom::Point::new(120.0, 120.0), 10.0));
    app.run("select.none", json!({})).unwrap();
    assert!(!menus::enabled(&app, ZOOM));
    assert!(app.run(ZOOM, json!({})).is_err());
}

#[test]
fn group_overlapping_objects_runs_directly() {
    let (mut app, a, b) = app();
    let c = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 5, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.all", json!({})).unwrap();
    click(&mut app, "Group Overlapping Objects");
    assert!(app.ui.dialog.is_none());
    let doc = &app.session.doc().unwrap().doc;
    let g = doc.parent_of(a).unwrap();
    assert_eq!(doc.parent_of(c), Some(g), "a and c overlap: grouped");
    assert_ne!(doc.parent_of(b), Some(g), "b is left alone");
}

#[test]
fn randomize_opacity_opens_its_dialog_and_runs_on_ok() {
    let (mut app, a, b) = app();
    click(&mut app, "Randomize Opacity…");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(OPACITY));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Randomize Opacity", "Minimum:", "Maximum:", "Seed:", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!((&d.fields["min"], &d.fields["max"]), (&json!(30), &json!(100)));
    // Minimum above maximum: OK is off.
    d.fields.insert("min".into(), json!(80));
    d.fields.insert("max".into(), json!(20));
    assert!(!(crate::addons::arrange::SPEC.ok_enabled.unwrap())(&app, app.ui.dialog.as_ref().unwrap()));
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("min".into(), json!(40));
    d.fields.insert("max".into(), json!(60));
    let n = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
    let doc = &app.session.doc().unwrap().doc;
    for id in [a, b] {
        assert!((0.4..=0.6).contains(&doc.node(id).unwrap().opacity), "{id:?}");
    }
}

#[test]
fn sort_objects_opens_its_dialog_and_runs_on_ok() {
    let (mut app, a, b) = app();
    click(&mut app, "Sort Objects…");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(SORT));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Sort Objects", "Sort By:", "Apply To:", "Order:", "Area", "Stacking Order", "Small to Large", "Distribute Evenly", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!((&d.fields["by"], &d.fields["apply"], &d.fields["order"]), (&json!("area"), &json!("stacking"), &json!("ascending")));
    // Large to small along x: b (the larger) takes the left edge 0, a the left edge 100.
    d.fields.insert("apply".into(), json!("x"));
    d.fields.insert("order".into(), json!("descending"));
    let n = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
    assert_eq!((bounds(&app, b).x0, bounds(&app, a).x0), (0.0, 100.0));
}
