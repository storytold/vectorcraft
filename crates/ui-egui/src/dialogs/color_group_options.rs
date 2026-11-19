//! Color Group Options: the name of a color group in the Swatches panel. OK renames the group
//! (`swatch.renameGroup`) and keeps it selected in the panel; its swatches and their colors stay
//! as they are.
//!
//! Fields: `name`, `__group` (the group being renamed), `__swatches` (the names of its swatches
//! when the dialog opened), `__index` (its place in the list of color groups) and `__doc` (the uid
//! of its document).

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Color Group Options.
pub const KIND: &str = "colorGroupOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Color Group Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// The field that holds the uid of the document the dialog opened on.
const DOC: &str = "__doc";

/// The field that holds the names of the group's swatches when the dialog opened.
const SWATCHES: &str = "__swatches";

/// The field that holds the group's place in the document's list of color groups.
const INDEX: &str = "__index";

/// `ui.colorGroupOptions {group?}`: open Color Group Options for color group `group`, or without
/// `group` for the color group selected alone in the Swatches panel.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "group";
    let group = match p.get(key) {
        Some(Value::String(g)) => g.clone(),
        None | Some(Value::Null) => crate::panels::swatches::sole_selected_group(app).ok_or("missing `group`")?,
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let st = app.session.active().ok_or("no document open")?;
    let at = st.doc.swatch_groups.iter().position(|g| g.name == group).ok_or_else(|| format!("no colour group `{group}`"))?;
    let g = &st.doc.swatch_groups[at];
    let swatches: Vec<&str> = g.swatches.iter().map(|w| w.name.as_str()).collect();
    let fields = json!({ "name": g.name.clone(), "__group": g.name.clone(), SWATCHES: swatches, INDEX: at, DOC: st.uid });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// Whether the dialog's group is still in its document, at the place in the list, under the name
/// and with the swatches it opened with, and that document is the active one.
fn same_group(app: &VectorcraftApp, d: &Dialog) -> bool {
    let Some(st) = app.session.active().filter(|st| d.fields.get(DOC).and_then(Value::as_u64) == Some(st.uid)) else { return false };
    let at = d.fields.get(INDEX).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let opened: Vec<&str> =
        d.fields.get(SWATCHES).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let group = d.str("__group");
    at.and_then(|i| st.doc.swatch_groups.get(i))
        .is_some_and(|g| g.name == group && g.swatches.iter().map(|w| w.name.as_str()).eq(opened.iter().copied()))
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Name:"));
        form::text(ui, d, "name", 190.0);
        ui.end_row();
    });
    false
}

/// OK: rename the group (the Swatches panel keeps it selected, [`crate::panels::swatches::after_command`]).
/// A name that is empty or that another group or a swatch has keeps the dialog open. When another
/// document has become the active one, or the color group at the group's place in the list no
/// longer has the name and swatches it opened with, it closes as Cancel does and renames nothing.
/// Color groups carry nothing else to tell them apart.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if !same_group(app, d) {
        app.ui.dialog = None;
        return Ok(json!({}));
    }
    let r = app.run("swatch.renameGroup", json!({ "group": d.str("__group"), "name": d.str("name") }));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

    fn set(app: &mut VectorcraftApp, v: &str) {
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(v));
    }

    fn groups(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.swatch_groups.iter().map(|g| g.name.clone()).collect()
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// The dialog opens on the group's name; OK renames it in one undo step, a taken or empty name
    /// keeps the dialog open with nothing changed, Cancel changes nothing, and OK with the name
    /// unchanged records no undo step.
    #[test]
    fn renames_the_group_and_keeps_a_refused_name_open() {
        let mut app = app();
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("name").as_str(), d.str("__group").as_str()), (KIND, "Brights", "Brights"));
        // Drawn headlessly in the shared frame.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let undo = undo_len(&app);
        for taken in ["Grays", "White", "  "] {
            set(&mut app, taken);
            assert!(super::super::confirm(&mut app).is_err(), "{taken:?}");
            assert!(app.ui.dialog.is_some(), "{taken:?} keeps the dialog open");
            assert_eq!(undo_len(&app), undo, "{taken:?}");
        }
        set(&mut app, "Vivid");
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none() && groups(&app).contains(&"Brights".to_string()), "Cancel keeps the name");
        assert_eq!(undo_len(&app), undo);
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(undo_len(&app), undo, "the same name records no undo step");
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        set(&mut app, "Vivid");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && groups(&app).contains(&"Vivid".to_string()));
        assert_eq!(undo_len(&app), undo + 1);
        assert!(app.run("ui.colorGroupOptions", json!({"group": "Brights"})).is_err());
        assert!(app.run("ui.colorGroupOptions", json!({"group": "White"})).is_err(), "a swatch is not a group");
        assert!(app.run("ui.colorGroupOptions", json!({})).is_err());
        assert!(app.run("ui.colorGroupOptions", json!({"group": 5})).is_err());
    }

    /// OK closes the dialog and renames nothing when another document has become the active one,
    /// or when another group took the name of the group it opened on while it was open.
    #[test]
    fn renames_only_the_group_it_opened_on() {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        app.run("document.activate", json!({"index": 1})).unwrap();
        set(&mut app, "Vivid");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert!(groups(&app).contains(&"Brights".to_string()), "the document made active keeps its group");
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(groups(&app).contains(&"Brights".to_string()), "the group it opened on keeps its name");
        // Its group ungrouped, and another group renamed to its name.
        app.run("ui.colorGroupOptions", json!({"group": "Brights"})).unwrap();
        app.run("swatch.ungroup", json!({"name": "Brights"})).unwrap();
        app.run("swatch.renameGroup", json!({"group": "Grays", "name": "Brights"})).unwrap();
        set(&mut app, "Vivid");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert!(groups(&app).contains(&"Brights".to_string()) && !groups(&app).contains(&"Vivid".to_string()));
        // Its group moved to another place in the list.
        app.run("swatch.newGroup", json!({"name": "Extra", "swatches": ["White"]})).unwrap();
        let first = groups(&app)[0].clone();
        app.run("ui.colorGroupOptions", json!({"group": first})).unwrap();
        app.run("swatch.move", json!({"names": [first]})).unwrap();
        set(&mut app, "Vivid");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && groups(&app).contains(&first) && !groups(&app).contains(&"Vivid".to_string()));
    }
}
