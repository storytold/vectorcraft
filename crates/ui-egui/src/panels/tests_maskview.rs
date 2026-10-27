//! Headless frames of the Transparency panel's mask thumbnail (M3.86): Alt-click views only the
//! mask (and edits it), Alt-click again shows the artwork.

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
