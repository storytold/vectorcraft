//! Pixel Snapping Options (the Control bar's Snapping menu): whether View › Snap to Pixel lands
//! drawing, moving and scaling on whole pixels. OK runs `view.snapToPixel.options` (the options live
//! in the preferences); Cancel leaves them as they were.
//!
//! Fields: `drawing`, `moving` and `scaling` (bool). While Snap to Pixel is off at open, the
//! options stay editable and a note says they take effect once it is on.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::Value;

use super::{form, host};
use crate::VectorcraftApp;

/// The dialog kind of Pixel Snapping Options.
pub const KIND: &str = "pixelSnapOptions";

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    let mut params: Vec<Param> =
        [("drawing", "Snap to Pixel While Drawing"), ("moving", "Snap to Pixel While Moving"), ("scaling", "Snap to Pixel While Scaling")]
            .into_iter()
            .map(|(key, label)| Param::new(key, tl!(label), ParamKind::bool(true)))
            .collect();
    if !app.ui.view.snap_to_pixel {
        params.push(Param::read_only_text("note", "", tl!("These options take effect when View › Snap to Pixel is on.")).full_width());
    }
    let options = serde_json::json!(app.session.prefs.pixel_snap);
    let fields = options.as_object().map(|m| form::fields_of(&params, m)).unwrap_or_default();
    let shown = params.clone();
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        app.run("view.snapToPixel.options", Value::Object(form::json_of(&params, &result.fields))).map(|_| ())
    };
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| {
            ParamDialog::new(KIND, tl!("Pixel Snapping Options"), shown.clone()).values(Bind::bound(f))
        });
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        app.status(tl!("No window to show the dialog in.").to_string());
    }
}
