//! The Artboard tool's canvas menu › Rename, and the Artboards panel's row menu › Rename: a name
//! for the selected artboards. OK runs
//! `artboard.rename` as one undo step; with several artboards the name is numbered.
//!
//! Fields: `name` (the artboard's name with one artboard, empty with several). The artboards the
//! dialog opened on are held by its OK.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::{Value, json};

use super::host::open_held;
use crate::VectorcraftApp;

/// The command Rename answers for; the key of its dialog.
pub(crate) const CMD: &str = "artboard.rename";

/// `artboard.rename` for the selected artboards: opens the dialog on the one artboard's name, or on an empty name
/// for several.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let st = app.session.active().ok_or_else(|| tl!("No document").to_owned())?;
    let document = st.uid;
    let selected: Vec<_> =
        crate::panels::artboards::picked(app, st.doc.artboards.len()).into_iter().filter_map(|index| st.doc.artboards.get(index)).collect();
    let name = match selected.as_slice() {
        [one] => one.name.clone(),
        _ => String::new(),
    };
    let artboards: Vec<_> = selected.iter().map(|board| board.id).collect();
    let mut fields = DialogFields::new();
    fields.set("name", name);
    let dialog = HostedParamDialog::command(&ctx, CMD, fields, |fields| {
        let rows = vec![Param::new("name", tl!("Name:"), ParamKind::text(""))];
        ParamDialog::command(CMD, tl!("Rename"), rows).values(Bind::bound(fields))
    });
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        let name = result.fields.text("name").unwrap_or_default();
        // An empty name keeps the names as they are, as the Artboards panel's name field does.
        if name.trim().is_empty() {
            return Ok(());
        }
        let st =
            app.session.active().filter(|st| st.uid == document).ok_or_else(|| tl!("This dialog was opened in a different document.").to_owned())?;
        let indices = artboards
            .iter()
            .map(|id| st.doc.artboards.iter().position(|board| board.id == *id))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| tl!("One of the artboards being renamed no longer exists.").to_owned())?;
        if let Err(e) = app.run(CMD, json!({ "indices": indices, "name": name })) {
            app.status(e);
        }
        Ok(())
    };
    open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        app
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect()
    }

    /// The open Rename dialog: its id and fields.
    fn open_rename(app: &VectorcraftApp) -> (craft::ui::dialog::DialogId, Value) {
        let dialogs = app.dialogs().unwrap();
        let open = dialogs.with(|host| host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(CMD))).unwrap();
        (open.id, open.fields.unwrap())
    }

    /// Sets `name` in the open dialog, presses OK and runs the host pump, which runs the held OK.
    fn rename(app: &mut VectorcraftApp, name: &str) {
        let ctx = app.ui_ctx.clone().unwrap();
        let (id, _) = open_rename(app);
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|host| host.set_json_field(&ctx, id, "name", json!(name))).unwrap();
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
    }

    /// With one artboard the field starts on its name; with several it starts empty, and OK numbers
    /// the name in one undo step.
    #[test]
    fn the_menu_item_renames_the_selected_artboards() {
        let mut app = app();
        crate::menus::invoke(&mut app, CMD, json!({}));
        assert_eq!(open_rename(&app).1["name"], json!("Artboard 1"));
        let ctx = app.ui_ctx.clone().unwrap();
        let (id, _) = open_rename(&app);
        app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(!app.any_dialog_open(), "Cancel closes the first dialog");

        app.session.set_tool_option("selected", &json!([2, 0]));
        crate::menus::invoke(&mut app, CMD, json!({}));
        assert_eq!(open_rename(&app).1["name"], json!(""));
        let steps = app.session.active().unwrap().history.undo.len();
        rename(&mut app, " Page ");
        assert!(!app.any_dialog_open());
        assert_eq!(names(&app), ["Page-1", "Artboard 2", "Page-2"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step");
    }

    #[test]
    fn rename_follows_the_original_artboards_after_reordering() {
        let mut app = app();
        app.session.set_tool_option("selected", &json!([0, 2]));
        open(&mut app).unwrap();
        app.run("artboard.reorder", json!({"index": 0, "to": 2})).unwrap();
        assert_eq!(names(&app), ["Artboard 2", "Artboard 3", "Artboard 1"]);
        let steps = app.session.active().unwrap().history.undo.len();
        rename(&mut app, "Page");
        assert_eq!(names(&app), ["Artboard 2", "Page-1", "Page-2"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);
    }

    #[test]
    fn rename_waits_for_the_document_it_opened_in() {
        let mut app = app();
        open(&mut app).unwrap();
        app.run("file.new", json!({"width": 100, "height": 50, "artboards": 3})).unwrap();
        let steps = app.session.active().unwrap().history.undo.len();
        rename(&mut app, "Source only");
        assert_eq!(names(&app), ["Artboard 1", "Artboard 2", "Artboard 3"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
        assert!(app.any_dialog_open(), "the original document can still answer");

        app.session.execute("document.activate", &json!({"index": 0})).unwrap();
        rename(&mut app, "Source only");
        assert_eq!(names(&app), ["Source only", "Artboard 2", "Artboard 3"]);
        assert!(!app.any_dialog_open());
    }

    #[test]
    fn rename_leaves_the_other_targets_alone_if_an_original_artboard_was_removed() {
        let mut app = app();
        app.session.set_tool_option("selected", &json!([0, 2]));
        open(&mut app).unwrap();
        app.run("artboard.delete", json!({"index": 2})).unwrap();
        app.run("artboard.duplicate", json!({"index": 1})).unwrap();
        let before = names(&app);
        let steps = app.session.active().unwrap().history.undo.len();
        rename(&mut app, "Page");
        assert_eq!(names(&app), before);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
        assert!(app.any_dialog_open());
    }

    /// OK with an empty name or the name the artboard already has records no undo step.
    #[test]
    fn nothing_changes_for_an_empty_or_unchanged_name() {
        let mut app = app();
        let steps = app.session.active().unwrap().history.undo.len();
        for name in ["   ", "Artboard 1"] {
            crate::menus::invoke(&mut app, CMD, json!({}));
            rename(&mut app, name);
            assert!(!app.any_dialog_open());
        }
        assert_eq!(names(&app), ["Artboard 1", "Artboard 2", "Artboard 3"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
    }
}
