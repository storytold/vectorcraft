//! Actions panel › Create New Action (and ≡ › New Action…): the name, set, function key and color
//! of a new action. Record adds the action and starts recording into it (`ui.action.new`).
//!
//! Fields: `set` (the index of the set the action goes into), `name`, `key` (0 for none, 1 to 15
//! for F1 to F15), `cmd` (Ctrl, Command on macOS), `shift`, `color` (a value of
//! [`crate::panels::actions::COLORS`], `""` for none). An invalid set or function key is an error:
//! Record then adds nothing and the dialog stays open.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::panels::actions::{COLORS, FUNCTION_KEYS, FunctionKey};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of New Action.
pub const KIND: &str = "actionNew";

/// The width of the label column.
const LABEL_W: f32 = 90.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("New Action").into(),
    body,
    confirm,
    ok: Some("Record"),
    ok_enabled: Some(|_, d| !d.str("name").trim().is_empty()),
    min_width: 340.0,
    ..DialogSpec::FORM
};

/// Whether New Action can open: there is a set to put the action in and nothing is recording.
pub fn can_open(app: &VectorcraftApp) -> bool {
    !app.ui.action_sets.is_empty() && app.ui.recording.is_none()
}

/// Opens New Action for the current set, else the first set, named "Action N" with the first
/// number from the set's action count plus one that no action of the set uses, with no function
/// key and no color.
pub fn open(app: &mut VectorcraftApp) -> Result<(), String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    let len = app.ui.action_sets.len();
    let set = app.ui.action_set.filter(|&s| s < len).unwrap_or(0);
    let Some(s) = app.ui.action_sets.get(set) else { return Err("no such action set".into()) };
    let taken = |n: usize| s.actions.iter().any(|a| a.name == format!("Action {n}"));
    let n = (s.actions.len() + 1..).find(|&n| !taken(n)).unwrap_or(1);
    let fields = json!({"set": set, "name": format!("Action {n}"), "key": 0, "cmd": false, "shift": false, "color": ""});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(())
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, "Name:", LABEL_W, |ui| {
        super::form::text(ui, d, "name", 200.0);
    });
    let names: Vec<&str> = app.ui.action_sets.iter().map(|s| s.name.as_str()).collect();
    let set = d.fields.get("set").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let shown = set.and_then(|s| names.get(s)).copied().unwrap_or_default();
    widgets::label_row(ui, "Action Set:", LABEL_W, |ui| {
        if let Some(i) = widgets::dropdown_names(ui, "action-new-set", shown, &names, 200.0) {
            d.fields.insert("set".into(), json!(i));
        }
    });
    key_and_color(ui, d);
    false
}

/// The Function Key and Color rows of New Action and Action Options show errors for invalid keys.
pub(super) fn key_and_color(ui: &mut egui::Ui, d: &mut Dialog) {
    let key = d.fields.get("key").and_then(Value::as_u64).unwrap_or(0);
    let none = tl!("None");
    let keys: Vec<String> = std::iter::once(none.to_string()).chain((1..=FUNCTION_KEYS).map(|n| format!("F{n}"))).collect();
    let labels: Vec<&str> = keys.iter().map(String::as_str).collect();
    let shown = usize::try_from(key).ok().and_then(|k| labels.get(k)).copied().unwrap_or(none);
    let modifier = if crate::keymap::platform() == craft_keymap::Platform::Mac { "⌘" } else { "Ctrl" };
    widgets::label_row(ui, "Function Key:", LABEL_W, |ui| {
        if let Some(i) = widgets::dropdown_names(ui, "action-key", shown, &labels, 70.0) {
            d.fields.insert("key".into(), json!(i));
        }
        super::form::check(ui, d, "cmd", modifier);
        super::form::check(ui, d, "shift", "Shift");
    });
    let picked = FunctionKey::from_params(&json!({"key": key, "cmd": d.bool("cmd"), "shift": d.bool("shift")}));
    if let Ok(Some(k)) = picked
        && let Some(message) = crate::panels::actions::key_error(k)
    {
        widgets::label_row(ui, "", LABEL_W, |ui| {
            ui.label(crate::i18n::msg(&message));
        });
    }
    let colors: Vec<(&str, &str)> = std::iter::once(("", "None")).chain(COLORS).collect();
    super::form::choice(ui, d, "color", "Color:", (LABEL_W, 120.0), &colors);
}

/// The function key and color params the dialog's fields give, as `ui.action.new` and
/// `ui.action.options` take them.
pub(super) fn key_and_color_params(d: &Dialog) -> Value {
    json!({
        "key": d.fields.get("key").and_then(Value::as_u64).unwrap_or(0),
        "cmd": d.bool("cmd"),
        "shift": d.bool("shift"),
        "color": d.str("color"),
    })
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let name = d.str("name").trim().to_string();
    // Record is disabled without a name; `ui.dialog.confirm` keeps the dialog open the same way.
    if name.is_empty() {
        return Ok(Value::Null);
    }
    let mut p = key_and_color_params(d);
    p["set"] = d.fields.get("set").cloned().unwrap_or(Value::Null);
    p["name"] = json!(name);
    app.run(crate::panels::actions::NEW_ACTION, p)?;
    app.ui.dialog = None;
    Ok(Value::Null)
}
