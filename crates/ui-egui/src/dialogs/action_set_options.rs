//! Actions panel › Create New Set (and ≡ › New Set…) and ≡ › Set Options…: the name of a new action
//! set, or of the current one.
//!
//! Fields: `set` (the index of the set renamed; none for a new set) and `name`. OK adds the set and
//! makes it the current one, or renames the set.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of New Set and Set Options.
pub const KIND: &str = "actionSetOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading, body, confirm, ok_enabled: Some(|_, d| !d.str("name").trim().is_empty()), min_width: 300.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    if d.fields.contains_key("set") { tl!("Set Options").into() } else { tl!("New Set").into() }
}

/// Open New Set, named "Set N" with the first number no set uses.
pub fn open_new(app: &mut VectorcraftApp) {
    let taken = |n: usize| app.ui.action_sets.iter().any(|s| s.name == format!("Set {n}"));
    let n = (1..).find(|&n| !taken(n)).unwrap_or(1);
    app.ui.dialog = Some(Dialog::new(KIND, json!({"name": format!("Set {n}")})));
}

/// Open Set Options for the set at `set`, if there is one.
pub fn open(app: &mut VectorcraftApp, set: usize) {
    if let Some(s) = app.ui.action_sets.get(set) {
        app.ui.dialog = Some(Dialog::new(KIND, json!({"set": set, "name": s.name})));
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, "Name:", 50.0, |ui| {
        super::form::text(ui, d, "name", 200.0);
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let name = d.str("name").trim().to_string();
    // OK is disabled without a name; `ui.dialog.confirm` keeps the dialog open the same way.
    if name.is_empty() {
        return Ok(Value::Null);
    }
    match d.fields.get("set").and_then(Value::as_u64) {
        Some(set) => {
            if let Some(s) = app.ui.action_sets.get_mut(set as usize) {
                s.name = name;
            }
        }
        None => {
            app.ui.action_sets.push(crate::panels::actions::ActionSet { name, actions: vec![] });
            app.ui.action_set = Some(app.ui.action_sets.len() - 1);
        }
    }
    app.ui.dialog = None;
    Ok(Value::Null)
}
