//! Headless frames of panel rows that the framework lays out: controls side by side stay inside the panel's width.

use super::tests_appearance::app_with_rect;
use super::*;
use crate::panel_docking::PANEL_MIN_WIDTH;
use crate::tests::panel_harness::{Panel, ROOM};

#[test]
fn the_arrowhead_row_stays_inside_the_stroke_panel() {
    let panel = Panel::new(app_with_rect(), stroke::show);
    // The Arrowheads field spans the panel's width.
    let row = panel.rect("Arrowheads");
    for name in ["Start arrowhead", "End arrowhead", "Swap start and end arrowheads"] {
        assert!(row.contains_rect(panel.rect(name)), "{name} at {:?} is outside the row {row:?}", panel.rect(name));
    }
}

#[test]
fn the_color_themes_hex_row_stays_inside_a_narrow_panel() {
    let mut panel = Panel::new(app_with_rect(), color_themes::show);
    panel.room(egui::vec2(PANEL_MIN_WIDTH, ROOM.y)).frame();
    // The tab strip spans the panel's width.
    let width = panel.rect("Color Themes");
    for name in ["Color", "Set the current fill or stroke to the selected color", "Set the selected color to the current color"] {
        let at = panel.rect(name);
        assert!(at.left() >= width.left() && at.right() <= width.right(), "{name} at {at:?} is outside the panel {width:?}");
    }
}

#[test]
fn the_color_wheel_box_and_its_strip_stay_inside_the_panel() {
    let mut app = app_with_rect();
    app.ui.color_wheel.view = color_wheel::View::Box;
    let panel = Panel::new(app, color_wheel::show);
    let (field, strip) = (panel.rect("Color box"), panel.rect("Hue"));
    assert!(strip.height() <= field.height(), "the strip {strip:?} is taller than the box {field:?}");
    assert!(field.right() <= ROOM.x && strip.right() <= ROOM.x, "the box {field:?} or its strip {strip:?} runs past the panel");
}
