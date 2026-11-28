//! Type › Smart Punctuation…: which keyboard characters to replace with their typographic forms,
//! where (the selected type or the whole document) and whether to report how many of each were
//! replaced. OK runs `type.smartPunctuation` as one undo step, and the dialog opens on the fields
//! confirmed last.
//!
//! Fields: the switches `fiLigatures`, `flLigatures`, `quotes`, `spaces`, `dashes`, `ellipsis` and
//! `fractions`, `scope` (`selection` or `document`) and `report`.

use std::collections::HashSet;

use craft::ui::contract::{Bind, SaveKey};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The command OK runs, the menu item's, and the key of the dialog on the host.
pub const COMMAND: &str = "type.smartPunctuation";

// PLAN: a SaveKey is `crate_prefix.name`, so the key carries the `vectorcraft` prefix.
const REMEMBER: SaveKey = SaveKey::new("vectorcraft.smartPunctuation");

/// The replacement options in the dialog's order: (field, the key of their count in the
/// command's reply, label).
fn options() -> [(&'static str, &'static str, &'static str); 7] {
    [
        ("fiLigatures", "fiLigatures", tl!("ff, fi, ffi Ligatures")),
        ("flLigatures", "flLigatures", tl!("ff, fl, ffl Ligatures")),
        ("quotes", "quotes", tl!("Smart Quotes")),
        ("spaces", "spaces", tl!("Smart Spaces")),
        ("dashes", "dashes", tl!("En, Em Dashes")),
        ("ellipsis", "ellipses", tl!("Ellipses")),
        ("fractions", "fractions", tl!("Expert Fractions")),
    ]
}

/// The command's parameters: the switches and the scope.
fn params_of(result: &ParamResult) -> Value {
    let mut p = serde_json::Map::new();
    for (key, ..) in options() {
        p.insert(key.into(), json!(result.fields.bool(key).unwrap_or(false)));
    }
    p.insert("scope".into(), json!(if result.fields.text("scope") == Some("document") { "document" } else { "selection" }));
    Value::Object(p)
}

/// Open Smart Punctuation on the fields confirmed last (quotes, dashes and ellipses on the first
/// time). With nothing selected the scope is the whole document. OK runs the command and, with
/// Report Results on, lists how many of each chosen replacement it made.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let source = app.session.active().ok_or("no document")?;
    let document = (source.uid, source.reverts);
    let selected = !source.selection.is_empty();
    let selection = source.selection.objects.iter().copied().collect::<HashSet<_>>();
    let mut params: Vec<Param> = options()
        .into_iter()
        .map(|(key, _, label)| Param::new(key, label, ParamKind::bool(matches!(key, "quotes" | "dashes" | "ellipsis"))))
        .collect();
    params.push(Param::section(tl!("Replace In")));
    let scope = ParamKind::radio(
        vec![("selection".to_owned(), Label::from(tl!("Selected Text Only"))), ("document".to_owned(), Label::from(tl!("Entire Document")))],
        Some("selection".to_owned()),
    );
    params.push(Param::new("scope", tl!("Replace In"), scope).disabled_options(if selected { vec![] } else { vec!["selection"] }));
    params.push(Param::new("report", tl!("Report Results"), ParamKind::bool(false)));
    let mut dialog = HostedParamDialog::new(&ctx, COMMAND, DialogFields::new(), move |fields| {
        ParamDialog::command(COMMAND, tl!("Smart Punctuation"), params.clone()).values(Bind::bound(fields)).remember(REMEMBER)
    });
    if !selected {
        dialog.set_field("scope", FieldValue::Text("document".into()));
    }
    let owner = egui::Id::unique((COMMAND, "opening-drawing"));
    ctx.data_mut(|data| data.insert_temp(owner, document));
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| -> Result<(), String> {
        let document = ctx.data(|data| data.get_temp::<(u64, u64)>(owner));
        let Some(active) = app.session.active().filter(|st| document.is_some_and(|(uid, _)| st.uid == uid)) else {
            return Err(tl!("Return to the original drawing to replace its punctuation").into());
        };
        if document.is_some_and(|(_, reverts)| active.reverts != reverts) {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").into());
        }
        let params = params_of(result);
        if params["scope"] == "selection"
            && (active.selection.is_empty()
                || active.selection.objects.len() != selection.len()
                || active.selection.objects.iter().any(|id| !selection.contains(id)))
        {
            return Err(tl!("The selected text changed. Restore the selection to continue.").into());
        }
        let r = app.run(COMMAND, params)?;
        result.remember(&ctx);
        if result.fields.bool("report") == Some(true) {
            let lines: Vec<String> = options()
                .into_iter()
                .filter(|(key, ..)| result.fields.bool(key) == Some(true))
                .map(|(_, count, label)| format!("{} {}", crate::i18n::colon(label), r["replaced"][count].as_u64().unwrap_or(0)))
                .collect();
            super::confirm::tell(app, tl!("Smart Punctuation"), &lines.join("\n"));
        }
        Ok(())
    };
    super::host::open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app_with_text(text: &str) -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": text, "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
        (app, id)
    }

    fn plain(app: &VectorcraftApp, id: u64) -> String {
        match &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => String::new(),
        }
    }

    /// The open host entry with `key`.
    fn entry(app: &VectorcraftApp, key: &str) -> Option<craft::ui::dialog::OpenDialog> {
        app.dialogs().unwrap().with(|h| h.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)))
    }

    fn set(app: &VectorcraftApp, field: &str, value: Value) {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), entry(app, COMMAND).unwrap().id);
        app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, field, value)).unwrap();
    }

    /// OK as the button does it, then the pump that runs the command, then the close the accepted OK waits for, and
    /// a frame that opens what waited for the dialog (the report).
    fn ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), entry(app, COMMAND).unwrap().id);
        let host = app.dialogs().unwrap();
        host.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        host.with(|h| h.confirm_by_id(&ctx, id));
        crate::testing::app_frame(app, &ctx, Default::default());
    }

    /// The menu item opens the dialog; OK replaces in one undo step, reports each chosen
    /// replacement's count, and the next opening starts from the fields confirmed.
    #[test]
    fn menu_item_opens_the_dialog_and_ok_reports_the_counts() {
        let (mut app, id) = app_with_text("\"Hi\".  1/2 -- off");
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::menus::invoke(&mut app, COMMAND, json!({}));
        let fields = entry(&app, COMMAND).expect("Smart Punctuation opens").fields.unwrap();
        assert_eq!(fields["scope"], "selection");
        for key in ["spaces", "fractions", "fiLigatures", "report"] {
            set(&app, key, json!(true));
        }
        set(&app, "ellipsis", json!(false));
        let undo = app.session.doc().unwrap().history.undo.len();
        ok(&mut app);
        assert_eq!(plain(&app, id), "“Hi”. ½ – o\u{FB00}");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        // The alert's automation fields list only its message: its detail is read as the app told it.
        let (_, detail) = crate::dialogs::confirm::open_message(&app).expect("Report Results shows the counts");
        assert_eq!(detail, "ff, fi, ffi Ligatures: 1\nSmart Quotes: 2\nSmart Spaces: 1\nEn, Em Dashes: 1\nExpert Fractions: 1");
        assert!(entry(&app, COMMAND).is_none());
        crate::menus::invoke(&mut app, COMMAND, json!({}));
        let again = entry(&app, COMMAND).unwrap().fields.unwrap();
        assert!(again["fractions"] == true && again["report"] == true && again["ellipsis"] == false);
    }

    /// With nothing selected the dialog replaces in the entire document, and without Report
    /// Results it shows no message. An unchanged document keeps its undo history.
    #[test]
    fn with_nothing_selected_the_whole_document_is_replaced() {
        let (mut app, id) = app_with_text("a--b");
        app.run("select.none", json!({})).unwrap();
        crate::menus::invoke(&mut app, COMMAND, json!({}));
        assert_eq!(entry(&app, COMMAND).unwrap().fields.unwrap()["scope"], "document");
        ok(&mut app);
        assert_eq!(plain(&app, id), "a–b");
        assert!(entry(&app, COMMAND).is_none() && entry(&app, "message").is_none());
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::menus::invoke(&mut app, COMMAND, json!({}));
        ok(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    }

    #[test]
    fn smart_punctuation_waits_for_its_original_drawing() {
        let (mut app, source) = app_with_text("a--b");
        app.run("select.none", json!({})).unwrap();
        open(&mut app).unwrap();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let other = app.run("text.create", json!({"x": 10, "y": 40, "text": "c--d", "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        let error = super::super::tests_export::try_confirm(&mut app).expect_err("Smart Punctuation must refuse the other drawing");
        assert!(error.contains("original drawing"), "{error}");
        assert_eq!(plain(&app, other), "c--d");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        assert_eq!(entry(&app, COMMAND).unwrap().fields.unwrap()["scope"], "document");
        app.run("document.activate", json!({ "index": 0 })).unwrap();
        ok(&mut app);
        assert_eq!(plain(&app, source), "a–b");
    }

    #[test]
    fn smart_punctuation_refuses_a_draft_from_before_revert() {
        let (mut app, id) = app_with_text("a--b");
        let dir = vectorcraft_testkit::TempDir::new("smart-punctuation-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run("select.none", json!({})).unwrap();
        open(&mut app).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        let answer = super::super::tests_export::try_confirm(&mut app);
        assert_eq!(plain(&app, id), "a--b", "OK changed the restored drawing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(answer, Err("The document was reverted. Reopen this dialog to continue.".into()));

        let (ctx, dialog) = (app.ui_ctx.clone().unwrap(), entry(&app, COMMAND).unwrap().id);
        app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, dialog));
        crate::testing::app_frame(&mut app, &ctx, Default::default());
        assert!(entry(&app, COMMAND).is_none());
        open(&mut app).unwrap();
        ok(&mut app);
        assert_eq!(plain(&app, id), "a–b");
    }

    #[test]
    fn reopened_smart_punctuation_applies_in_its_new_drawing() {
        let (mut app, source) = app_with_text("a--b");
        app.run("select.none", json!({})).unwrap();
        open(&mut app).unwrap();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let other = app.run("text.create", json!({"x": 10, "y": 40, "text": "c--d", "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.none", json!({})).unwrap();
        open(&mut app).unwrap();
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), entry(&app, COMMAND).unwrap().id);
        app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!(plain(&app, other), "c–d");
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(plain(&app, source), "a--b");
    }

    fn selected_text_pair() -> (VectorcraftApp, u64, u64) {
        let (mut app, selected) = app_with_text("a--b");
        let other = app.run("text.create", json!({"x": 10, "y": 70, "text": "c--d", "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [selected]})).unwrap();
        open(&mut app).unwrap();
        assert_eq!(entry(&app, COMMAND).unwrap().fields.unwrap()["scope"], "selection");
        (app, selected, other)
    }

    #[test]
    fn selected_text_only_does_not_expand_to_the_document_after_clearing_selection() {
        let (mut app, selected, other) = selected_text_pair();
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("select.none", json!({})).unwrap();
        assert_eq!(entry(&app, COMMAND).unwrap().fields.unwrap()["scope"], "selection");

        let answer = super::super::tests_export::try_confirm(&mut app);
        assert_eq!([plain(&app, selected), plain(&app, other)], ["a--b", "c--d"], "Selected Text Only must not expand to the document");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(answer, Err("The selected text changed. Restore the selection to continue.".into()));

        app.run("select.set", json!({"ids": [selected]})).unwrap();
        ok(&mut app);
        assert_eq!([plain(&app, selected), plain(&app, other)], ["a–b", "c--d"], "retry changes only the original selection");
    }

    #[test]
    fn selected_text_only_refuses_later_art_but_entire_document_still_works() {
        let (mut app, selected, other) = selected_text_pair();
        app.run("select.set", json!({"ids": [other]})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();

        let answer = super::super::tests_export::try_confirm(&mut app);
        assert_eq!([plain(&app, selected), plain(&app, other)], ["a--b", "c--d"], "Selected Text Only must not change later artwork");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(answer, Err("The selected text changed. Restore the selection to continue.".into()));

        set(&app, "scope", json!("document"));
        ok(&mut app);
        assert_eq!([plain(&app, selected), plain(&app, other)], ["a–b", "c–d"], "an explicit Entire Document still works");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }
}
