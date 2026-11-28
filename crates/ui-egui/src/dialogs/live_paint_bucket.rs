//! Live Paint Bucket Options (double-click the Live Paint Bucket tool): what a click with the tool
//! paints and how the face or edge under the pointer highlights. The options last across tool
//! switches and are saved with the preferences. OK sets them with
//! `tool.setOption {tool: "livePaintBucket", values}`.
//!
//! Fields: `paintFills` and `paintStrokes` (the faces and the edges of Live Paint groups),
//! `cursorSwatchPreview` (whether the active paint and the color swatches beside it show above the
//! pointer), `highlight` (whether the face or edge under the pointer is outlined), `color` (the highlight
//! color: `#rrggbb`, or a layer color's name such as "Red") and `width` (the width of a face's
//! highlight in screen pixels; an edge's highlight is half as wide).

use craft::ui::runtime::env::UnitKind;
use serde_json::Value;
use vectorcraft_tools::builder::LP_BUCKET_WIDTH_RANGE;

use super::option_form::{self, OptionSpec, Rows, is_on};
use crate::VectorcraftApp;

/// The dialog kind of the Live Paint Bucket Options.
pub const KIND: &str = "livePaintBucketOptions";

/// The tool the dialog serves.
const TOOL: &str = "livePaintBucket";

/// The options the dialog edits.
const KEYS: [&str; 6] = ["paintFills", "paintStrokes", "cursorSwatchPreview", "highlight", "color", "width"];

/// The dialog's description for the option form.
pub(super) static SPEC: OptionSpec = OptionSpec {
    key: KIND,
    title: || option_form::tool_title(TOOL),
    keys: &KEYS,
    colors: &["color"],
    preview: false,
    body,
    // The highlight's color and width take input while it is on.
    inactive: |values, key| matches!(key, "color" | "width") && !is_on(values, "highlight"),
    run: |app, values, _| option_form::set_tool_options(app, TOOL, values),
    cancelled: None,
};

/// Open the Live Paint Bucket Options with the tool's current values.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    option_form::open_tool(app, &SPEC, TOOL)
}

fn body(ui: &mut egui::Ui, rows: &mut Rows<'_>) {
    rows.check(ui, "paintFills", tl!("Paint Fills"), true);
    rows.check(ui, "paintStrokes", tl!("Paint Strokes"), true);
    rows.check(ui, "cursorSwatchPreview", tl!("Cursor Swatch Preview"), true);
    rows.check(ui, "highlight", tl!("Highlight"), true);
    let highlight = rows.flag("highlight");
    rows.field(ui, tl!("Color"), highlight, |ui, rows, label| rows.color(ui, "color", label, highlight));
    let (min, max) = LP_BUCKET_WIDTH_RANGE;
    rows.field(ui, tl!("Width"), highlight, |ui, rows, label| rows.number(ui, "width", label, min..=max, UnitKind::Pixels, highlight));
}
