//! New Color Group: a name and what the group is made from, the swatches selected in the Swatches
//! panel or the colours of the selected artwork (whose process colours can become global swatches
//! the art links to, with swatches for tints too). OK runs `swatch.newGroup`.
//!
//! Fields: `name`, `from` (`swatches` or `artwork`), `toGlobal` and `includeTints` (artwork only). The selected
//! swatch names are kept by the dialog's OK.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind, ParamResult};
use serde_json::{Value, json};

use super::swatch_options::{NO_WINDOW, field_label, hosted};
use crate::VectorcraftApp;

/// The dialog kind of New Color Group.
pub const KIND: &str = "newColorGroup";

/// Open New Color Group for the swatches selected in the panel. It starts from the artwork when
/// art is selected and no swatches are.
pub fn open(app: &mut VectorcraftApp, swatches: Vec<String>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let (document, reverts) = (st.uid, st.reverts);
    let art = !st.selection.is_empty();
    let mut fields = DialogFields::new();
    fields.set("name", st.doc.free_swatch_name("Color Group"));
    fields.set("from", if art && swatches.is_empty() { "artwork" } else { "swatches" });
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    let sources =
        vec![("swatches".to_owned(), tl!("Selected Swatches").to_owned().into()), ("artwork".to_owned(), tl!("Selected Artwork").to_owned().into())];
    let rows = vec![
        Param::new("name", field_label(tl!("Name:")), ParamKind::text("")),
        Param::new("from", field_label(tl!("Create From:")), ParamKind::radio(sources, Some("swatches".to_owned()))).disabled_options(if art {
            None
        } else {
            Some("artwork")
        }),
        Param::new("toGlobal", tl!("Convert Process to Global").to_owned(), ParamKind::bool(true)).enabled_when("from", "artwork"),
        Param::new("includeTints", tl!("Include Swatches for Tints").to_owned(), ParamKind::bool(false)).enabled_when("from", "artwork"),
    ];
    let dialog = hosted(&ctx, KIND, tl!("New Color Group"), fields, rows, false);
    let ok = move |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        let Some(source) = app.session.active().filter(|active| active.uid == document) else {
            return Err(tl!("Create this color group in the document that opened the dialog.").to_owned());
        };
        if source.reverts != reverts {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").to_owned());
        }
        app.run("swatch.newGroup", params(&answer.fields, &swatches)).map(|_| ())
    };
    super::host::open_held(app, dialog, ok).ok_or(NO_WINDOW)?;
    Ok(Value::Null)
}

/// `swatch.newGroup` parameters from the fields and the swatches selected when the dialog opened.
fn params(fields: &DialogFields, swatches: &[String]) -> Value {
    let name = fields.text("name").unwrap_or_default();
    if fields.text("from") == Some("artwork") {
        let on = |key: &str| fields.bool(key).unwrap_or(false);
        json!({"name": name, "fromArtwork": true, "toGlobal": on("toGlobal"), "includeTints": on("includeTints")})
    } else {
        json!({"name": name, "swatches": swatches})
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::*;

    #[test]
    fn a_color_group_draft_cannot_change_the_drawing_restored_by_revert() {
        for swatches in [vec!["Saved Ink".to_owned()], Vec::new()] {
            let dir = vectorcraft_testkit::TempDir::new("new-color-group-revert");
            let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
            app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
            app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"ids": [id], "color": "#123456"})).unwrap();
            app.run("swatch.new", json!({"name": "Saved Ink", "color": "#446688"})).unwrap();
            app.run("file.saveAs", json!({"path": dir.join("Saved.vectorcraft")})).unwrap();
            app.run("paint.setFill", json!({"ids": [id], "color": "#abcdef"})).unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
            let before = app.session.active().unwrap();
            let (uid, reverts) = (before.uid, before.reverts);
            open(&mut app, swatches).unwrap();
            app.run("file.revert", json!({"confirmed": true})).unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
            let loaded = app.session.active().unwrap();
            assert_eq!((loaded.uid, loaded.reverts), (uid, reverts + 1));
            let (saved, undo) = (loaded.doc.clone(), loaded.history.undo.len());
            let refused = crate::dialogs::tests_export::try_confirm(&mut app);
            let current = app.session.active().unwrap();
            assert_eq!(current.doc, saved);
            assert_eq!(current.history.undo.len(), undo);
            assert_eq!(refused.unwrap_err(), "The document was reverted. Reopen this dialog to continue.");
        }
    }

    #[test]
    fn color_group_does_not_convert_the_other_drawings_artwork() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        for color in ["#123456", "#446688"] {
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let id = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 25, "height": 25})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"ids": [id], "color": color})).unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
        }
        let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
        let other = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        activate(&mut app, 0);
        open(&mut app, Vec::new()).unwrap();
        activate(&mut app, 1);

        let ctx = app.ui_ctx.clone().unwrap();
        let dialogs = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        let confirm = |app: &mut VectorcraftApp| {
            dialogs.with(|host| host.confirm_by_id(&ctx, id));
            crate::dialogs::host::pump(app, &ctx);
            crate::testing::app_frame(app, &ctx, Default::default());
        };
        confirm(&mut app);
        let current = app.session.active().unwrap();
        assert_eq!(current.doc, other, "the other drawing keeps its swatches and process fills");
        assert_eq!(current.history.undo.len(), undo);

        activate(&mut app, 0);
        confirm(&mut app);
        assert!(app.session.active().unwrap().doc.swatch_groups.iter().any(|group| group.name == "Color Group"));
    }
}
