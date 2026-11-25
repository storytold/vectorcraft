//! Curvature Circle Tool Options (double-click the Curvature Circle tool, or `tool.options`): Snap
//! Tolerance is how near, in screen points, a path pulls the pointer (Smart Guides' Snapping
//! Tolerance applies when it is larger); with Snap with Smart Guides Off unchecked, the pointer
//! reaches no further than the selection tolerance while Smart Guides are off; Annotate Radius
//! off hides the radius label; Anchors at Standard Positions off gives the circle an anchor where
//! it touches the path and one every 90° from there; Use the Radius of Circular Arcs off gives the
//! curve's own curvature on the curve of a circular arc too. OK sets them with
//! `tool.setOption {tool: "curvatureCircle", values}`; they last across tool switches and are saved
//! with the preferences.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_tools::curvature_circle::{DEFAULT_SNAP, ID as TOOL, SNAP_RANGE};

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of Curvature Circle Tool Options.
pub const KIND: &str = "curvatureCircleOptions";

/// The switches the dialog edits, in the order it shows them, with their labels.
const SWITCHES: [(&str, &str); 4] = [
    ("snapWithoutGuides", "Snap with Smart Guides Off"),
    ("annotateRadius", "Annotate Radius"),
    ("standardAnchors", "Anchors at Standard Positions"),
    ("constantRadius", "Use the Radius of Circular Arcs"),
];

/// The rows: Snap Tolerance, then the switches.
fn params() -> Vec<Param> {
    let (min, max) = SNAP_RANGE;
    let snap = Param::number("snapTolerance", tl!("Snap Tolerance:"), DEFAULT_SNAP, 1.0, 1).unit(UnitKind::Pixels).bounds(Some(min), Some(max));
    std::iter::once(snap).chain(SWITCHES.iter().map(|(key, label)| Param::new(*key, tl!(*label), ParamKind::bool(true)))).collect()
}

/// Open the tool's options with their current values; an error when there is no window to show them in.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
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
        let message = tl!("No window to show the dialog in.").to_string();
        app.status(message.clone());
        return Err(message);
    }
    Ok(json!({ "dialog": KIND }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// `tool.setOption` takes only the values the options take, and an invalid one changes none.
    #[test]
    fn invalid_values_are_errors_and_change_nothing() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        for values in [
            json!({"annotateRadius": false, "constantRadius": "no"}),
            json!({"annotateRadius": false, "snapTolerance": 0}),
            json!({"annotateRadius": false, "size": 3}),
        ] {
            assert!(app.run("tool.setOption", json!({"tool": TOOL, "values": values})).is_err(), "{values}");
            assert_eq!(app.session.tool_options_of(TOOL)["annotateRadius"], json!(true));
        }
    }

    /// Without a window the dialog cannot open, and `tool.options` says so instead of reporting it open.
    #[test]
    fn without_a_window_opening_the_options_is_an_error() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        assert!(app.run("tool.options", json!({"tool": TOOL})).is_err());
    }
}
