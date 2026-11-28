//! Circle Tool Options (double-click the Circle tool, or `tool.options`): Show Center, Show Radii
//! and Show Diameter switch the center cross, the radius lines and the diameter label the tool
//! shows while a circle is drawn; Color is the color of the cross and the lines; Anchors at
//! Standard Positions off puts the circle's anchors at the points it was drawn through; Third
//! Point by Dragging off places the second point with a click and the third with another. OK sets
//! them with `tool.setOption {tool: "circle", values}`; they last across tool switches and are
//! saved with the preferences.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_tools::circle::{COLORS, ID as TOOL};

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of Circle Tool Options.
pub const KIND: &str = "circleOptions";

/// The options the dialog edits, in the order it shows its check boxes, with their labels.
const SWITCHES: [(&str, &str); 5] = [
    ("center", "Show Center"),
    ("radii", "Show Radii"),
    ("diameter", "Show Diameter"),
    ("standardAnchors", "Anchors at Standard Positions"),
    ("dragThird", "Third Point by Dragging"),
];

/// The labels of [`COLORS`], in its order.
const COLOR_LABELS: [&str; 6] = ["Red", "Blue", "Magenta", "Green", "Black", "Gray"];

/// The rows: the three display switches, the Color, then the two drawing switches.
fn params() -> Vec<Param> {
    let switch = |(key, label): &(&str, &'static str)| Param::new(*key, tl!(*label), ParamKind::bool(true));
    let colors = COLORS.iter().zip(COLOR_LABELS).map(|((name, _), label)| (name.to_string(), Label::from(tl!(label)))).collect();
    let mut rows: Vec<Param> = SWITCHES[..3].iter().map(switch).collect();
    rows.push(Param::new("color", tl!("Color:"), ParamKind::choice(colors, Some("blue".into()))));
    rows.extend(SWITCHES[3..].iter().map(switch));
    rows
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let params = params();
    let options = app.session.tool_options_of(TOOL);
    let fields = options.as_object().map(|m| form::fields_of(&params, m)).unwrap_or_default();
    let (title, shown) = (tool_options::title(TOOL), params.clone());
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| tool_options::set_options(app, TOOL, form::json_of(&params, &result.fields));
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| ParamDialog::new(KIND, title.clone(), shown.clone()).values(Bind::bound(f)));
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        app.status(tl!("No window to show the dialog in.").to_string());
    }
    json!({ "dialog": KIND })
}
