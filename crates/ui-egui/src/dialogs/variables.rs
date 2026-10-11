//! The Variables dialogs (`variable.define`, `variable.bind`, `dataset.new`, `dataset.set`
//! and `dataset.select`), opened from Window › Variables or the command palette.
//!
//! One dialog covers all of them: what it shows is what the command takes. A variable is a
//! name and a kind; a dataset is a name and one field per variable — a text field for a
//! `text` variable, a checkbox for a `visibility` one. `dataset.set` opens with the row's
//! current values, so editing a row is editing what it already says.

use serde_json::{Value, json};
use vectorcraft_doc::{DataValue, VariableKind, Variables};

use super::{DialogSpec, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// The dialog kind of every Variables dialog.
pub const KIND: &str = "variables";

/// The kinds a variable can have, as `variable.define` names them.
const KINDS: [&str; 2] = ["text", "visibility"];

/// What the dialog shows for each of [`KINDS`].
fn kind_labels() -> [&'static str; 2] {
    [tl!("Text"), tl!("Visibility")]
}

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |d| tl!(&d.str("__label")).to_string(), body, confirm, min_width: 380.0, ..DialogSpec::FORM };

/// The command the dialog runs and what it is called.
fn opened(id: &str) -> Option<&'static str> {
    Some(match id {
        "variable.define" => "New Variable",
        "variable.rename" => "Variable Options",
        "variable.bind" => "Bind Variable",
        "dataset.new" => "New Data Set",
        "dataset.set" => "Edit Data Set",
        "dataset.rename" => "Rename Data Set",
        "dataset.select" => "Select Data Set",
        _ => return None,
    })
}

/// The row `id` renames, which the dialog offers as a dropdown when the document has more than
/// one. Variable Options renames the one the panel has highlighted, so it starts there.
fn rename_of(id: &str) -> Option<&'static str> {
    Some(match id {
        "variable.rename" => "variable",
        "dataset.rename" => "name",
        _ => return None,
    })
}

/// Open `id`'s dialog: OK runs the command with the fields below. `preselect` names the row
/// a rename starts on (the panel's highlighted variable, the active dataset); without it, or
/// when it no longer exists, the first row — the dialog still offers the dropdown.
pub fn open(app: &mut VectorcraftApp, id: &str, preselect: Option<&str>) -> bool {
    let Some(label) = opened(id) else { return false };
    let Some(st) = app.session.active() else {
        app.status(tl!("no document").to_string());
        return true;
    };
    let vars = st.doc.variables.clone();
    let mut fields = serde_json::Map::new();
    match id {
        "variable.define" => {
            fields.insert("name".into(), json!(""));
            fields.insert("kind".into(), json!(KINDS[0]));
        }
        "variable.bind" => {
            fields.insert("variable".into(), json!(""));
        }
        "dataset.new" => {
            fields.insert("name".into(), json!(""));
            value_fields(&vars, vars.datasets.first().map(|d| &d.values), &mut fields);
        }
        "dataset.set" => {
            // Without a row to edit there is nothing to open.
            let Some(name) = vars.active_dataset.clone().or_else(|| vars.datasets.first().map(|d| d.name.clone())) else {
                app.status(tl!("no data set to edit").to_string());
                return true;
            };
            let Some(current) = vars.dataset(&name) else { return true };
            fields.insert("name".into(), json!(name));
            value_fields(&vars, Some(&current.values), &mut fields);
        }
        "dataset.select" => {
            fields.insert("name".into(), json!(""));
        }
        _ => {
            // A rename: which row, and its new name.
            let key = rename_of(id).unwrap_or("name");
            let names: Vec<&str> = if key == "variable" {
                vars.variables.iter().map(|v| v.name.as_str()).collect()
            } else {
                vars.datasets.iter().map(|d| d.name.as_str()).collect()
            };
            if names.is_empty() {
                app.status(if key == "variable" { tl!("no variable to rename").to_string() } else { tl!("no data set to rename").to_string() });
                return true;
            }
            fields.insert(key.into(), json!(preselect.filter(|p| names.contains(p)).or(names.first().copied()).unwrap_or_default()));
            fields.insert("newName".into(), json!(""));
        }
    }
    fields.insert("__command".into(), json!(id));
    fields.insert("__label".into(), json!(label));
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
    true
}

/// Whether `id` opens one of these dialogs.
pub fn opens(id: &str) -> bool {
    opened(id).is_some()
}

/// One field per variable, prefilled from `current` (the row being edited, else the first row
/// so a new one starts from something rather than from nothing).
fn value_fields(vars: &Variables, current: Option<&std::collections::BTreeMap<String, DataValue>>, out: &mut serde_json::Map<String, Value>) {
    for v in &vars.variables {
        let value = match (current.and_then(|c| c.get(&v.name)), v.kind) {
            (Some(DataValue::Text(t)), _) => json!(t),
            (Some(DataValue::Visible(b)), _) => json!(b),
            // Without a value in the row a text field starts empty (no value in this row) and
            // a visibility one starts shown: its false is a value, so it is never "unset".
            (None, VariableKind::Text) => json!(""),
            (None, VariableKind::Visibility) => json!(true),
        };
        out.insert(value_key(&v.name), value);
    }
}

/// A variable name as a field key. A field key shares the dialog's flat namespace with the
/// command's own parameters, so names are prefixed to stay clear of `name` and `values`.
fn value_key(name: &str) -> String {
    format!("value:{name}")
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        widgets::dim_label(ui, tl!("No document"));
        return false;
    };
    let vars = &st.doc.variables;
    let variable_names: Vec<&str> = vars.variables.iter().map(|v| v.name.as_str()).collect();
    let dataset_names: Vec<&str> = vars.datasets.iter().map(|x| x.name.as_str()).collect();
    egui::Grid::new("variables-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        let label = |ui: &mut egui::Ui, text: &str| widgets::field_label(ui, egui::RichText::new(text).color(t.text));
        match d.str("__command").as_str() {
            "variable.define" => {
                label(ui, tl!("Name"));
                text(ui, d, "name", 220.0);
                ui.end_row();
                label(ui, tl!("Kind"));
                // The kinds by their translated names; the field keeps the command's own word.
                let labels = kind_labels();
                let shown = KINDS.iter().position(|k| *k == d.str("kind")).and_then(|i| labels.get(i)).copied().unwrap_or_default();
                if let Some(kind) = widgets::dropdown_names(ui, ("variables-dd", "kind"), shown, &labels, 220.0).and_then(|i| KINDS.get(i)) {
                    d.fields.insert("kind".into(), json!(kind));
                }
                ui.end_row();
            }
            "variable.bind" => {
                label(ui, tl!("Variable"));
                dropdown(ui, d, "variable", &variable_names, 220.0);
                ui.end_row();
            }
            "dataset.select" => {
                label(ui, tl!("Data Set"));
                dropdown(ui, d, "name", &dataset_names, 220.0);
                ui.end_row();
            }
            // A rename: which row, and its new name.
            "variable.rename" | "dataset.rename" => {
                let (key, label_text, names) = if d.str("__command") == "variable.rename" {
                    ("variable", tl!("Variable").to_string(), &variable_names)
                } else {
                    ("name", tl!("Data Set").to_string(), &dataset_names)
                };
                label(ui, &label_text);
                dropdown(ui, d, key, names, 220.0);
                ui.end_row();
                label(ui, tl!("New Name"));
                text(ui, d, "newName", 220.0);
                ui.end_row();
            }
            _ => {
                label(ui, tl!("Name"));
                text(ui, d, "name", 220.0);
                ui.end_row();
                if vars.variables.is_empty() {
                    widgets::dim_label(ui, tl!("Define a variable first"));
                    ui.end_row();
                    return;
                }
                // One row per variable, in the order they are defined, each with the editor
                // its kind takes.
                for v in &vars.variables {
                    label(ui, &v.name);
                    let key = value_key(&v.name);
                    match v.kind {
                        VariableKind::Text => {
                            text(ui, d, &key, 220.0);
                        }
                        VariableKind::Visibility => checkbox(ui, d, &key),
                    }
                    ui.end_row();
                }
            }
        }
    });
    false
}

/// A text field for `key`, written back as it is typed.
fn text(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) -> egui::Response {
    let mut s = d.str(key);
    let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(width));
    if r.changed() {
        d.fields.insert(key.into(), json!(s));
    }
    r
}

/// A checkbox for `key`, written back as it is toggled.
fn checkbox(ui: &mut egui::Ui, d: &mut Dialog, key: &str) {
    let b = d.bool(key);
    if widgets::check(ui, "", b, true) {
        d.fields.insert(key.into(), json!(!b));
    }
}

/// A dropdown over the document's names `options` (shown as they are, never translated) for
/// `key`, written back as it changes. The empty option is first so a fresh dialog can still be
/// filled in; a name that is no longer offered reads as empty rather than being silently
/// swapped for another one.
fn dropdown(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[&str], width: f32) {
    let cur = d.str(key);
    let mut labels: Vec<&str> = vec![""];
    labels.extend_from_slice(options);
    let shown = if cur.is_empty() || options.contains(&cur.as_str()) { cur.as_str() } else { "" };
    if let Some(i) = widgets::dropdown_names(ui, ("variables-dd", key), shown, &labels, width) {
        // The empty option (0) empties the field; the rest are `options`, one along.
        d.fields.insert(key.into(), json!(i.checked_sub(1).and_then(|i| options.get(i)).copied().unwrap_or_default()));
    }
}

/// The `values` object the dialog's fields mean: a text variable whose field is empty carries
/// no value in this row, and a visibility checkbox is always a value — its false, too.
fn values_of(d: &Dialog, vars: &Variables) -> Value {
    let mut out = serde_json::Map::new();
    for v in &vars.variables {
        let key = value_key(&v.name);
        if v.kind == VariableKind::Text {
            let text = d.str(&key);
            if !text.is_empty() {
                out.insert(v.name.clone(), json!(text));
            }
        } else {
            out.insert(v.name.clone(), json!(d.bool(&key)));
        }
    }
    Value::Object(out)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let cmd = d.str("__command");
    let name = d.str("name");
    let p = match cmd.as_str() {
        "variable.define" => json!({"name": name, "kind": d.str("kind")}),
        "variable.bind" => json!({"variable": d.str("variable")}),
        "variable.rename" => json!({"name": d.str("variable"), "newName": d.str("newName")}),
        "dataset.rename" => json!({"name": name, "newName": d.str("newName")}),
        "dataset.new" | "dataset.set" => {
            let values = app.session.active().map(|st| values_of(d, &st.doc.variables)).unwrap_or_else(|| json!({}));
            json!({"name": name, "values": values})
        }
        "dataset.select" => json!({"name": name}),
        _ => json!({}),
    };
    run_and_close(app, &cmd, p)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    fn app_with_variables() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("variable.define", json!({"name": "Title", "kind": "text"})).unwrap();
        app.run("variable.define", json!({"name": "Badge", "kind": "visibility"})).unwrap();
        app
    }

    #[test]
    fn defining_a_variable_from_the_dialog_works() {
        let mut app = app_with_variables();
        assert!(open(&mut app, "variable.define", None));
        {
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!(d.kind, KIND);
            assert_eq!(d.str("name"), "");
            assert_eq!(d.str("kind"), "text", "text is the default kind");
        }
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Price"));
        crate::dialogs::confirm(&mut app).unwrap();
        let vars = app.run("variable.list", json!({})).unwrap();
        assert!(vars["variables"].as_array().unwrap().iter().any(|v| v["name"] == "Price"));
    }

    #[test]
    fn a_new_dataset_carries_the_dialogs_values_and_omits_the_empty_ones() {
        let mut app = app_with_variables();
        open(&mut app, "dataset.new", None);
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("name".into(), json!("Row 1"));
            d.fields.insert(value_key("Title"), json!("Hello"));
            d.fields.insert(value_key("Badge"), json!(false));
        }
        crate::dialogs::confirm(&mut app).unwrap();
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["name"], "Row 1");
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": "Hello"}));
        assert_eq!(sets["datasets"][0]["values"]["Badge"], json!({"visible": false}));
    }

    #[test]
    fn editing_a_dataset_opens_with_the_rows_own_values() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hello", "Badge": false}})).unwrap();
        app.run("dataset.set", json!({"name": "Row 1", "values": {"Badge": true}})).unwrap();
        open(&mut app, "dataset.set", None);
        {
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!(d.str("name"), "Row 1");
            assert_eq!(d.str(&value_key("Title")), "", "the row carries no value for it");
            assert!(d.bool(&value_key("Badge")), "the row's own value");
        }
        app.ui.dialog.as_mut().unwrap().fields.insert(value_key("Title"), json!("Changed"));
        crate::dialogs::confirm(&mut app).unwrap();
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": "Changed"}));
        assert_eq!(sets["datasets"][0]["values"]["Badge"], json!({"visible": true}));
    }

    #[test]
    fn selecting_a_dataset_from_the_dialog_applies_it() {
        let mut app = app_with_variables();
        let id = app.run("text.create", json!({"x": 10, "y": 10, "text": "x"})).unwrap()["id"].as_u64().unwrap();
        app.run("variable.bind", json!({"variable": "Title", "ids": [id]})).unwrap();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hi"}})).unwrap();
        open(&mut app, "dataset.select", None);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Row 1"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.run("text.getRange", json!({"id": id})).unwrap()["text"], "Hi");
    }

    #[test]
    fn only_the_variables_commands_open_one() {
        let mut app = app_with_variables();
        assert!(opens("dataset.set") && opens("variable.define"));
        assert!(!opens("dataset.list") && !opens("dataset.next"), "no dialog for a query or a step");
        assert!(!open(&mut app, "dataset.list", None), "and it opens nothing");
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn rename_dialogs_start_on_the_preselected_row() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hello"}})).unwrap();
        open(&mut app, "variable.rename", Some("Badge"));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("variable"), "Badge");
        // A preselect the document no longer has falls back to the first row.
        open(&mut app, "variable.rename", Some("Gone"));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("variable"), "Title");
        open(&mut app, "dataset.rename", Some("Row 1"));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "Row 1");
    }
}
