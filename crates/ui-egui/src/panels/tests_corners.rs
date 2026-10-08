//! The live rectangle's corner radius in the Control bar, the Properties panel and the Transform
//! panel, and the workspaces that show it (Essentials, Essentials Classic, Printing and Proofing).

use egui::{Event, Key, Modifiers, PointerButton};
use serde_json::json;

use super::tests_appearance::{frame_raw, run};
use super::*;
use crate::state::DockTab;
use vectorcraft_doc::NodeKind;

type Draw = fn(&mut VectorcraftApp, &mut Ui);

/// A selected 100 × 50 pt live rectangle with 7 pt corners.
fn rect_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    run(&mut app, "object.setLiveShape", json!({"radius": 7}));
    app
}

fn radius(app: &VectorcraftApp) -> f64 {
    let st = app.session.active().unwrap();
    let n = st.doc.node(st.selection.objects[0]).unwrap();
    let NodeKind::Path { live: Some(vectorcraft_doc::LiveShape::Rectangle { radii, .. }), .. } = &n.kind else { panic!("not a live rectangle") };
    radii[0]
}

/// Type `value` into the field showing "7 pt" in what `draw` shows, and press Enter.
fn set_radius(app: &mut VectorcraftApp, draw: Draw, value: &str) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += 1.0;
        frame_raw(&ctx, app, egui::RawInput { events, time: Some(time), ..Default::default() }, draw)
    };
    let shown = frame(app, vec![]);
    let at = shown.iter().find(|(s, _)| s == "7 pt").unwrap_or_else(|| panic!("no corner radius field in {shown:?}")).1.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(app, vec![Event::PointerMoved(at), button(true), button(false)]);
    frame(app, vec![]);
    frame(app, vec![Event::Text(value.into())]);
    let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(app, vec![enter]);
}

#[test]
fn every_surface_sets_the_corner_radius() {
    let surfaces: [(&str, Draw); 3] =
        [("Control bar", crate::chrome::control_bar), ("Properties", properties::transform_section), ("Transform panel", transform::show)];
    for (name, draw) in surfaces {
        let mut app = rect_app();
        assert_eq!(radius(&app), 7.0);
        set_radius(&mut app, draw, "12");
        assert_eq!(radius(&app), 12.0, "{name}");
    }
}

#[test]
fn the_three_workspaces_show_it() {
    let mut app = rect_app();
    for name in ["Essentials", "Essentials Classic", "Printing and Proofing"] {
        let w = crate::workspaces::find(&app.ui, name).unwrap();
        crate::workspaces::apply(&mut app.ui, &w);
        // The Control bar carries it; Essentials has no Control bar but opens on the Properties panel.
        let shown = app.ui.control_bar || (app.ui.dock && app.ui.dock_tab == DockTab::Properties);
        assert!(shown, "{name}: no corner radius control on screen");
    }
}

#[test]
fn the_control_bar_sets_the_corner_picked_with_the_white_arrow() {
    let mut app = rect_app();
    let id = app.session.active().unwrap().selection.objects[0].0;
    // The first top-right anchor of the rounded rectangle.
    run(&mut app, "select.anchors", json!({"id": id, "anchors": [[0, 1]], "mode": "set"}));
    set_radius(&mut app, crate::chrome::control_bar, "12");
    let st = app.session.active().unwrap();
    let NodeKind::Path { live: Some(vectorcraft_doc::LiveShape::Rectangle { radii, .. }), .. } = &st.doc.node(st.selection.objects[0]).unwrap().kind
    else {
        panic!("not a live rectangle")
    };
    assert_eq!(*radii, [7.0, 12.0, 7.0, 7.0]);
}
