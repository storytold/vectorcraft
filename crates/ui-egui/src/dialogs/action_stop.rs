//! Actions panel menu › Insert Stop… (`ui.action.insertStop`) and the message box a playing action
//! shows at a stop.
//!
//! Record Stop's fields: `message`, `allowContinue`, and `set`, `action` and `step`, the place of the
//! new stop (no `set` and `action`: the recording in progress). An invalid place is an error, and
//! the dialog stays open.
//!
//! The message box's fields: `action` (the action's name, shown as the heading), `message` and
//! `allowContinue`. Stop or Cancel closes it, and the playback keeps waiting for Play Current
//! Selection or Stop Playing/Recording. With `allowContinue`, Continue or `ui.dialog.confirm` plays
//! on; without it, `ui.dialog.confirm` presses Stop.

use serde_json::{Value, json};

use super::{DialogAction, DialogSpec};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Record Stop.
pub const RECORD_KIND: &str = "actionStopRecord";

/// The dialog kind of a stop's message box.
pub const MESSAGE_KIND: &str = "actionStopMessage";

/// The width of Record Stop's label column.
const LABEL_W: f32 = 70.0;

pub(super) const RECORD_SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Record Stop").into(), body: record_body, confirm: record_confirm, min_width: 340.0, ..DialogSpec::FORM };

pub(super) const MESSAGE_SPEC: DialogSpec = DialogSpec {
    heading: |d| d.str("action"),
    body_with_actions: Some(message_body),
    confirm: message_confirm,
    ok: None,
    min_width: 300.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};

/// Opens Record Stop for action `target` as (set, action), after `step`, else for the recording in
/// progress.
pub fn open_record(app: &mut VectorcraftApp, target: Option<(usize, usize)>, step: Option<usize>) {
    let mut fields = json!({"message": "", "allowContinue": false});
    if let Some((set, action)) = target {
        fields["set"] = json!(set);
        fields["action"] = json!(action);
        if let Some(step) = step {
            fields["step"] = json!(step);
        }
    }
    app.ui.dialog = Some(Dialog::new(RECORD_KIND, fields));
}

fn record_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, "Message:", LABEL_W, |ui| {
        super::form::text(ui, d, "message", 240.0);
    });
    super::form::check(ui, d, "allowContinue", "Allow Continue");
    false
}

fn record_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = json!({"message": d.str("message"), "allowContinue": d.bool("allowContinue")});
    for key in ["set", "action", "step"] {
        if let Some(value) = d.fields.get(key) {
            params[key] = value.clone();
        }
    }
    let r = app.run(crate::panels::actions::INSERT_STOP, params)?;
    app.ui.dialog = None;
    Ok(r)
}

/// Shows the message box of a stop in the action named `action`.
pub(crate) fn open_message(app: &mut VectorcraftApp, action: &str, message: &str, allow_continue: bool) {
    app.ui.dialog = Some(Dialog::new(MESSAGE_KIND, json!({"action": action, "message": message, "allowContinue": allow_continue})));
}

fn message_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let message = d.str("message");
    if !message.is_empty() {
        ui.label(message);
        ui.add_space(8.0);
    }
    ui.label(egui::RichText::new(tl!("Press Play Current Selection in the Actions panel to go on.")).color(Tokens::get(ui.ctx()).text_dim));
    ui.add_space(16.0);
    let mut action = DialogAction::None;
    ui.horizontal(|ui| {
        if d.bool("allowContinue") && widgets::primary_button_enabled(ui, tl!("Continue"), true).clicked() {
            action = DialogAction::Confirm;
        }
        if widgets::secondary_button(ui, tl!("Stop")).clicked() {
            action = DialogAction::Cancel;
        }
    });
    action
}

fn message_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.bool("allowContinue") {
        return app.run(crate::panels::actions::CONTINUE_PLAYBACK, json!({}));
    }
    app.ui.dialog = None;
    Ok(Value::Null)
}
