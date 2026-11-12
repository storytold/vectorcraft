//! Actions panel menu › Select Object… (`ui.action.insertSelectObject`): Set Selection, which adds a
//! step that selects the objects whose Attributes note matches the note typed.
//!
//! Its fields: `note`, `caseSensitive`, `wholeWord`, and `set`, `action` and `step`, the place of the
//! new step (no `set` and `action`: the recording in progress). OK is off while the note is empty.
//! An invalid note or place is an error, and the dialog stays open.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Set Selection.
pub const KIND: &str = "actionSelectObject";

/// The width of the Note label column.
const LABEL_W: f32 = 70.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Set Selection").into(),
    body,
    confirm,
    ok_enabled: Some(|_, d| !d.str("note").trim().is_empty()),
    min_width: 340.0,
    ..DialogSpec::FORM
};

/// Opens Set Selection for action `target` as (set, action), after `step`, else for the recording
/// in progress.
pub fn open(app: &mut VectorcraftApp, target: Option<(usize, usize)>, step: Option<usize>) {
    let mut fields = json!({"note": "", "caseSensitive": false, "wholeWord": false});
    if let Some((set, action)) = target {
        fields["set"] = json!(set);
        fields["action"] = json!(action);
        if let Some(step) = step {
            fields["step"] = json!(step);
        }
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, "Note:", LABEL_W, |ui| {
        super::form::text(ui, d, "note", 240.0);
    });
    super::form::check(ui, d, "caseSensitive", "Case Sensitive");
    super::form::check(ui, d, "wholeWord", "Whole Word");
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = json!({"note": d.str("note"), "caseSensitive": d.bool("caseSensitive"), "wholeWord": d.bool("wholeWord")});
    for key in ["set", "action", "step"] {
        if let Some(value) = d.fields.get(key) {
            params[key] = value.clone();
        }
    }
    let r = app.run(crate::panels::actions::INSERT_SELECT_OBJECT, params)?;
    app.ui.dialog = None;
    Ok(r)
}
