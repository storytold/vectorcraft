//! A question answered with OK or Cancel; OK runs a command ([`ask`]). Used for confirmations such
//! as deleting swatches.
//!
//! Fields: `message` (the question, shown as the heading), `detail` (an optional line under it),
//! `__command` and `__params` (the command OK runs). Both texts are shown as given: callers
//! translate their template before filling in a name, so the name is never translated.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

/// The dialog kind of a confirmation.
pub const KIND: &str = "confirm";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| d.str("message"),
    body: |_, ui, d| {
        let detail = d.str("detail");
        if !detail.is_empty() {
            ui.label(egui::RichText::new(detail).color(Tokens::get(ui.ctx()).text_dim));
        }
        false
    },
    confirm,
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Ask `message` (with an optional `detail` line), both in the UI language; OK runs `command`
/// with `params`.
pub fn ask(app: &mut VectorcraftApp, message: &str, detail: &str, command: &str, params: Value) {
    let fields = json!({"message": message, "detail": detail, "__command": command, "__params": params});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// The dialog kind of a message with a Close button only ([`tell`]).
pub const MESSAGE: &str = "message";

pub(super) const MESSAGE_SPEC: DialogSpec = DialogSpec { ok: None, ..SPEC };

/// Tell the user `message` (with an optional `detail` line), both in the UI language, in a dialog
/// they close.
pub fn tell(app: &mut VectorcraftApp, message: &str, detail: &str) {
    app.ui.dialog = Some(Dialog::new(MESSAGE, json!({"message": message, "detail": detail})));
}

/// Closes before running, so a dialog the command opens stays open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = d.fields.get("__params").cloned().unwrap_or_else(|| json!({}));
    app.ui.dialog = None;
    app.run(&d.str("__command"), params)
}
