//! Select → Edit Selection…: the document's saved selections in a list, renamed in place or deleted.
//! Nothing changes until OK, which applies every rename and delete as one undo step
//! (`select.editSaved` `edits`); Cancel leaves them as they were. The dialog is the saved-names dialog,
//! which Edit Views shares ([`super::saved_names`]).

use serde_json::{Value, json};
use vectorcraft_engine::doc::SavedSelection;

use super::saved_names::{self, SavedNames};
use crate::VectorcraftApp;

/// The dialog kind of Edit Selection.
pub const KIND: &str = "editSelection";

/// Open Edit Selection on the active document's saved selections.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let names = app.run("select.savedList", json!({}))?;
    let names: Vec<String> = names.as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    if names.is_empty() {
        return Err("No saved selections (Select → Save Selection…)".into());
    }
    let dialog =
        SavedNames { kind: KIND, title: tl!("Edit Selection"), list: tl!("Saved Selections"), command: "select.editSaved", max: SavedSelection::MAX };
    saved_names::open(app, dialog, names)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{DialogHost, HeldOk, StackDialog as _};
    use serde_json::json;

    use super::super::saved_names::{SavedNamesDialog, id_of};
    use super::KIND;
    use crate::VectorcraftApp;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
        app.run("file.new", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        for n in ["Logo", "Icons", "Text"] {
            app.run("select.save", json!({ "name": n })).unwrap();
        }
        app
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("the test context")
    }

    fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().expect("a window").with(f)
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        host(app, |h| h.get_by_id::<SavedNamesDialog>(id_of(KIND)).is_some())
    }

    fn set(app: &VectorcraftApp, k: &str, v: serde_json::Value) {
        let ctx = ctx(app);
        host(app, |h| h.set_json_field(&ctx, id_of(KIND), k, v)).unwrap();
    }

    fn names(app: &mut VectorcraftApp) -> serde_json::Value {
        app.run("select.savedList", json!({})).unwrap()
    }

    /// OK as a click: the edits wait for the pump, which sends them to the command and answers.
    fn press_ok(app: &mut VectorcraftApp) -> Option<HeldOk> {
        let ctx = ctx(app);
        host(app, |h| h.confirm_by_id(&ctx, id_of(KIND)));
        crate::dialogs::host::pump(app, &ctx);
        host(app, |h| h.get_by_id::<SavedNamesDialog>(id_of(KIND)).and_then(|d| d.held_state()))
    }

    /// The next OK of a dialog whose edits were accepted closes it, as the pass that draws it would.
    fn finish(app: &VectorcraftApp) {
        let ctx = ctx(app);
        host(app, |h| h.confirm_by_id(&ctx, id_of(KIND)));
    }

    fn cancel(app: &VectorcraftApp) {
        let ctx = ctx(app);
        host(app, |h| h.cancel_by_id(&ctx, id_of(KIND)));
    }

    #[test]
    fn nothing_changes_until_ok_and_all_edits_are_one_undo_step() {
        let mut app = app();
        super::open(&mut app).unwrap();
        // Rename the first and delete the last.
        set(&app, "name0", json!(" Brand "));
        set(&app, "deleted2", json!(true));
        assert_eq!(names(&mut app), json!(["Logo", "Icons", "Text"]));
        assert!(matches!(press_ok(&mut app), Some(HeldOk::Accepted)));
        finish(&app);
        assert!(!is_open(&app));
        assert_eq!(names(&mut app), json!(["Brand", "Icons"]));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(names(&mut app), json!(["Logo", "Icons", "Text"]));
    }

    #[test]
    fn cancel_leaves_them_and_a_shared_name_keeps_the_dialog_open() {
        let mut app = app();
        super::open(&mut app).unwrap();
        set(&app, "name0", json!("Gone"));
        cancel(&app);
        assert!(!is_open(&app));
        assert_eq!(names(&mut app), json!(["Logo", "Icons", "Text"]));
        super::open(&mut app).unwrap();
        set(&app, "name0", json!("Text"));
        assert!(matches!(press_ok(&mut app), Some(HeldOk::Refused(_))), "two saved selections would share the name");
        assert!(is_open(&app));
        assert_eq!(names(&mut app), json!(["Logo", "Icons", "Text"]));
        // Deleting the other one frees the name.
        set(&app, "deleted2", json!(true));
        assert!(matches!(press_ok(&mut app), Some(HeldOk::Accepted)));
        finish(&app);
        assert_eq!(names(&mut app), json!(["Text", "Icons"]));
    }

    #[test]
    fn it_needs_a_saved_selection() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
        app.run("file.new", json!({})).unwrap();
        assert!(super::open(&mut app).is_err());
        assert!(!app.any_dialog_open());
        assert!(!is_open(&app));
    }
}
