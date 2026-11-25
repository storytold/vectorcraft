//! Live Paint Selection Tool Options (double-click the Live Paint Selection tool): what a click
//! with the tool selects and how the face or edge under the pointer highlights. The options last
//! across tool switches and are saved with the preferences. OK sets them with
//! `tool.setOption {tool: "livePaintSelection", values}`.
//!
//! Fields: `selectFills` and `selectStrokes` (the faces and the edges of Live Paint groups),
//! `color` (the highlight color: `#rrggbb`, or a layer color's name such as "Red") and `width`
//! (the highlight's width in screen pixels).

use craft::ui::runtime::env::UnitKind;
use serde_json::Value;
use vectorcraft_tools::builder::LP_SELECT_WIDTH_RANGE;

use super::option_form::{self, OptionSpec, Rows};
use crate::VectorcraftApp;

/// The dialog kind of the Live Paint Selection Tool Options.
pub const KIND: &str = "livePaintSelectionOptions";

/// The tool the dialog serves.
const TOOL: &str = "livePaintSelection";

/// The options the dialog edits.
const KEYS: [&str; 4] = ["selectFills", "selectStrokes", "color", "width"];

/// The dialog's description for the option form.
pub(super) static SPEC: OptionSpec = OptionSpec {
    key: KIND,
    title: || option_form::tool_title(TOOL),
    keys: &KEYS,
    colors: &["color"],
    preview: false,
    body,
    inactive: |_, _| false,
    run: |app, values, _| option_form::set_tool_options(app, TOOL, values),
    cancelled: None,
};

/// Open the Live Paint Selection Tool Options with the tool's current values.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    option_form::open_tool(app, &SPEC, TOOL)
}

fn body(ui: &mut egui::Ui, rows: &mut Rows<'_>) {
    rows.check(ui, "selectFills", tl!("Select Fills"), true);
    rows.check(ui, "selectStrokes", tl!("Select Strokes"), true);
    rows.field(ui, tl!("Color"), true, |ui, rows, label| rows.color(ui, "color", label, true));
    let (min, max) = LP_SELECT_WIDTH_RANGE;
    rows.field(ui, tl!("Width"), true, |ui, rows, label| rows.number(ui, "width", label, min..=max, UnitKind::Pixels, true));
}
