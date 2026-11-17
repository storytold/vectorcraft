//! Color Group Options: the name of a color group in the Swatches panel. OK renames the group
//! (`swatch.renameGroup`) and keeps it selected in the panel; its swatches and their colors stay
//! as they are.
//!
//! Fields: `name` and `__group` (the group being renamed).

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Color Group Options.
pub const KIND: &str = "colorGroupOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Color Group Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// `ui.colorGroupOptions {group}`: open Color Group Options for color group `group`.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "group";
    let group = match p.get(key) {
        Some(Value::String(g)) => g.as_str(),
        None | Some(Value::Null) => return Err("missing `group`".into()),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let d = &app.session.active().ok_or("no document open")?.doc;
    let name = d.swatch_groups.iter().find(|g| g.name == group).ok_or_else(|| format!("no colour group `{group}`"))?.name.clone();
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "name": name, "__group": name })));
    Ok(Value::Null)
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
/// A name that is empty or that another group or a swatch has keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
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
}
