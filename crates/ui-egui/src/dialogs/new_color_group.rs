//! New Color Group: a name and what the group is made from, the swatches selected in the Swatches
//! panel or the colours of the selected artwork (whose process colours can become global swatches
//! the art links to, with swatches for tints too). OK runs `swatch.newGroup`.
//!
//! Fields: `name`, `from` (`swatches` or `artwork`), `toGlobal` and `includeTints` (artwork only). The selected
//! swatch names are kept by the dialog's OK.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind, ParamResult};
use serde_json::{Value, json};

use super::swatch_options::{NO_WINDOW, field_label, hosted};
use crate::VectorcraftApp;

/// The dialog kind of New Color Group.
pub const KIND: &str = "newColorGroup";

/// Open New Color Group for the swatches selected in the panel. It starts from the artwork when
/// art is selected and no swatches are.
pub fn open(app: &mut VectorcraftApp, swatches: Vec<String>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let art = !st.selection.is_empty();
    let mut fields = DialogFields::new();
    fields.set("name", st.doc.free_swatch_name("Color Group"));
    fields.set("from", if art && swatches.is_empty() { "artwork" } else { "swatches" });
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    let sources =
        vec![("swatches".to_owned(), tl!("Selected Swatches").to_owned().into()), ("artwork".to_owned(), tl!("Selected Artwork").to_owned().into())];
    let rows = vec![
        Param::new("name", field_label(tl!("Name:")), ParamKind::text("")),
        Param::new("from", field_label(tl!("Create From:")), ParamKind::radio(sources, Some("swatches".to_owned()))).disabled_options(if art {
            None
        } else {
            Some("artwork")
        }),
        Param::new("toGlobal", tl!("Convert Process to Global").to_owned(), ParamKind::bool(true)).enabled_when("from", "artwork"),
        Param::new("includeTints", tl!("Include Swatches for Tints").to_owned(), ParamKind::bool(false)).enabled_when("from", "artwork"),
    ];
    let dialog = hosted(&ctx, KIND, tl!("New Color Group"), fields, rows, false);
    let ok = move |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        app.run("swatch.newGroup", params(&answer.fields, &swatches)).map(|_| ())
    };
    super::host::open_held(app, dialog, ok).ok_or(NO_WINDOW)?;
    Ok(Value::Null)
}

/// `swatch.newGroup` parameters from the fields and the swatches selected when the dialog opened.
fn params(fields: &DialogFields, swatches: &[String]) -> Value {
    let name = fields.text("name").unwrap_or_default();
    if fields.text("from") == Some("artwork") {
        let on = |key: &str| fields.bool(key).unwrap_or(false);
        json!({"name": name, "fromArtwork": true, "toGlobal": on("toGlobal"), "includeTints": on("includeTints")})
    } else {
        json!({"name": name, "swatches": swatches})
    }
}
