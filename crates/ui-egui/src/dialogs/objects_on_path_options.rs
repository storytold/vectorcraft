//! Objects on Path Tool Options (double-click the Objects on Path tool): the Pivot and Rotate of
//! the objects the tool attaches next. Groups attached before keep theirs. Save sets them with
//! `tool.setOption {tool: "objectsOnPath", values}`; they last across tool switches and are saved
//! with the preferences. Reset puts the center and 0 back.
//!
//! Fields: `pivot` (0-8, row by row from the top left; 4, the center) and `rotate` (degrees
//! counter-clockwise from upright on the path).
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Outcome as _, Show as _, clicked};
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamResult};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_doc::objects_on_path::CENTER;

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of Objects on Path Tool Options.
pub const KIND: &str = "objectsOnPathOptions";

/// The tool whose options the dialog edits.
const TOOL: &str = "objectsOnPath";

/// The rows: Pivot, Rotate, and Reset, which puts the center and 0 back in the fields.
fn params() -> Vec<Param> {
    vec![
        tool_options::reference_param("pivot", tl!("Pivot:")),
        Param::number("rotate", tl!("Rotate:"), 0.0, 1.0, 1).unit(UnitKind::Degrees).bounds(Some(-360.0), Some(360.0)),
        Param::custom("reset", "", |ui, fields, _| {
            let out = Button::new(tl!("Reset")).show(ui);
            if clicked(out.response()) {
                fields.set("pivot", i64::from(CENTER));
                fields.set("rotate", 0.0);
            }
            out.response().clone()
        }),
    ]
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let params = params();
    let options = app.session.tool_options_of(TOOL);
    let fields = options.as_object().map(|m| form::fields_of(&params, m)).unwrap_or_default();
    let (title, shown) = (tool_options::title(TOOL), params.clone());
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| tool_options::set_options(app, TOOL, form::json_of(&params, &result.fields));
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| {
            ParamDialog::new(KIND, title.clone(), shown.clone()).ok_label(tl!("Save").into()).values(Bind::bound(f))
        });
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        app.status(tl!("No window to show the dialog in.").to_string());
    }
    json!({ "dialog": KIND })
}
