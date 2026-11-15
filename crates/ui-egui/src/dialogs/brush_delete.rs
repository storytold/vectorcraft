//! The Brushes panel's Delete Brush question ([`ask`]): Yes or No when none of the brushes is in
//! use, else Remove Strokes or Cancel. Yes and Remove Strokes run `brush.delete`, which leaves the
//! strokes that used the brushes as plain strokes.
//!
//! Fields: `message` (the question, shown as the heading), `used` (whether a brush is in use),
//! `params` (the `brush.delete` params) and `__doc` (the uid of the document it asks about).

use serde_json::{Value, json};

use super::{DialogAction, DialogSpec};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::widgets;

/// The dialog kind of the Delete Brush question.
pub const KIND: &str = "brushDelete";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |d| d.str("message"), body_with_actions: Some(body), confirm, ok: None, max_width: Some(540.0), ..DialogSpec::FORM };

/// Ask whether to delete the brushes `params` names (`used`: one of them is in use).
pub fn ask(app: &mut VectorcraftApp, params: Value, used: bool) {
    let message = if used {
        tl!("Strokes use one or more of these brushes. Take the brushes off those strokes and delete the brushes?")
    } else {
        tl!("Delete the selected brushes?")
    };
    let doc = app.session.active().map(|d| d.uid);
    app.ui.dialog = Some(Dialog::new(KIND, json!({"params": params, "used": used, "message": message, DOC: doc})));
}

/// The field that holds the uid of the document the question is about.
const DOC: &str = "__doc";

/// Whether a stroke uses one of `names` (`brush.unused`), or `None` when the query fails.
pub fn used(app: &mut VectorcraftApp, names: &[String]) -> Option<bool> {
    let unused = app.run("brush.unused", json!({})).ok()?;
    let unused = unused.get("names").and_then(Value::as_array).cloned().unwrap_or_default();
    Some(names.iter().any(|name| !unused.iter().any(|n| n.as_str() == Some(name))))
}

/// The brush names in `brush.delete` params (`name` or `names`).
fn names(params: &Value) -> Vec<String> {
    match params.get("name").and_then(Value::as_str) {
        Some(name) => vec![name.to_string()],
        None => params.get("names").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let mut action = DialogAction::None;
    ui.horizontal(|ui| {
        if d.bool("used") {
            if widgets::primary_button_enabled(ui, tl!("Remove Strokes"), true).clicked() {
                action = DialogAction::Confirm;
            }
            if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                action = DialogAction::Cancel;
            }
        } else {
            if widgets::primary_button_enabled(ui, tl!("Yes"), true).clicked() {
                action = DialogAction::Confirm;
            }
            if widgets::secondary_button(ui, tl!("No")).clicked() {
                action = DialogAction::Cancel;
            }
        }
    });
    action
}

/// Closes before running, so a dialog the command opens stays open. Yes on brushes that a stroke
/// has started to use since the question opened asks again with Remove Strokes and Cancel. When
/// another document has become the active one, it closes as Cancel does and deletes nothing.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = d.fields.get("params").cloned().unwrap_or_else(|| json!({}));
    if d.fields.get(DOC).and_then(Value::as_u64) != app.session.active().map(|doc| doc.uid) {
        app.ui.dialog = None;
        return Ok(json!({}));
    }
    if !d.bool("used") && used(app, &names(&params)) == Some(true) {
        ask(app, params, true);
        return Ok(json!({}));
    }
    app.ui.dialog = None;
    app.run("brush.delete", params)
}

#[cfg(test)]
mod tests;
