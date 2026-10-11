//! Closing documents with unsaved changes. File → Close / Close All, a tab's ×, Quit and the
//! window's close button first ask Save / Don't Save / Cancel for each modified document (one
//! dialog per document); Cancel stops the whole operation and a cancelled Save As keeps the
//! document open.
//!
//! The question is an ordinary dialog (`UiState::dialog`, kind [`KIND`]), so agents answer it
//! like any other: `ui.dialog.confirm` saves, `ui.dialog.set {field: "discard", value: true}` then
//! confirm discards, `ui.dialog.cancel` cancels.

use serde_json::{Value, json};

use crate::state::Dialog;
use crate::{VectorcraftApp, io};

/// Dialog kind. Fields: `index` (document), `uid` (the same document, which `index` may no longer
/// point at once a file dialog answers), `name` (its title), `then` (`close`, `closeAll` or `quit`:
/// what continues after this document) and `discard` (Don't Save).
pub const KIND: &str = "saveChanges";

/// Nothing closes while a file dialog's sheet is on the window (macOS): what asked for it runs
/// again when it answers. (Elsewhere the dialog is a window of its own, and closing goes on.)
fn refuse_while_picking(app: &VectorcraftApp) -> Result<(), String> {
    if app.file_sheet_open() { Err(crate::picks::busy().into()) } else { Ok(()) }
}

/// File → Close (or a tab's ×) for document `i`.
pub fn close(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {
    refuse_while_picking(app)?;
    // A save still running decides whether there is anything left to save.
    crate::background::wait_all(app);
    let dirty = app.session.documents().get(i).ok_or("no such document")?.is_dirty();
    if dirty { ask(app, i, "close") } else { close_now(app, i) }
}

/// File → Close All (`then` = `closeAll`) or Quit (`quit`): ask about the next modified document,
/// or finish once none is left.
pub fn close_all(app: &mut VectorcraftApp, then: &str) -> Result<Value, String> {
    refuse_while_picking(app)?;
    crate::background::wait_all(app);
    if let Some(i) = app.session.documents().iter().position(|d| d.is_dirty()) {
        return ask(app, i, then);
    }
    if then == "quit" {
        // Nothing unsaved is left: no recovery copies either.
        vectorcraft_engine::cmd::recovery::forget_all(&mut app.session);
        // The host closes the window.
        app.ui.status = "quit".into();
        return Ok(Value::Null);
    }
    if app.session.documents().is_empty() {
        return Ok(json!({ "closed": 0 }));
    }
    let r = app.session.execute("file.closeAll", &json!({})).map_err(|e| e.to_string());
    app.sync_views();
    r
}

/// Is any open document modified (the window's close button must ask first)?
pub fn any_dirty(app: &VectorcraftApp) -> bool {
    app.session.documents().iter().any(|d| d.is_dirty())
}

/// Show document `i` and ask whether to save it.
fn ask(app: &mut VectorcraftApp, i: usize, then: &str) -> Result<Value, String> {
    app.session.set_active(i);
    let doc = &app.session.documents()[i];
    let (name, uid) = (doc.title(), doc.uid);
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "index": i, "uid": uid, "name": name, "then": then })));
    Ok(json!({ "pending": KIND }))
}

/// Close document `i` through the engine command, keeping `views` aligned with the documents.
fn close_now(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {
    let r = app.session.execute("file.close", &json!({ "index": i })).map_err(|e| e.to_string())?;
    if i < app.views.len() {
        app.views.remove(i);
    }
    app.sync_views();
    Ok(r)
}

/// Answer the open dialog: save (or, with `discard`, don't), close the document and carry on.
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = app.ui.dialog.take().ok_or("no dialog open")?;
    let docs = app.session.documents();
    // By its uid: documents may have closed while its Save dialog was open.
    let i = match d.fields.get("uid").and_then(Value::as_u64) {
        Some(uid) => docs.iter().position(|doc| doc.uid == uid),
        None => d.fields.get("index").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < docs.len()),
    };
    let i = i.ok_or("no such document")?;
    if !d.bool("discard") {
        app.session.set_active(i);
        let r = io::save(app, vectorcraft_engine::cmd::fileio::SaveMode::Save, &json!({}), false)?;
        // A save that asks first (replacing a file that would lose what opening it left out) keeps
        // the document open: it closes once saved.
        if r.get("pending").is_some() {
            return Ok(r);
        }
        // Closing needs the file written: wait for a background save, and stop if it failed.
        if r["background"] == true {
            crate::background::wait_all(app);
            if let Some(d) = app.session.documents().get(i).filter(|d| d.is_dirty()) {
                return Err(format!("{}: not saved, so not closed", d.title()));
            }
        }
    }
    close_now(app, i)?;
    match d.str("then").as_str() {
        "close" => Ok(Value::Null),
        then => close_all(app, then),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use vectorcraft_engine::Session;

    /// An app whose Save As picks `out.vectorcraft` and records written paths.
    fn app() -> (VectorcraftApp, Arc<Mutex<Vec<String>>>) {
        let written = Arc::new(Mutex::new(vec![]));
        let w = written.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(|_: &crate::FilePick| Some("out.vectorcraft".to_string()))),
            write: Some(Box::new(move |p: &str, _: &[u8]| {
                w.lock().unwrap().push(p.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        (VectorcraftApp::new(Session::new(), services), written)
    }

    fn new_doc(app: &mut VectorcraftApp, dirty: bool) {
        app.run("file.new", json!({ "width": 100, "height": 100 })).unwrap();
        if dirty {
            app.run("shape.rectangle", json!({ "x": 0, "y": 0, "width": 10, "height": 10 })).unwrap();
        }
    }

    fn docs(app: &VectorcraftApp) -> usize {
        app.session.documents().len()
    }

    fn pending(app: &VectorcraftApp) -> Option<usize> {
        app.ui.dialog.as_ref().filter(|d| d.kind == KIND).and_then(|d| d.fields["index"].as_u64()).map(|i| i as usize)
    }

    #[test]
    fn clean_documents_close_at_once_keeping_views_aligned() {
        let (mut app, _) = app();
        for _ in 0..3 {
            new_doc(&mut app, false);
        }
        assert!(!any_dirty(&app));
        app.views[0].zoom = 2.0;
        app.views[2].zoom = 3.0;
        close(&mut app, 1).unwrap();
        assert_eq!(docs(&app), 2);
        assert_eq!(app.views.iter().map(|v| v.zoom).collect::<Vec<_>>(), vec![2.0, 3.0]);
        assert_eq!(app.run("file.close", json!({})).unwrap(), Value::Null);
        assert_eq!(docs(&app), 1);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn closing_a_modified_document_asks_and_cancel_keeps_it() {
        let (mut app, written) = app();
        new_doc(&mut app, true);
        new_doc(&mut app, false);
        assert!(any_dirty(&app));
        assert_eq!(app.run("file.close", json!({ "index": 0 })).unwrap(), json!({ "pending": KIND }));
        assert_eq!((docs(&app), pending(&app), app.session.active_index()), (2, Some(0), Some(0)));
        // Cancel (Esc / Cancel button / ui.dialog.cancel) closes nothing.
        app.ui.dialog = None;
        assert_eq!(docs(&app), 2);
        // Save writes the file (Save As for an untitled document), then closes it.
        app.run("file.close", json!({ "index": 0 })).unwrap();
        confirm(&mut app).unwrap();
        assert_eq!(*written.lock().unwrap(), vec!["out.vectorcraft".to_string()]);
        assert_eq!((docs(&app), app.views.len()), (1, 1));
    }

    #[test]
    fn a_cancelled_save_as_keeps_the_document_open() {
        let (mut app, _) = app();
        app.services.pick_save = Some(Box::new(|_: &crate::FilePick| None));
        new_doc(&mut app, true);
        close(&mut app, 0).unwrap();
        assert!(confirm(&mut app).is_err());
        assert_eq!(docs(&app), 1);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn close_all_and_quit_ask_once_per_modified_document() {
        let (mut app, written) = app();
        new_doc(&mut app, false);
        new_doc(&mut app, true);
        new_doc(&mut app, true);
        // Close All: asks about each modified document in turn, then closes the rest.
        app.run("file.closeAll", json!({})).unwrap();
        assert_eq!(pending(&app), Some(1));
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), pending(&app)), (2, Some(1)));
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), app.views.len(), app.ui.dialog.is_none()), (0, 0, true));
        assert!(written.lock().unwrap().is_empty());
        // Quit: nothing modified quits at once; otherwise only after every answer.
        new_doc(&mut app, true);
        app.run("app.quit", json!({})).unwrap();
        assert_ne!(app.ui.status, "quit");
        assert_eq!(pending(&app), Some(0));
        confirm(&mut app).unwrap();
        assert_eq!(app.ui.status, "quit");
        app.ui.status.clear();
        app.run("app.quit", json!({})).unwrap();
        assert_eq!(app.ui.status, "quit");
    }

    #[test]
    fn the_control_channels_quit_says_when_it_asks_first() {
        // #830: it answered null at once, before the question.
        let (mut app, _) = app();
        let ctx = egui::Context::default();
        let quit = |app: &mut VectorcraftApp| {
            let (req, _rx) = crate::control::ControlRequest::new("app.quit", json!({}));
            let crate::control::Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
            r
        };
        new_doc(&mut app, true);
        assert_eq!(quit(&mut app), json!({"ok": true, "result": {"pending": KIND}}));
        assert_eq!((pending(&app), app.ui.status.as_str()), (Some(0), ""));
        app.ui.dialog = None;
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        assert_eq!(quit(&mut app), json!({"ok": true, "result": null}));
        assert_eq!(app.ui.status, "quit");
    }
}
