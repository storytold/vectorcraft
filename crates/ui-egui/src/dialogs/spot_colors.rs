//! Swatches panel → Spot Colors…: whether spot colours defined in Lab show and separate from
//! their Lab values or from their CMYK equivalents. OK runs `swatch.spotOptions`.
//!
//! Fields: `values` (`lab`: Lab values; `cmyk`: CMYK equivalents).

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind, ParamResult};
use serde_json::{Value, json};

use super::swatch_options::{NO_WINDOW, field_label, hosted};
use crate::VectorcraftApp;

/// The dialog kind of Spot Colors.
pub const KIND: &str = "spotColors";

/// The choices: (`useLab`, label, what it does).
const CHOICES: [(bool, &str, &str); 2] = [
    (true, "Lab Values", "Show and print from the Lab definition; PDF files carry it as the ink's alternate color."),
    (false, "CMYK Equivalents", "Show and separate through the working CMYK space, as in older documents."),
];

/// The `values` field's id of a choice.
const fn id(use_lab: bool) -> &'static str {
    if use_lab { "lab" } else { "cmyk" }
}

/// Open Spot Colors on the active document's setting.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let use_lab = app.session.active().ok_or("no document")?.doc.spot_use_lab;
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    let mut fields = DialogFields::new();
    fields.set("values", id(use_lab));
    let options = CHOICES.iter().map(|(lab, text, help)| (id(*lab).to_owned(), tl!(text).to_owned().into(), tl!(help).to_owned().into())).collect();
    let rows = vec![Param::new("values", field_label(tl!("Lab Spot Colors:")), ParamKind::radio_with_details(options, Some(id(true).to_owned())))];
    let dialog = hosted(&ctx, KIND, tl!("Spot Colors"), fields, rows, false);
    let ok = |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        let use_lab = answer.fields.text("values") == Some(id(true));
        app.run("swatch.spotOptions", json!({ "useLab": use_lab })).map(|_| ())
    };
    super::host::open_held(app, dialog, ok).ok_or(NO_WINDOW)?;
    Ok(Value::Null)
}
