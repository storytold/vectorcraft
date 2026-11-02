//! Actions panel › Create New Set (and ≡ › New Set…), ≡ › Set Options… and ≡ › Action Options…:
//! the name of a new action set, of the current one, or of the selected action.
//!
//! Fields: `set` (the index of the set renamed, or of the action's set; none for a new set),
//! `action` (the index of the action renamed, in `set`; none for a set) and `name`. OK adds the set
//! and makes it the current one, renames the set, or renames the action (`ui.action.rename`). A
//! `set` or `action` field that is not the index of a set or action is an error, OK then changes
//! no set or action, and the dialog stays open.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of New Set, Set Options and Action Options.
pub const KIND: &str = "actionSetOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading, body, confirm, ok_enabled: Some(|_, d| !d.str("name").trim().is_empty()), min_width: 300.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    if d.fields.contains_key("action") {
        tl!("Action Options").into()
    } else if d.fields.contains_key("set") {
        tl!("Set Options").into()
    } else {
        tl!("New Set").into()
    }
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

/// Open Action Options for the action at `action` in set `set`, if there is one.
pub fn open_action(app: &mut VectorcraftApp, set: usize, action: usize) {
    if let Some(a) = app.ui.action_sets.get(set).and_then(|s| s.actions.get(action)) {
        app.ui.dialog = Some(Dialog::new(KIND, json!({"set": set, "action": action, "name": a.name})));
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
    // The fields the dialog holds choose what OK does, as they choose its heading.
    if d.fields.contains_key("action") {
        app.run(crate::panels::actions::RENAME_ACTION, json!({"set": d.fields.get("set"), "action": d.fields.get("action"), "name": name}))?;
    } else if let Some(set) = d.fields.get("set") {
        let s = set.as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| app.ui.action_sets.get_mut(i)).ok_or("no such action set")?;
        s.name = name;
    } else {
        app.ui.action_sets.push(crate::panels::actions::ActionSet::new(name, vec![]));
        app.ui.action_set = Some(app.ui.action_sets.len() - 1);
    }
    app.ui.dialog = None;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heading names the dialog by what it holds: an action, a set, or neither.
    #[test]
    fn the_heading_names_action_options_set_options_or_new_set() {
        let heading_of = |fields: Value| heading(&Dialog::new(KIND, fields));
        assert_eq!(heading_of(json!({"set": 1, "action": 0, "name": "Nudge"})), "Action Options");
        assert_eq!(heading_of(json!({"set": 1, "name": "Mine"})), "Set Options");
        assert_eq!(heading_of(json!({"name": "Set 1"})), "New Set");
    }
}
