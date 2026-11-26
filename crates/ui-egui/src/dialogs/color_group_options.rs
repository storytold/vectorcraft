//! Color Group Options: the name of a color group in the Swatches panel. OK renames the group
//! (`swatch.renameGroup`) and keeps it selected in the panel; its swatches and their colors stay
//! as they are.
//!
//! Fields: `name`. The group being renamed (its name, its place in the list of color groups, the names of
//! its swatches and the uid of its document when the dialog opened) is kept by the dialog's OK.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind, ParamResult};
use serde_json::{Value, json};

use super::swatch_options::{NO_WINDOW, field_label, hosted};
use crate::VectorcraftApp;

/// The dialog kind of Color Group Options.
pub const KIND: &str = "colorGroupOptions";

/// A color group as the dialog found it: what tells it apart from the others, since color groups carry nothing else.
struct Identity {
    /// The uid of the document the dialog opened on.
    doc: u64,
    /// The group's place in the document's list of color groups.
    at: usize,
    /// The group's name.
    group: String,
    /// The names of the group's swatches.
    swatches: Vec<String>,
}

/// `ui.colorGroupOptions {group?}`: open Color Group Options for color group `group`, or without
/// `group` for the color group selected alone in the Swatches panel.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "group";
    let group = match p.get(key) {
        Some(Value::String(g)) => g.clone(),
        None | Some(Value::Null) => {
            crate::panels::swatches::sole_selected_group(app).ok_or("select a color group in the Swatches panel, or pass group")?
        }
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let st = app.session.active().ok_or("no document open")?;
    let at = st.doc.swatch_groups.iter().position(|g| g.name == group).ok_or_else(|| format!("no colour group `{group}`"))?;
    let g = &st.doc.swatch_groups[at];
    let identity = Identity { doc: st.uid, at, group: g.name.clone(), swatches: g.swatches.iter().map(|w| w.name.clone()).collect() };
    let mut fields = DialogFields::new();
    fields.set("name", g.name.as_str());
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    let rows = vec![Param::new("name", field_label(tl!("Name:")), ParamKind::text(""))];
    let dialog = hosted(&ctx, KIND, tl!("Color Group Options"), fields, rows, false);
    let ok = move |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        // Another document or group: it closes as Cancel does and renames nothing.
        if !same_group(app, &identity) {
            return Ok(());
        }
        let name = answer.fields.text("name").unwrap_or_default();
        // The Swatches panel keeps the renamed group selected, [`crate::panels::swatches::after_command`].
        app.run("swatch.renameGroup", json!({ "group": identity.group, "name": name })).map(|_| ())
    };
    super::host::open_held(app, dialog, ok).ok_or(NO_WINDOW)?;
    Ok(Value::Null)
}

/// Whether the dialog's group is still in its document, at the place in the list, under the name
/// and with the swatches it opened with, and that document is the active one.
fn same_group(app: &VectorcraftApp, identity: &Identity) -> bool {
    let Some(st) = app.session.active().filter(|st| st.uid == identity.doc) else { return false };
    st.doc
        .swatch_groups
        .get(identity.at)
        .is_some_and(|g| g.name == identity.group && g.swatches.iter().map(|w| w.name.as_str()).eq(identity.swatches.iter().map(String::as_str)))
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::*;

    /// An app with a window for its dialogs and one open document.
    fn app() -> (egui::Context, VectorcraftApp) {
        let ctx = crate::testing::ui_context();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        (ctx, app)
    }

    /// Types `name` into the open dialog and presses OK.
    fn ok(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
        let host = app.dialogs().unwrap();
        host.with(|h| h.set_json_field(ctx, dialog_id(KIND), "name", json!(name))).unwrap();
        host.with(|h| h.confirm_by_id(ctx, dialog_id(KIND)));
        crate::dialogs::host::pump(app, ctx);
    }

    fn groups(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.swatch_groups.iter().map(|g| g.name.clone()).collect()
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// Whether the dialog is still open.
    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|h| h.get_by_id::<craft::ui::patterns::form_dialogs::HostedParamDialog>(dialog_id(KIND)).is_some())
    }

    /// OK renames the group in one undo step, a taken or empty name keeps the dialog open with nothing changed,
    /// Cancel changes nothing, and OK with the name unchanged records no undo step.
    #[test]
    fn renames_the_group_and_keeps_a_refused_name_open() {
        let (ctx, mut app) = app();
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        let undo = undo_len(&app);
        for taken in ["Grays", "White", "  "] {
            ok(&mut app, &ctx, taken);
            assert!(is_open(&app), "{taken:?} keeps the dialog open");
            assert_eq!(undo_len(&app), undo, "{taken:?}");
        }
        app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, dialog_id(KIND)));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(groups(&app).contains(&"Brights".to_string()), "Cancel keeps the name");
        assert_eq!(undo_len(&app), undo);
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        ok(&mut app, &ctx, "Brights");
        assert_eq!(undo_len(&app), undo, "the same name records no undo step");
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        ok(&mut app, &ctx, "Vivid");
        assert!(groups(&app).contains(&"Vivid".to_string()));
        assert_eq!(undo_len(&app), undo + 1);
        assert!(app.run("ui.colorGroupOptions", json!({"group": "Brights"})).is_err());
        assert!(app.run("ui.colorGroupOptions", json!({"group": "White"})).is_err(), "a swatch is not a group");
        assert!(app.run("ui.colorGroupOptions", json!({})).is_err());
        assert!(app.run("ui.colorGroupOptions", json!({"group": 5})).is_err());
    }

    /// OK renames nothing when another document has become the active one, or when another group took the name of
    /// the group it opened on while it was open.
    #[test]
    fn renames_only_the_group_it_opened_on() {
        let (ctx, mut app) = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        app.run("document.activate", json!({"index": 1})).unwrap();
        ok(&mut app, &ctx, "Vivid");
        assert!(groups(&app).contains(&"Brights".to_string()), "the document made active keeps its group");
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(groups(&app).contains(&"Brights".to_string()), "the group it opened on keeps its name");
        // Its group ungrouped, and another group renamed to its name.
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        app.run("swatch.ungroup", json!({"name": "Brights"})).unwrap();
        app.run("swatch.renameGroup", json!({"group": "Grays", "name": "Brights"})).unwrap();
        ok(&mut app, &ctx, "Vivid");
        assert!(groups(&app).contains(&"Brights".to_string()) && !groups(&app).contains(&"Vivid".to_string()));
        // Its group moved to another place in the list.
        app.run("swatch.newGroup", json!({"name": "Extra", "swatches": ["White"]})).unwrap();
        let first = groups(&app)[0].clone();
        app.run("ui.colorGroupOptions", json!({"group": first})).unwrap();
        app.run("swatch.move", json!({"names": [first]})).unwrap();
        ok(&mut app, &ctx, "Vivid");
        assert!(groups(&app).contains(&first) && !groups(&app).contains(&"Vivid".to_string()));
    }
}
