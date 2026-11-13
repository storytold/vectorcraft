//! The Artboard tool's canvas menu › Rename, and the Artboards panel's row menu › Rename: a name
//! for the selected artboards. OK runs
//! `artboard.rename` as one undo step; with several artboards the name is numbered.
//!
//! Fields: `name` (the artboard's name with one artboard, empty with several) and `indices` (the
//! artboards the dialog opened on).

use serde_json::{Value, json};

use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the artboard Rename dialog.
pub const KIND: &str = "renameArtboards";

const CMD: &str = "artboard.rename";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Rename").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// The fields the menu item opens with, for the selected artboards.
pub fn open_fields(app: &VectorcraftApp) -> Value {
    let Some(st) = app.session.active() else { return json!({"name": "", "indices": []}) };
    let indices = crate::panels::artboards::picked(app, st.doc.artboards.len());
    let name = match indices.as_slice() {
        [one] => st.doc.artboards.get(*one).map_or("", |a| a.name.as_str()),
        _ => "",
    };
    json!({ "name": name, "indices": indices })
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("Name:"), 56.0, |ui| {
        let r = form::text_edit_named(ui, d, "name", tl!("Name:"), 220.0);
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
    });
    false
}

/// An empty name keeps the names as they are, as the Artboards panel's name field does.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let name = d.str("name");
    if name.trim().is_empty() {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let indices = d.fields.get("indices").cloned().unwrap_or(Value::Null);
    run_and_close(app, CMD, json!({ "indices": indices, "name": name }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        app
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect()
    }

    /// With one artboard the field starts on its name; with several it starts empty, and OK numbers
    /// the name in one undo step.
    #[test]
    fn the_menu_item_renames_the_selected_artboards() {
        let mut app = app();
        crate::menus::invoke(&mut app, CMD, json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, KIND);
        assert_eq!(d.str("name"), "Artboard 1");
        app.ui.dialog = None;

        app.session.set_tool_option("selected", &json!([2, 0]));
        crate::menus::invoke(&mut app, CMD, json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(" Page "));
        let steps = app.session.active().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(names(&app), ["Page-1", "Artboard 2", "Page-2"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step");
    }

    /// OK with an empty name or the name the artboard already has records no undo step.
    #[test]
    fn nothing_changes_for_an_empty_or_unchanged_name() {
        let mut app = app();
        let steps = app.session.active().unwrap().history.undo.len();
        for name in ["   ", "Artboard 1"] {
            crate::menus::invoke(&mut app, CMD, json!({}));
            app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(name));
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(app.ui.dialog.is_none());
        }
        assert_eq!(names(&app), ["Artboard 1", "Artboard 2", "Artboard 3"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
    }
}
