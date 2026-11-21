//! Smart Pen in the interface: its submenu (Object › Plug-ins › Smart Pen), which selects the
//! Smart Pen Tool and opens the Smart Pen panel; and the panel: whether a click with the tool adds
//! a straight or a curved segment, and the point and path edits it shares with the tool. Everything
//! acts through `addon.pen.*` and the `addon.pathtools.*` anchor edits.

use craft::behavior::IconId;
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::addons::smart_pen;

use crate::VectorcraftApp;
use crate::menus::cp;
use crate::panels::empty_state;

/// The Smart Pen panel's id (`window.panel {panel: "smartPen"}`).
pub const ID: &str = "smartPen";

/// Object › Plug-ins › Smart Pen.
pub fn menu() -> Vec<Node> {
    vec![cp("Smart Pen Tool", "tool.select", json!({ "tool": smart_pen::ID })), cp("Smart Pen Panel", "window.panel", json!({ "panel": ID }))]
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
    let shown = app.session.tool_options_of(smart_pen::ID)["next"] == json!("curved");
    let mut curved = shown;
    let mut action: Option<(&str, Value)> = None;
    let segments = [Entry::new(false, tl!("Straight")).icon(IconId::new("dc-line")), Entry::new(true, tl!("Curved")).icon(IconId::new("spline"))];
    Section::fixed(tl!("Next Segment")).show(ui, |ui, _| {
        Segmented::new(tl!("Next Segment"), Bind::bound(&mut curved), &segments).show(ui);
    });
    if curved != shown {
        action = Some(("addon.pen.nextSegment", json!({ "curved": curved })));
    }
    if !selected {
        Text::new(tl!("Direct-select anchor points to edit them.")).shade(Shade::Muted).show(ui);
    }
    for (label, id, p) in edits() {
        if Button::new(label).enabled(selected).show(ui).clicks > 0 {
            action = Some((id, p));
        }
    }
    if let Some((id, p)) = action
        && let Err(e) = app.run(id, p)
    {
        app.status(e);
    }
}
