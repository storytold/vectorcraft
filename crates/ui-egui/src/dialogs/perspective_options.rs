//! Perspective Grid Options (double-click the Perspective Grid tool): whether the Plane Switching
//! Widget shows and which corner of the window it sits in, and what Automatic Plane Positioning
//! takes a point from. OK runs `perspective.widget.options` (the options live in the preferences).
//!
//! Fields: `show` (bool), `position` (`topLeft`, `topRight`, `bottomLeft`, `bottomRight`),
//! `anchorPoint` (bool) and `gridlineIntersection` (bool).

use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use serde_json::{Value, json};
use vectorcraft_tools::distort::perspective::widget::WidgetCorner;

use super::form;
use super::host::open_held;
use crate::VectorcraftApp;

/// The dialog kind of Perspective Grid Options.
pub const KIND: &str = "perspectiveGridOptions";

/// The command OK runs.
const CMD: &str = "perspective.widget.options";

/// The dialog's rows; the position row is off while the widget is hidden.
fn params() -> Vec<Param> {
    let corners = WidgetCorner::ALL.iter().map(|c| (c.id().to_string(), c.label().into())).collect();
    vec![
        Param::new("show", tl!("Show Active Plane Widget"), ParamKind::bool(true)),
        Param::new("position", tl!("Widget Position:"), ParamKind::choice(corners, None)).enabled_when("show", true),
        Param::section(tl!("Automatic Plane Positioning")),
        Param::new("anchorPoint", tl!("Anchor Point of Perspective Artwork"), ParamKind::bool(true)),
        Param::new("gridlineIntersection", tl!("Intersection of Gridlines"), ParamKind::bool(true)),
    ]
}

fn build(_fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new(KIND, tl!("Perspective Grid Options"), params())
}

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("no window".to_string());
        return;
    };
    let Value::Object(map) = json!(app.session.prefs.perspective_widget) else { return };
    let dialog = HostedParamDialog::new(&ctx, KIND, form::fields_of(&params(), &map), build);
    // A refusal keeps the dialog open with the reason under its rows.
    let ok = |app: &mut VectorcraftApp, r: &craft::ui::patterns::form_dialogs::ParamResult| {
        app.run(CMD, Value::Object(form::json_of(&params(), &r.fields))).map(|_| ())
    };
    if open_held(app, dialog, ok).is_none() {
        app.status("no window".to_string());
    }
}
