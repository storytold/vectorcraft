//! Three-Point Arc Tool Options (double-click the Three-Point Arc tool, or `tool.options`):
//! Third Point Sets Direction, the tool's `direction` option. With it on, the third point gives the
//! direction the arc leaves an end in; with it off, the arc passes through the third point. Snap
//! Where the Arc Continues a Path (`tangentSnap`, on by default) snaps the third point, in
//! direction mode, to where the arc carries on an open path it starts at. Join to Open Path Ends
//! (`join`) joins the arc to the open paths whose ends its ends are placed on. OK sets them with
//! `tool.setOption {tool: "threePointArc", values: {direction, tangentSnap, join}}`; they last
//! across tool switches and are saved with the preferences. `]` also switches `direction`, and J
//! `join`, while an arc is drawn.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::{Value, json};
use vectorcraft_tools::three_point_arc::ID as TOOL;

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of Three-Point Arc Tool Options.
pub const KIND: &str = "threePointArcOptions";

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let params = vec![
        Param::new("direction", tl!("Third Point Sets Direction"), ParamKind::bool(false)),
        Param::new("tangentSnap", tl!("Snap Where the Arc Continues a Path"), ParamKind::bool(true)),
        Param::new("join", tl!("Join to Open Path Ends"), ParamKind::bool(false)),
    ];
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
