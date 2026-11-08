//! Headless frames of the Transparency panel's mask thumbnail (M3.86): Alt-click views only the
//! mask (and edits it), Alt-click again shows the artwork. In isolation mode neither a click nor an
//! Alt-click edits the mask.

use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

/// One frame of the Transparency panel with `events`, `modifiers` held.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| transparency::show(app, ui));
    out.textures_delta.clear();
}

/// Click the mask thumbnail with `modifiers` held.
fn click_mask(ctx: &egui::Context, app: &mut VectorcraftApp, modifiers: egui::Modifiers) {
    frame(ctx, app, vec![], egui::Modifiers::NONE);
    let at = ctx.read_response(egui::Id::unique(transparency::MASK_THUMB)).expect("the mask thumbnail is drawn").rect.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    frame(ctx, app, vec![egui::Event::PointerMoved(at), button(true), button(false)], modifiers);
    frame(ctx, app, vec![], egui::Modifiers::NONE);
}

/// In pattern editing mode a click or an Alt-click on the mask thumbnail of a masked object in the
/// tile does not edit the mask, and the status bar says to finish editing the pattern first. Make
/// Mask is off for one object, which it would take into mask editing. After Done a click edits the
/// mask.
#[test]
fn the_mask_thumbnail_does_not_edit_the_mask_in_pattern_editing_mode() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let b = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "transparency.makeOpacityMask", json!({}));
    run(&mut app, "select.set", json!({"ids": [a]}));
    run(&mut app, "object.pattern.make", json!({"name": "P"}));
    let tile = app.session.active().unwrap().isolation;
    let ctx = egui::Context::default();
    for modifiers in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
        app.ui.status.clear();
        click_mask(&ctx, &mut app, modifiers);
        let st = app.session.active().unwrap();
        assert!(st.doc.mask_edit.is_none() && st.shown_mask().is_none(), "{modifiers:?}");
        assert!(st.doc.pattern_edit.is_some() && st.isolation == tile, "{modifiers:?}");
        assert!(app.ui.status.ends_with("finish editing the pattern first"), "{modifiers:?}: {}", app.ui.status);
    }
    run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 30, "height": 30}));
    assert!(!crate::menus::enabled(&app, "transparency.makeOpacityMask"), "Make Mask is off for one object");
    run(&mut app, "object.pattern.done", json!({}));
    run(&mut app, "select.set", json!({"ids": [a]}));
    click_mask(&ctx, &mut app, egui::Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().doc.mask_edit.map(|e| e.object.0), Some(a));
}

#[test]
fn alt_clicking_the_mask_thumbnail_views_only_the_mask() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let b = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "transparency.makeOpacityMask", json!({}));
    let ctx = egui::Context::default();
    let viewed = |app: &VectorcraftApp| app.session.active().unwrap().shown_mask().map(|n| n.0);
    click_mask(&ctx, &mut app, egui::Modifiers::ALT);
    assert_eq!(viewed(&app), Some(a));
    assert!(app.session.active().unwrap().doc.mask_edit.is_some(), "viewing the mask edits it");
    click_mask(&ctx, &mut app, egui::Modifiers::ALT);
    assert_eq!(viewed(&app), None);
    assert!(app.session.active().unwrap().doc.mask_edit.is_some(), "still editing");
    // A plain click while editing does nothing; Alt views again.
    click_mask(&ctx, &mut app, egui::Modifiers::NONE);
    assert_eq!(viewed(&app), None);
    click_mask(&ctx, &mut app, egui::Modifiers::ALT);
    assert_eq!(viewed(&app), Some(a));
}

/// Where the panel painted `label`, in a frame of it.
fn label_at(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) -> egui::Pos2 {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| transparency::show(app, ui));
    out.textures_delta.clear();
    let at = out.shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
        _ => None,
    });
    at.unwrap_or_else(|| panic!("the panel draws {label:?}"))
}

/// In isolation mode a click or an Alt-click on the mask thumbnail does not edit the mask, and the
/// status bar says to exit isolation mode first. Make Mask is off for one object, which it would
/// take into mask editing. After Exit Isolation Mode a click edits the mask.
#[test]
fn the_mask_thumbnail_does_not_edit_the_mask_in_isolation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let b = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "transparency.makeOpacityMask", json!({}));
    let c = run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, c]}));
    let g = run(&mut app, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut app, "object.isolate", json!({"id": g}));
    run(&mut app, "select.set", json!({"ids": [a]}));
    let ctx = egui::Context::default();
    for modifiers in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
        app.ui.status.clear();
        click_mask(&ctx, &mut app, modifiers);
        let st = app.session.active().unwrap();
        assert!(st.doc.mask_edit.is_none() && st.shown_mask().is_none(), "{modifiers:?}");
        assert_eq!(st.isolation.map(|i| i.0), Some(g));
        assert!(app.ui.status.ends_with("exit isolation mode first"), "{modifiers:?}: {}", app.ui.status);
    }
    // One object without a mask: Make Mask is off, and a click on it does nothing.
    run(&mut app, "select.set", json!({"ids": [c]}));
    app.ui.status.clear();
    frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
    let at = label_at(&ctx, &mut app, "Make Mask");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at), button(true), button(false)], egui::Modifiers::NONE);
    let st = app.session.active().unwrap();
    let id = vectorcraft_doc::NodeId(c.as_u64().unwrap());
    assert!(st.doc.node(id).unwrap().extras().mask.is_none() && st.doc.mask_edit.is_none());
    assert_eq!(app.ui.status, "", "the button is off");
    run(&mut app, "object.exitIsolation", json!({}));
    run(&mut app, "select.set", json!({"ids": [a]}));
    click_mask(&ctx, &mut app, egui::Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().doc.mask_edit.map(|e| e.object.0), Some(a));
}
