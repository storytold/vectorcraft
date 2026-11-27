//! The Variables dialogs (`variable.define`, `variable.rename` as Variable Options, `variable.bind`,
//! `dataset.new`, `dataset.set`, `dataset.rename` and `dataset.select`), opened from Window ›
//! Variables or the command palette. Variable Options shows the variable's name and kind, and OK
//! changes either or both.
//!
//! One dialog covers all of them: what it shows is what the command takes. A variable is a
//! name and a kind; a dataset is a name and one field per variable — a text field for a
//! `text`, `image` (a path) or `graph` (CSV) variable, a checkbox for a `visibility` one.
//! `dataset.set` opens with the row's current values, so editing a row is editing what it
//! already says, and its Name field renames the row. The data set dialogs stay open when OK
//! fails, keeping what was typed.

use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_doc::{DataValue, VariableKind, Variables};

use crate::VectorcraftApp;

/// The host key of every Variables dialog: one dialog at a time, so opening another command
/// replaces the open one.
pub(crate) const KEY: &str = "variables";

/// The field of Edit Data Set that holds the name of the row it edits, which its Name field
/// renames.
const EDITED: &str = "__dataset";

/// The kinds a variable can have, as `variable.define` names them.
const KINDS: [&str; 4] = ["text", "visibility", "image", "graph"];

/// What the dialog shows for each of [`KINDS`].
fn kind_labels() -> [&'static str; 4] {
    [tl!("Text"), tl!("Visibility"), tl!("Image"), tl!("Graph")]
}

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
/// one. Variable Options edits the one the panel has highlighted, so it starts there.
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
    let document = st.uid;
    let vars = st.doc.variables.clone();
    let mut fields = DialogFields::new();
    match id {
        "variable.define" => {
            fields.set("name", "");
            fields.set("kind", KINDS[0]);
        }
        "variable.bind" => fields.set("variable", ""),
        "dataset.new" => {
            fields.set("name", "");
            value_fields(&vars, vars.datasets.first().map(|d| &d.values), &mut fields);
        }
        "dataset.set" => {
            // Without a row to edit there is nothing to open.
            let Some(name) = vars.active_dataset.clone().or_else(|| vars.datasets.first().map(|d| d.name.clone())) else {
                app.status(tl!("no data set to edit").to_string());
                return true;
            };
            let Some(current) = vars.dataset(&name) else { return true };
            fields.set("name", name.as_str());
            // The row being edited; the Name field renames it.
            fields.set(EDITED, name.as_str());
            value_fields(&vars, Some(&current.values), &mut fields);
            // An empty text value is a value, but its field looks like a field with no value:
            // mark it so OK keeps it until the field is edited.
            for (var, value) in &current.values {
                if matches!(value, DataValue::Text(t) if t.is_empty()) {
                    fields.set(empty_marker(&value_key(var)), true);
                }
            }
        }
        "dataset.select" => fields.set("name", ""),
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
            let row = preselect.filter(|p| names.contains(p)).or(names.first().copied()).unwrap_or_default();
            fields.set(key, row);
            // Variable Options' Name and Kind rows default to the variable's own.
            if key != "variable" {
                fields.set("newName", "");
            }
        }
    }
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status(tl!("no window").to_string());
        return true;
    };
    let (command, title) = (id.to_string(), tl!(label).to_string());
    let build_id = command.clone();
    let dialog = HostedParamDialog::new(&ctx, KEY, fields, move |fields| {
        let rows = params(&vars, &build_id, fields);
        ParamDialog::command(KEY, Label::from(title.clone()), rows)
    });
    crate::dialogs::host::open_held(app, dialog, move |app, result| ok(app, document, &command, result));
    true
}

/// Runs `command` with what the confirmed `result` says. A data set dialog answers the error, which
/// keeps it open with what was typed (a taken or empty name); any other command's error goes to the
/// status line and the dialog closes.
fn ok(app: &mut VectorcraftApp, document: u64, command: &str, result: &ParamResult) -> Result<(), String> {
    let Some(st) = app.session.active().filter(|st| st.uid == document) else {
        return Err(tl!("These changes belong to the document this dialog was opened for.").to_owned());
    };
    let params = params_of(command, result, &st.doc.variables);
    let Err(e) = app.run(command, params) else { return Ok(()) };
    if matches!(command, "dataset.new" | "dataset.set") {
        return Err(e);
    }
    app.status(e);
    Ok(())
}

/// Variable Options' Name and Kind defaults for the variable `name`: its name, and its kind as
/// `variable.rename` takes it (empty for a variable of unknown kind, which OK leaves as it is).
fn defaults_of(vars: &Variables, name: &str) -> (String, &'static str) {
    let kind = vars.variable(name).map(|v| v.kind).filter(|k| *k != VariableKind::Unknown).map(VariableKind::label).unwrap_or_default();
    (name.to_string(), kind)
}

/// Whether `id` opens one of these dialogs.
pub fn opens(id: &str) -> bool {
    opened(id).is_some()
}

/// One field per variable, prefilled from `current` (the row being edited, else the first row
/// so a new one starts from something rather than from nothing).
fn value_fields(vars: &Variables, current: Option<&std::collections::BTreeMap<String, DataValue>>, out: &mut DialogFields) {
    for v in &vars.variables {
        let key = value_key(&v.name);
        match (current.and_then(|c| c.get(&v.name)), v.kind) {
            (Some(DataValue::Text(t)), _) => out.set(key, t.to_string()),
            (Some(DataValue::Visible(b)), _) => out.set(key, *b),
            (Some(DataValue::Image(p)), _) => out.set(key, p.to_string()),
            (Some(DataValue::Graph(c)), _) => out.set(key, c.to_string()),
            // Without a value in the row a text, image or graph field starts empty (no value in
            // this row) and a visibility one starts shown: its false is a value, so it is never
            // "unset".
            (None, VariableKind::Text | VariableKind::Image | VariableKind::Graph) => out.set(key, ""),
            (None, VariableKind::Visibility) => out.set(key, true),
            // A variable of unknown kind takes no value, so it has no field.
            (None, VariableKind::Unknown) => {}
        }
    }
}

/// A variable name as a field key. A field key shares the dialog's flat namespace with the
/// command's own parameters, so names are prefixed to stay clear of `name` and `values`.
fn value_key(name: &str) -> String {
    format!("value:{name}")
}

/// The key that marks the field `key` as holding the edited row's empty text value. Editing the
/// field removes the mark.
fn empty_marker(key: &str) -> String {
    format!("__empty:{key}")
}

/// A dropdown row over the document's names (shown as they are, never translated). The empty
/// option is first so a fresh dialog can still be filled in.
fn names_row(key: &str, label: &'static str, names: Vec<&str>) -> Param {
    let options =
        std::iter::once((String::new(), Label::from(""))).chain(names.into_iter().map(|n| (n.to_string(), Label::from(n.to_string())))).collect();
    Param::new(key, Label::from(label), ParamKind::choice(options, None))
}

/// A text row.
fn text_row(key: &str, label: &'static str, default: &str) -> Param {
    Param::new(key, Label::from(label), ParamKind::text(default))
}

/// The Kind row: the kinds by their translated names, while the field keeps the command's own word.
fn kind_row(default: Option<String>) -> Param {
    let options = KINDS.iter().zip(kind_labels()).map(|(k, l)| (k.to_string(), Label::from(l))).collect();
    Param::new("kind", Label::from(tl!("Kind")), ParamKind::choice(options, default))
}

/// The rows of `id`'s dialog over the document's `vars` and the dialog's current `fields`.
fn params(vars: &Variables, id: &str, fields: &DialogFields) -> Vec<Param> {
    let variables: Vec<&str> = vars.variables.iter().map(|v| v.name.as_str()).collect();
    let datasets: Vec<&str> = vars.datasets.iter().map(|d| d.name.as_str()).collect();
    match id {
        "variable.define" => vec![text_row("name", tl!("Name"), ""), kind_row(Some(KINDS[0].to_string()))],
        // Variable Options: which variable, and its name and kind; picking another variable
        // starts the two rows over from that one.
        "variable.rename" => {
            let (name, kind) = defaults_of(vars, fields.text("variable").unwrap_or_default());
            vec![
                names_row("variable", tl!("Variable"), variables).resets(["newName", "kind"]),
                text_row("newName", tl!("Name"), &name),
                kind_row((!kind.is_empty()).then(|| kind.to_string())),
            ]
        }
        "variable.bind" => vec![names_row("variable", tl!("Variable"), variables)],
        "dataset.select" => vec![names_row("name", tl!("Data Set"), datasets)],
        "dataset.rename" => vec![names_row("name", tl!("Data Set"), datasets), text_row("newName", tl!("New Name"), "")],
        _ => {
            let mut rows = vec![text_row("name", tl!("Name"), "")];
            if vars.variables.is_empty() {
                rows.push(Param::read_only_text("note", "", tl!("Define a variable first")));
                return rows;
            }
            // One row per variable, in the order they are defined, each with the editor its kind takes.
            for v in &vars.variables {
                let key = value_key(&v.name);
                let label = Label::from(v.name.clone());
                match v.kind {
                    VariableKind::Text | VariableKind::Image => {
                        rows.push(Param::new(key.as_str(), label, ParamKind::text("")).resets([empty_marker(&key)]));
                    }
                    VariableKind::Graph => rows.push(Param::new(key.as_str(), label, ParamKind::text("")).multiline(4)),
                    VariableKind::Visibility => rows.push(Param::new(key.as_str(), label, ParamKind::bool(true))),
                    VariableKind::Unknown => {}
                }
            }
            rows
        }
    }
}

/// The `values` object the dialog's fields mean: a text, image or graph variable whose field is
/// empty carries no value in this row, except a text variable whose value in the edited row was
/// already the empty text, which keeps it. A visibility checkbox is always a value — its false,
/// too.
fn values_of(fields: &DialogFields, vars: &Variables) -> Value {
    let mut out = serde_json::Map::new();
    for v in &vars.variables {
        let key = value_key(&v.name);
        match v.kind {
            VariableKind::Text | VariableKind::Image | VariableKind::Graph => {
                let text = fields.text(&key).unwrap_or_default();
                if !text.is_empty() || (v.kind == VariableKind::Text && fields.bool(&empty_marker(&key)).unwrap_or(false)) {
                    out.insert(v.name.clone(), json!(text));
                }
            }
            VariableKind::Visibility => {
                out.insert(v.name.clone(), json!(fields.bool(&key).unwrap_or(false)));
            }
            VariableKind::Unknown => {}
        }
    }
    Value::Object(out)
}

/// The params of `command` for the confirmed `result`.
fn params_of(command: &str, result: &ParamResult, vars: &Variables) -> Value {
    let f = &result.fields;
    let text = |key: &str| f.text(key).unwrap_or_default().to_string();
    let name = text("name");
    match command {
        "variable.define" => json!({"name": name, "kind": text("kind")}),
        "variable.bind" => json!({"variable": text("variable")}),
        // A Name the Variable field just reset keeps the variable's name (an emptied one is sent,
        // and refused), and a reset or empty Kind (a variable of unknown kind) keeps its kind.
        "variable.rename" => {
            let variable = text("variable");
            let new_name = f.text("newName").map_or_else(|| defaults_of(vars, &variable).0, str::to_string);
            match f.text("kind").filter(|k| !k.is_empty()) {
                // Any kind word goes to the command, which refuses one it doesn't know.
                Some(kind) => json!({"name": variable, "newName": new_name, "kind": kind}),
                None => json!({"name": variable, "newName": new_name}),
            }
        }
        "dataset.rename" => json!({"name": name, "newName": text("newName")}),
        "dataset.new" => json!({"name": name, "values": values_of(f, vars)}),
        "dataset.set" => json!({"name": text(EDITED), "newName": name, "values": values_of(f, vars)}),
        "dataset.select" => json!({"name": name}),
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{DialogId, StackDialog};
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    fn app_with_variables() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        // PLAN: assumed `crate::testing::ui_context() -> egui::Context` (hub-w1, S-SH4) and `ui_ctx: Option<egui::Context>`
        app = crate::testing::windowed(app);
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("variable.define", json!({"name": "Title", "kind": "text"})).unwrap();
        app.run("variable.define", json!({"name": "Badge", "kind": "visibility"})).unwrap();
        app
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().unwrap()
    }

    /// The id of the open dialog.
    fn open_id(app: &VectorcraftApp) -> DialogId {
        app.dialogs().unwrap().with(|h| h.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(KEY)).map(|d| d.id)).unwrap()
    }

    /// Sets `key` on the open dialog the way `ui.dialog.set` does.
    fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
        let (ctx, id) = (ctx(app), open_id(app));
        app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, key, value)).unwrap();
    }

    /// The open dialog's fields.
    fn fields(app: &VectorcraftApp) -> Value {
        let id = open_id(app);
        app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).and_then(|d| d.json_fields())).unwrap()
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|h| h.open_dialogs().iter().any(|d| d.key.as_deref() == Some(KEY)))
    }

    /// Presses OK on the open dialog, runs the host pass that answers it and the frame that closes it.
    fn ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (ctx(app), open_id(app));
        app.dialogs().unwrap().with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        crate::testing::app_frame(app, &ctx, Default::default());
    }

    fn kind_of(app: &VectorcraftApp, name: &str) -> Option<VariableKind> {
        app.session.active().and_then(|st| st.doc.variables.variable(name).map(|v| v.kind))
    }

    #[test]
    fn defining_a_variable_from_the_dialog_works() {
        let mut app = app_with_variables();
        assert!(open(&mut app, "variable.define", None));
        assert!(is_open(&app));
        let f = fields(&app);
        assert_eq!(f["name"], "");
        assert_eq!(f["kind"], "text", "text is the default kind");
        set(&mut app, "name", json!("Price"));
        ok(&mut app);
        let vars = app.run("variable.list", json!({})).unwrap();
        assert!(vars["variables"].as_array().unwrap().iter().any(|v| v["name"] == "Price"));
    }

    #[test]
    fn a_new_dataset_carries_the_dialogs_values_and_omits_the_empty_ones() {
        let mut app = app_with_variables();
        open(&mut app, "dataset.new", None);
        set(&mut app, "name", json!("Row 1"));
        set(&mut app, &value_key("Title"), json!("Hello"));
        set(&mut app, &value_key("Badge"), json!(false));
        ok(&mut app);
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["name"], "Row 1");
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": "Hello"}));
        assert_eq!(sets["datasets"][0]["values"]["Badge"], json!({"visible": false}));
    }

    #[test]
    fn an_untouched_edit_keeps_an_empty_text_value() {
        let mut app = app_with_variables();
        let id = app.run("text.create", json!({"x": 10, "y": 10, "text": "x"})).unwrap()["id"].as_u64().unwrap();
        app.run("variable.bind", json!({"variable": "Title", "ids": [id]})).unwrap();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "", "Badge": true}})).unwrap();
        app.run("dataset.new", json!({"name": "Row 2", "values": {"Title": "Bob", "Badge": true}})).unwrap();
        app.run("dataset.select", json!({"name": "Row 1"})).unwrap();
        open(&mut app, "dataset.set", None);
        assert_eq!(fields(&app)[value_key("Title")], "");
        ok(&mut app);
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": ""}), "OK with nothing changed keeps the empty text");
        app.run("dataset.select", json!({"name": "Row 2"})).unwrap();
        app.run("dataset.select", json!({"name": "Row 1"})).unwrap();
        assert_eq!(app.run("text.getRange", json!({"id": id})).unwrap()["text"], "", "the row still empties the text");
        // A field the user empties on a row that had a value still leaves the variable unset.
        app.run("dataset.select", json!({"name": "Row 2"})).unwrap();
        open(&mut app, "dataset.set", None);
        set(&mut app, &value_key("Title"), json!(""));
        ok(&mut app);
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][1]["values"]["Title"], json!(null));
        // Typing into the empty text value's field and emptying it again unsets it too.
        app.run("dataset.set", json!({"name": "Row 1", "values": {"Title": "", "Badge": true}})).unwrap();
        app.run("dataset.select", json!({"name": "Row 1"})).unwrap();
        open(&mut app, "dataset.set", None);
        for s in ["a", ""] {
            set(&mut app, &value_key("Title"), json!(s));
        }
        ok(&mut app);
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!(null));
    }

    #[test]
    fn editing_a_dataset_opens_with_the_rows_own_values() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hello", "Badge": false}})).unwrap();
        app.run("dataset.set", json!({"name": "Row 1", "values": {"Badge": true}})).unwrap();
        open(&mut app, "dataset.set", None);
        let f = fields(&app);
        assert_eq!(f["name"], "Row 1");
        assert_eq!(f[value_key("Title")], "", "the row carries no value for it");
        assert_eq!(f[value_key("Badge")], true, "the row's own value");
        set(&mut app, &value_key("Title"), json!("Changed"));
        ok(&mut app);
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": "Changed"}));
        assert_eq!(sets["datasets"][0]["values"]["Badge"], json!({"visible": true}));
    }

    #[test]
    fn a_data_set_edit_stays_with_the_drawing_that_opened_it() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Source"}})).unwrap();
        app.run("file.new", json!({"width": 240, "height": 220})).unwrap();
        app.run("variable.define", json!({"name": "Title", "kind": "text"})).unwrap();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Other"}})).unwrap();
        let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
        let other = app.run("dataset.list", json!({})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();

        activate(&mut app, 0);
        assert!(open(&mut app, "dataset.set", None));
        set(&mut app, "name", json!("Revised"));
        set(&mut app, &value_key("Title"), json!("New value"));
        activate(&mut app, 1);
        ok(&mut app);

        assert_eq!(app.run("dataset.list", json!({})).unwrap(), other);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(is_open(&app));
        assert_eq!(fields(&app)[value_key("Title")], "New value");

        activate(&mut app, 0);
        ok(&mut app);
        assert!(!is_open(&app));
        let source = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(source["datasets"][0]["name"], "Revised");
        assert_eq!(source["datasets"][0]["values"]["Title"], json!({"text": "New value"}));
    }

    #[test]
    fn edit_data_sets_name_field_renames_the_row_it_edits() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hello"}})).unwrap();
        app.run("dataset.new", json!({"name": "Row 2", "values": {"Title": "Other"}})).unwrap();
        app.run("dataset.select", json!({"name": "Row 1"})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        open(&mut app, "dataset.set", None);
        set(&mut app, "name", json!("Spring"));
        set(&mut app, &value_key("Title"), json!("Changed"));
        ok(&mut app);
        assert!(!is_open(&app));
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!((sets["datasets"][0]["name"].clone(), sets["active"].clone()), (json!("Spring"), json!("Spring")));
        assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": "Changed"}));
        assert_eq!(sets["datasets"][1]["values"]["Title"], json!({"text": "Other"}), "the other row is untouched");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");

        // Another row's name, or no name, is refused: neither row changes and the dialog keeps
        // what was typed.
        for taken in ["Row 2", " "] {
            open(&mut app, "dataset.set", None);
            set(&mut app, "name", json!(taken));
            set(&mut app, &value_key("Title"), json!("Meant for Spring"));
            ok(&mut app);
            assert!(is_open(&app), "{taken:?}");
            assert_eq!(fields(&app)[value_key("Title")], "Meant for Spring");
            let sets = app.run("dataset.list", json!({})).unwrap();
            assert_eq!(sets["datasets"][0]["values"]["Title"], json!({"text": "Changed"}));
            assert_eq!(sets["datasets"][1]["values"]["Title"], json!({"text": "Other"}));
        }
    }

    #[test]
    fn new_data_set_keeps_its_values_when_the_name_is_taken() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hello"}})).unwrap();
        open(&mut app, "dataset.new", None);
        set(&mut app, "name", json!("Row 1"));
        set(&mut app, &value_key("Title"), json!("Typed"));
        ok(&mut app);
        assert!(is_open(&app));
        assert_eq!(fields(&app)[value_key("Title")], "Typed");
        assert_eq!(app.run("dataset.list", json!({})).unwrap()["datasets"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn rename_dialogs_start_on_the_preselected_row() {
        let mut app = app_with_variables();
        app.run("dataset.new", json!({"name": "Row 1", "values": {"Title": "Hello"}})).unwrap();
        open(&mut app, "variable.rename", Some("Badge"));
        assert_eq!(fields(&app)["variable"], "Badge");
        // A preselect the document no longer has falls back to the first row.
        open(&mut app, "variable.rename", Some("Gone"));
        assert_eq!(fields(&app)["variable"], "Title");
        open(&mut app, "dataset.rename", Some("Row 1"));
        assert_eq!(fields(&app)["name"], "Row 1");
    }

    #[test]
    fn variable_options_shows_the_name_and_kind_and_changes_either() {
        let mut app = app_with_variables();
        open(&mut app, "variable.rename", Some("Badge"));
        let f = fields(&app);
        assert_eq!((f["newName"].clone(), f["kind"].clone()), (json!("Badge"), json!("visibility")));
        set(&mut app, "kind", json!("text"));
        ok(&mut app);
        assert_eq!(kind_of(&app, "Badge"), Some(VariableKind::Text));

        // OK with nothing changed adds no undo step, and an emptied Name is refused.
        let undo = app.session.active().unwrap().history.undo.len();
        open(&mut app, "variable.rename", Some("Title"));
        ok(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        open(&mut app, "variable.rename", Some("Title"));
        set(&mut app, "newName", json!(""));
        ok(&mut app);
        assert!(kind_of(&app, "Title").is_some(), "Title keeps its name");

        // Picking another variable keeps that variable's own name and kind until Name or Kind is set.
        open(&mut app, "variable.rename", Some("Title"));
        set(&mut app, "variable", json!("Badge"));
        ok(&mut app);
        assert_eq!((kind_of(&app, "Title"), kind_of(&app, "Badge")), (Some(VariableKind::Text), Some(VariableKind::Text)));
        open(&mut app, "variable.rename", Some("Title"));
        set(&mut app, "variable", json!("Badge"));
        set(&mut app, "newName", json!("Flag"));
        set(&mut app, "kind", json!("visibility"));
        ok(&mut app);
        assert_eq!((kind_of(&app, "Flag"), kind_of(&app, "Title")), (Some(VariableKind::Visibility), Some(VariableKind::Text)));
    }

    #[test]
    fn variable_options_fills_the_field_left_unset_after_the_variable_changes() {
        // Kind alone: Name is the newly picked variable's name.
        let mut app = app_with_variables();
        open(&mut app, "variable.rename", Some("Title"));
        set(&mut app, "variable", json!("Badge"));
        set(&mut app, "kind", json!("text"));
        ok(&mut app);
        assert_eq!((kind_of(&app, "Badge"), kind_of(&app, "Title")), (Some(VariableKind::Text), Some(VariableKind::Text)));

        // Name alone: Kind is the newly picked variable's kind, and OK renames Badge keeping it.
        let mut app = app_with_variables();
        open(&mut app, "variable.rename", Some("Title"));
        set(&mut app, "variable", json!("Badge"));
        set(&mut app, "newName", json!("Flag"));
        ok(&mut app);
        assert_eq!((kind_of(&app, "Flag"), kind_of(&app, "Badge")), (Some(VariableKind::Visibility), None));
    }
}
