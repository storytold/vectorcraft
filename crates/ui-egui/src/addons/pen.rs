//! Smart Pen in the interface: its submenu (Object › Plug-ins › Smart Pen), which selects the
//! Smart Pen Tool and opens the Smart Pen panel; and the panel: whether a click with the tool adds
//! a straight or a curved segment, and the point and path edits it shares with the tool. Everything
//! acts through `addon.pen.*` and the `addon.pathtools.*` anchor edits.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::addons::smart_pen;

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::panels::empty_state;
use crate::widgets::{self, dim_label};

/// The Smart Pen panel's id (`window.panel {panel: "smartPen"}`).
pub const ID: &str = "smartPen";

/// Object › Plug-ins › Smart Pen.
pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd("Smart Pen Tool", "tool.select", json!({ "tool": smart_pen::ID })),
        Item::Cmd("Smart Pen Panel", "window.panel", json!({ "panel": ID })),
    ]
}

/// The panel's point and path edits: (label, command, params).
fn edits() -> [(&'static str, &'static str, Value); 5] {
    [
        (tl!("Remove Points, Keep Shape"), "addon.pathtools.removeKeepShape", json!({})),
        (tl!("Make Smooth"), "addon.pathtools.smooth", json!({})),
        // Nothing moves: the handles stay where they are.
        (tl!("Make Corner"), "addon.pathtools.corner", json!({ "retract": false })),
        (tl!("Retract Handles"), "addon.pathtools.corner", json!({ "retract": true })),
        (tl!("Close or Join"), "addon.pen.closeJoin", json!({})),
    ]
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        empty_state(ui, "spline-pointer", tl!("No document"), tl!("Open a document to draw with the Smart Pen."));
        return;
    };
    let selected = !st.selection.is_empty();
    let curved = app.session.tool_options_of(smart_pen::ID)["next"] == json!("curved");
    let mut action: Option<(&str, Value)> = None;
    // The label above its buttons, so a long translation stays within the dock's width.
    widgets::subheader(ui, tl!("Next Segment"));
    ui.horizontal(|ui| {
        for (icon, tip, on) in [("dc-line", tl!("Straight"), false), ("spline", tl!("Curved"), true)] {
            if widgets::icon_button(ui, icon, tip, curved == on, 24.0).clicked() {
                action = Some(("addon.pen.nextSegment", json!({ "curved": on })));
            }
        }
    });
    widgets::divider(ui);
    if !selected {
        dim_label(ui, tl!("Direct-select anchor points to edit them."));
    }
    let width = ui.available_width();
    ui.add_enabled_ui(selected, |ui| {
        for (label, id, p) in edits() {
            if widgets::flat_button(ui, label, width).clicked() {
                action = Some((id, p));
            }
        }
    });
    if let Some((id, p)) = action
        && let Err(e) = app.run(id, p)
    {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
