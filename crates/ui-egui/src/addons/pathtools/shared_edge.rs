//! The Anchor Inspector panel's Shared Edges section: the Shared Edge Merge Tool's brush and
//! coincidence, and Keep Shared Points Together.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::display::Text;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::addons::shared_edge::{COINCIDENCE, DIAMETER, DIAMETER_RANGE, ID, MAX_COINCIDENCE};

use super::{row, run, typed};
use crate::VectorcraftApp;

/// The menu's command: the tool, with the panel its values are in.
pub const COMMAND: &str = "addon.pathtools.sharedEdgeTool";

/// Object › Plug-ins › Path Precision Tools › Shared Edge Merge Tool.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.try_select_tool(ID)?;
    crate::panel_docking::reveal_panel(app, super::ID)
}

pub(super) fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let options = app.session.tool_options_of(ID);
    let diameter = options["diameter"].as_f64().unwrap_or(DIAMETER);
    let tolerance = options["tolerance"].as_f64().unwrap_or(COINCIDENCE);
    let together = options["together"].as_bool().unwrap_or(false);
    Text::new(tl!("Shared Edges")).show(ui);
    let mut set: Option<Value> = None;
    let diameter_field = |ui: &mut Ui| {
        let field = NumberField::new(tl!("Diameter"), Bind::given(Some(diameter))).range(DIAMETER_RANGE.0..=DIAMETER_RANGE.1);
        typed(field.unit_kind(UnitKind::Points).fixed_decimals(2).show(ui).value())
    };
    if let Some(v) = row(ui, "Diameter", "shared-edge-diameter", diameter_field) {
        set = Some(json!({ "diameter": v }));
    }
    let coincidence_field = |ui: &mut Ui| {
        let field = NumberField::new(tl!("Coincidence"), Bind::given(Some(tolerance))).range(0.0..=MAX_COINCIDENCE);
        typed(field.unit_kind(UnitKind::Points).fixed_decimals(3).show(ui).value())
    };
    if let Some(v) = row(ui, "Coincidence", "shared-edge-tolerance", coincidence_field) {
        set = Some(json!({ "tolerance": v }));
    }
    let toggled = Checkbox::new(Bind::given(together)).label(tl!("Keep Shared Points Together")).show(ui).value().copied();
    if let Some(values) = set {
        run(app, "tool.setOption", json!({ "tool": ID, "values": values }));
    }
    if let Some(enabled) = toggled {
        run(app, "addon.pathtools.movePointsTogether", json!({ "enabled": enabled }));
    }
}
