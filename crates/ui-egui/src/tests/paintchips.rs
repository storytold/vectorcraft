//! The Fill/Stroke chips (Control bar, Properties), their swatch and mixer popovers, the Transform
//! link and the Recolor quick action (M3.51).

use egui::{Key, Modifiers, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::panel_harness::{Panel, ROOM};
use crate::{VectorcraftApp, chrome, panels};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

fn fill_hex(app: &VectorcraftApp) -> String {
    panels::current_paints(app).0.color().map(|c| c.to_hex()).unwrap_or_default()
}

/// The Red swatch's name in the swatch popover.
const RED: &str = "Red, #ed1c24";

/// A point of the panel's room that holds no control.
fn outside() -> Pos2 {
    Pos2::new(ROOM.x - 5.0, ROOM.y - 5.0)
}

#[test]
fn red_text_shows_a_red_toolbar_fill() {
    let mut app = app();
    app.run("text.create", json!({"x": 20, "y": 60, "text": "Red"})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(fill_hex(&app), "#ff0000", "the type's fill is what the Fill proxy shows");
}

#[test]
fn a_popover_tile_click_sets_the_fill() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.toggleActive", json!({"fill": false})).unwrap();
    let mut panel = Panel::new(app, chrome::control_bar);
    // The Control bar ranks its groups out of a panel's width; a window's width shows Fill and Stroke.
    panel.room(egui::vec2(2200.0, 200.0));
    panel.click("Fill");
    assert!(panel.app().session.fill_active, "the Fill chip brings its proxy forward");
    // A colour swatch is named by its name and value.
    panel.click(RED);
    assert_eq!(fill_hex(&panel.app()), "#ed1c24");
    assert_eq!(panel.app().session.doc().unwrap().history.undo.last().unwrap().label, "Fill Color", "through paint.setFill");
    // The Stroke chip: its proxy comes forward; Shift-click shows the mixer instead of swatches.
    panel.key(Key::Escape, Modifiers::NONE);
    panel.click_with("Stroke", Modifiers::SHIFT);
    assert!(!panel.app().session.fill_active);
    assert!(!panel.shows(RED), "no swatches in the mixer");
    assert!(panel.shows("Recent Colors"), "the Color panel's body");
}

/// The Properties panel has the Control bar's Fill and Stroke chips, with nothing selected too,
/// where they set up the next object drawn (Discord feedback).
#[test]
fn the_properties_panel_has_the_control_bars_fill_and_stroke() {
    let mut panel = Panel::new(app(), panels::properties::show);
    for name in ["Document", "Appearance", "Fill", "Stroke"] {
        assert!(panel.shows(name), "{name}");
    }
    assert!(panel.rect("Stroke").top() > panel.rect("Fill").bottom(), "Fill above Stroke");
    panel.click("Fill");
    panel.click(RED);
    assert_eq!(fill_hex(&panel.app()), "#ed1c24", "the next object's fill");
    panel.app().run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    assert_eq!(fill_hex(&panel.app()), "#ed1c24", "the new rectangle (selected) has it");
    // With the rectangle selected: the same chips, under the Transform section.
    panel.key(Key::Escape, Modifiers::NONE);
    panel.click("Fill");
    panel.click("Black, #000000");
    assert_eq!(fill_hex(&panel.app()), "#000000");
    assert_eq!(panel.app().session.doc().unwrap().history.undo.last().unwrap().label, "Fill Color");
}

#[test]
fn the_transform_link_opens_the_transform_panel() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let mut panel = Panel::new(app, panels::transform::link);
    assert!(!panel.shows("Scale Corners"));
    panel.click("Transform");
    assert!(panel.app().ui.transform_panel.popover, "the link opens the popover");
    for name in ["Scale Corners", "Scale Strokes & Effects", "Rotate"] {
        assert!(panel.shows(name), "the popover shows {name}");
    }
    // A click outside closes it.
    panel.click_at(outside());
    assert!(!panel.app().ui.transform_panel.popover);
    assert!(!panel.shows("Scale Corners"));
}
