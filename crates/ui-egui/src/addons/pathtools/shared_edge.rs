//! The Anchor Inspector panel's Shared Edges section: the Shared Edge Merge Tool's brush and
//! coincidence, and Keep Shared Points Together.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::addons::shared_edge::{COINCIDENCE, DIAMETER, DIAMETER_RANGE, ID, MAX_COINCIDENCE};

use super::run;
use crate::VectorcraftApp;
use crate::widgets;

/// The menu's command: the tool, with the panel its values are in.
pub const COMMAND: &str = "addon.pathtools.sharedEdgeTool";
const FIELD_W: f32 = 96.0;
/// Wide enough for "Coincidence:".
const LABEL_W: f32 = 80.0;

/// Object › Plug-ins › Path Precision Tools › Shared Edge Merge Tool.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.select_tool(ID);
    app.run("window.panel", json!({ "panel": super::ID }))
}

pub(super) fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let options = app.session.tool_options_of(ID);
    let diameter = options["diameter"].as_f64().unwrap_or(DIAMETER);
    let tolerance = options["tolerance"].as_f64().unwrap_or(COINCIDENCE);
    let together = options["together"].as_bool().unwrap_or(false);
    widgets::subheader(ui, "Shared Edges");
    let mut set: Option<Value> = None;
    widgets::label_row(ui, tl!("Diameter:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, "shared-edge-diameter", diameter, DIAMETER_RANGE.0..=DIAMETER_RANGE.1, " pt", 2, FIELD_W) {
            set = Some(json!({ "diameter": v }));
        }
    });
    widgets::label_row(ui, tl!("Coincidence:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, "shared-edge-tolerance", tolerance, 0.0..=MAX_COINCIDENCE, " pt", 3, FIELD_W) {
            set = Some(json!({ "tolerance": v }));
        }
    });
    let toggled = widgets::check(ui, "Keep Shared Points Together", together, true);
    if let Some(values) = set {
        run(app, "tool.setOption", json!({ "tool": ID, "values": values }));
    }
    if toggled {
        run(app, "addon.pathtools.movePointsTogether", json!({ "enabled": !together }));
    }
}
