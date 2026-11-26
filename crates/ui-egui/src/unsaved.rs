//! Closing documents with unsaved changes. File → Close / Close All, a tab's ×, Quit and the
//! window's close button first ask Save / Don't Save / Cancel for each modified document (one
//! question per document); Cancel stops the whole operation and a cancelled Save As keeps the
//! document open.
//!
//! The question is the framework's `UnsavedChangesDialog` on the dialog host, and a `CloseGuard`
//! (`VectorcraftApp::closing`) decides what comes next. Agents answer it like any other dialog:
//! `ui.dialog.set {field: "choice", value: "dontSave"}` then `ui.dialog.confirm`, or
//! `ui.dialog.cancel`. [`pump`] reads the answer each frame.

use craft::quit::{CloseAnswer, CloseGuard, CloseScope, GuardStep};
use craft::ui::dialog::UnsavedChangesDialog;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::SaveMode;

use crate::{VectorcraftApp, io};

/// The `pending` value of the reply to a close that asks first (`{"pending": "saveChanges"}`); it
/// is not a dialog kind.
pub const KIND: &str = "saveChanges";

/// What continues after the asked document is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    /// File → Close or a tab's ×: only the asked document.
    Close,
    /// File → Close All.
    CloseAll,
    /// Quit or the window's close button.
    Quit,
}

impl Then {
    fn scope(self) -> CloseScope {
        if self == Then::Quit { CloseScope::Quit } else { CloseScope::Close }
    }

    /// The name [`close_all`] takes.
    fn name(self) -> &'static str {
        match self {
            Then::Close => "close",
            Then::CloseAll => "closeAll",
            Then::Quit => "quit",
        }
    }
}

/// A close operation that is asking: the guard over the modified documents (by `Document::uid`)
/// and what the operation does once they are answered.
pub(crate) struct Closing {
    guard: CloseGuard<u64>,
    /// For [`Then::Close`], the uid of the document that was in front before the question showed
    /// the asked one: once the asked document closes, that document is in front again while it is
    /// still open.
    front: Option<u64>,
    then: Then,
    /// The document whose save waits on a later frame (its Save As or a replace question).
    saving: Option<u64>,
}

/// Nothing closes while a file dialog's sheet is on the window (macOS): what asked for it runs
/// again when it answers. (Elsewhere the dialog is a window of its own, and closing goes on.)
pub(crate) fn refuse_while_picking(app: &VectorcraftApp) -> Result<(), String> {
    if app.file_sheet_open() { Err(crate::file_asks::busy().into()) } else { Ok(()) }
}

/// Finishes the work that decides whether there is anything left to save: the saves running in
/// the background, and an edit the active tool is computing on another thread (a Shape Builder
/// merge), which is applied.
pub fn finish_pending(app: &mut VectorcraftApp) {
    crate::background::wait_all(app);
    let r = app.session.finish_tool_edit();
    crate::canvas::apply_requests(app, r);
}

/// File → Close (or a tab's ×) for document `i`.
pub fn close(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {
    refuse_while_picking(app)?;
    finish_pending(app);
    let dirty = app.session.documents().get(i).ok_or("no such document")?.is_dirty();
    if dirty { ask(app, i, Then::Close) } else { close_now(app, i, None) }
}

/// File → Close All (`then` = `closeAll`) or Quit (`quit`): ask about the modified documents, or
/// finish once none is left.
pub fn close_all(app: &mut VectorcraftApp, then: &str) -> Result<Value, String> {
    refuse_while_picking(app)?;
    finish_pending(app);
    let then = if then == "quit" { Then::Quit } else { Then::CloseAll };
    if let Some(i) = app.session.documents().iter().position(|d| d.is_dirty()) {
        return ask(app, i, then);
    }
    if then == Then::Quit {
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

/// The index of the open document with `uid`: documents may close while a question is open.
fn position(app: &VectorcraftApp, uid: u64) -> Option<usize> {
    app.session.documents().iter().position(|d| d.uid == uid)
}

/// Starts the operation at document `i` and asks about it. A single close asks only about `i`;
/// Close All and Quit ask about every modified document from `i` on.
fn ask(app: &mut VectorcraftApp, i: usize, then: Then) -> Result<Value, String> {
    let docs = app.session.documents();
    let uid = docs.get(i).ok_or("no such document")?.uid;
    let uids: Vec<u64> = if then == Then::Close { vec![uid] } else { docs.iter().skip(i).filter(|d| d.is_dirty()).map(|d| d.uid).collect() };
    let front = app.session.active().map(|d| d.uid).filter(|f| then == Then::Close && *f != uid);
    let guard = CloseGuard::new(then.scope(), uids);
    show_question(app, &guard, uid)?;
    app.closing = Some(Closing { guard, front, then, saving: None });
    Ok(json!({ "pending": KIND }))
}

/// Shows document `uid` and opens the question about it on the dialog host.
fn show_question(app: &mut VectorcraftApp, guard: &CloseGuard<u64>, uid: u64) -> Result<(), String> {
    let i = position(app, uid).ok_or("no such document")?;
    app.session.set_active(i);
    let name = app.session.documents().get(i).map(|d| d.title()).ok_or("no such document")?;
    let (scope, remaining) = (guard.scope(), guard.remaining());
    // PLAN: assumed `VectorcraftApp::dialogs() -> Option<handle>` whose `with(|host: &mut DialogHost| ..)` runs on the host (wg.S1).
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|h| {
        h.open(UnsavedChangesDialog::new(scope, name).remaining(remaining));
    });
    Ok(())
}

/// Close document `i` through the engine command; [`VectorcraftApp::sync_views`] drops its view.
/// When `front` names a document that is still open, that document is in front afterwards.
fn close_now(app: &mut VectorcraftApp, i: usize, front: Option<u64>) -> Result<Value, String> {
    let r = app.session.execute("file.close", &json!({ "index": i })).map_err(|e| e.to_string())?;
    if let Some(j) = front.and_then(|uid| position(app, uid)) {
        app.session.set_active(j);
    }
    app.sync_views();
    if app.session.tool_id() == "artboard"
        && let Some(n) = app.session.active().map(|d| d.doc.artboards.len())
    {
        // Closing recreates the tool; restore the surviving document's displayed board.
        let selected = crate::panels::artboards::selected(app, n);
        crate::panels::artboards::select(app, selected);
    }
    Ok(r)
}

/// Saves document `i` and closes it. `false` when the save asked first (replacing a file that
/// would lose what opening it left out): the document stays open and the guard waits.
fn save_then_close(app: &mut VectorcraftApp, i: usize, front: Option<u64>) -> Result<bool, String> {
    app.session.set_active(i);
    let r = io::save(app, SaveMode::Save, &json!({}), false)?;
    if r.get("pending").is_some() {
        return Ok(false);
    }
    // Closing needs the file written: wait for a background save, and stop if it failed.
    if r["background"] == true {
        crate::background::wait_all(app);
        if let Some(d) = app.session.documents().get(i).filter(|d| d.is_dirty()) {
            return Err(format!("{}: not saved, so not closed", d.title()));
        }
    }
    close_now(app, i, front)?;
    Ok(true)
}

/// Reads the answer to the open question and carries the operation on. An error stops the
/// operation (the document stays open) and is reported in the status line.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.closing.is_none() {
        return;
    }
    if let Some(uid) = app.closing.as_ref().and_then(|c| c.saving) {
        finish_save(app, uid);
        ctx.request_repaint();
        return;
    }
    let Some(result) = app.dialogs().and_then(|d| d.with(|h| h.take::<UnsavedChangesDialog>())) else { return };
    let Some(mut closing) = app.closing.take() else { return };
    // Closed from outside: the same as Cancel.
    let (answer, all) = result.ok().map_or((CloseAnswer::Cancel, false), |a| (a.answer, a.apply_to_all));
    let step = if all { closing.guard.answer_all(answer) } else { closing.guard.answer(answer) };
    advance(app, closing, step);
    ctx.request_repaint();
}

/// The save of document `uid` that a Save answer started and that waited on a later frame: once the
/// document is saved it closes and the operation goes on; while its dialog or write is still running
/// it waits; given up (its Save As cancelled) the document stays open and the operation stops.
fn finish_save(app: &mut VectorcraftApp, uid: u64) {
    let doc = app.session.documents().iter().find(|d| d.uid == uid);
    let saved = doc.is_none_or(|d| !d.is_dirty());
    let asking = app.dialogs().is_some_and(|d| d.with(|h| !h.open_dialogs().is_empty()));
    if !saved && (app.file_dialog_open() || asking || app.background.busy_with(uid)) {
        return;
    }
    let title = doc.map(|d| d.title()).unwrap_or_default();
    let Some(mut closing) = app.closing.take() else { return };
    closing.saving = None;
    let step = match position(app, uid) {
        Some(i) if saved => match close_now(app, i, closing.front) {
            Ok(_) => closing.guard.saved(),
            Err(e) => {
                app.status(e);
                return;
            }
        },
        None => closing.guard.saved(),
        Some(_) => {
            app.status(format!("{title}: not saved, so not closed"));
            closing.guard.save_failed()
        }
    };
    advance(app, closing, step);
}

/// Acts on `step` until the guard asks a question again, waits, or stops.
fn advance(app: &mut VectorcraftApp, mut closing: Closing, mut step: GuardStep<u64>) {
    loop {
        step = match step {
            GuardStep::Cancelled => return,
            GuardStep::Finished => {
                if closing.then != Then::Close
                    && let Err(e) = close_all(app, closing.then.name())
                {
                    app.status(e);
                }
                return;
            }
            GuardStep::Ask(uid) => match position(app, uid) {
                // Closed meanwhile: there is nothing to ask about.
                None => closing.guard.discarded(),
                Some(_) => {
                    if let Err(e) = show_question(app, &closing.guard, uid) {
                        app.status(e);
                        return;
                    }
                    app.closing = Some(closing);
                    return;
                }
            },
            GuardStep::Discard(uid) => {
                if let Some(i) = position(app, uid)
                    && let Err(e) = close_now(app, i, closing.front)
                {
                    app.status(e);
                    return;
                }
                closing.guard.discarded()
            }
            GuardStep::Save(uid) => match position(app, uid) {
                None => closing.guard.saved(),
                Some(i) => match save_then_close(app, i, closing.front) {
                    Ok(true) => closing.guard.saved(),
                    // Save As or the replace-file question is open: the guard stays in `saving`.
                    Ok(false) => {
                        closing.saving = Some(uid);
                        app.closing = Some(closing);
                        return;
                    }
                    Err(e) => {
                        app.status(e);
                        return;
                    }
                },
            },
        };
    }
}

/// One frame of the app, so it has its dialog host.
#[cfg(test)]
pub(crate) fn ready_for_test(app: &mut VectorcraftApp) -> egui::Context {
    let ctx = crate::testing::ui_context();
    crate::testing::app_frame(app, &ctx, egui::RawInput::default());
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_picks::{Picker, poll, scripted};
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), crate::Services::default())
    }

    /// The window's first frame, then a file service whose picks the test answers and a writer that
    /// records the paths written.
    fn ready_with_picks(app: &mut VectorcraftApp) -> (egui::Context, Picker) {
        let ctx = ready_for_test(app);
        let picker = scripted(app);
        (ctx, picker)
    }

    /// Save As picks `out.vectorcraft`, and what it asked runs.
    fn save_as_out(app: &mut VectorcraftApp, ctx: &egui::Context, picker: &Picker) {
        picker.answer_saved("out.vectorcraft", None);
        poll(app, ctx);
        pump(app, ctx);
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

    /// The index of the document the open question asks about.
    fn pending(app: &VectorcraftApp) -> Option<usize> {
        let uid = *app.closing.as_ref()?.guard.current()?;
        position(app, uid)
    }

    fn question_open(app: &VectorcraftApp) -> bool {
        app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<UnsavedChangesDialog>()))
    }

    /// Answers the open question on the host as its button would, then runs the pump.
    fn answer(app: &mut VectorcraftApp, ctx: &egui::Context, answer: CloseAnswer) {
        app.dialogs().unwrap().with(|h| {
            h.get_mut::<UnsavedChangesDialog>().unwrap().set_answer(answer);
            assert!(h.confirm::<UnsavedChangesDialog>(ctx), "a question is open");
        });
        pump(app, ctx);
    }

    /// Three documents, A clean, B modified and C clean, with A in front: their uids.
    fn front_and_modified_background_tab(app: &mut VectorcraftApp) -> [u64; 3] {
        new_doc(app, false);
        new_doc(app, true);
        new_doc(app, false);
        app.session.set_active(0);
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        [uids[0], uids[1], uids[2]]
    }

    fn front_uid(app: &VectorcraftApp) -> Option<u64> {
        app.session.active().map(|d| d.uid)
    }

    #[test]
    fn clean_documents_close_at_once_keeping_views_aligned() {
        let mut app = app();
        for _ in 0..3 {
            new_doc(&mut app, false);
        }
        let _ctx = ready_for_test(&mut app);
        assert!(!any_dirty(&app));
        app.views[0].zoom = 2.0;
        app.views[2].zoom = 3.0;
        close(&mut app, 1).unwrap();
        assert_eq!(docs(&app), 2);
        assert_eq!(app.views.iter().map(|v| v.zoom).collect::<Vec<_>>(), vec![2.0, 3.0]);
        assert_eq!(app.run("file.close", json!({})).unwrap(), Value::Null);
        assert_eq!(docs(&app), 1);
        assert!(!question_open(&app));
    }

    #[test]
    fn closing_a_modified_document_asks_and_cancel_keeps_it() {
        let mut app = app();
        new_doc(&mut app, true);
        new_doc(&mut app, false);
        let (ctx, picker) = ready_with_picks(&mut app);
        assert!(any_dirty(&app));
        assert_eq!(app.run("file.close", json!({ "index": 0 })).unwrap(), json!({ "pending": KIND }));
        assert_eq!((docs(&app), pending(&app), app.session.active_index()), (2, Some(0), Some(0)));
        assert!(question_open(&app));
        answer(&mut app, &ctx, CloseAnswer::Cancel);
        assert_eq!(docs(&app), 2);
        assert!(app.closing.is_none() && !question_open(&app));
        // Save writes the file (Save As for an untitled document), then closes it.
        app.run("file.close", json!({ "index": 0 })).unwrap();
        answer(&mut app, &ctx, CloseAnswer::Save);
        save_as_out(&mut app, &ctx, &picker);
        assert_eq!(picker.written(), vec!["out.vectorcraft".to_string()]);
        assert_eq!((docs(&app), app.views.len()), (1, 1));
    }

    #[test]
    fn closing_a_modified_background_tab_keeps_the_front_document_after_dont_save() {
        let mut app = app();
        let [a, _, c] = front_and_modified_background_tab(&mut app);
        let (ctx, picker) = ready_with_picks(&mut app);
        assert_eq!(app.run("file.close", json!({ "index": 1 })).unwrap(), json!({ "pending": KIND }));
        // The question shows the document it asks about.
        assert_eq!((pending(&app), app.session.active_index()), (Some(1), Some(1)));
        answer(&mut app, &ctx, CloseAnswer::DontSave);
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        assert_eq!((uids, front_uid(&app), app.views.len()), (vec![a, c], Some(a), 2));
        assert!(picker.written().is_empty());
    }

    #[test]
    fn closing_a_modified_background_tab_keeps_the_front_document_after_save() {
        let mut app = app();
        let [a, _, c] = front_and_modified_background_tab(&mut app);
        let (ctx, picker) = ready_with_picks(&mut app);
        close(&mut app, 1).unwrap();
        answer(&mut app, &ctx, CloseAnswer::Save);
        save_as_out(&mut app, &ctx, &picker);
        assert_eq!(picker.written(), vec!["out.vectorcraft".to_string()]);
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        assert_eq!((uids, front_uid(&app)), (vec![a, c], Some(a)));
    }

    #[test]
    fn cancel_keeps_the_asked_background_tab_in_front() {
        let mut app = app();
        let [_, b, _] = front_and_modified_background_tab(&mut app);
        let ctx = ready_for_test(&mut app);
        close(&mut app, 1).unwrap();
        answer(&mut app, &ctx, CloseAnswer::Cancel);
        assert_eq!((docs(&app), front_uid(&app)), (3, Some(b)));
        // A later close of the tab in front brings its neighbor to the front.
        close(&mut app, 1).unwrap();
        answer(&mut app, &ctx, CloseAnswer::DontSave);
        assert_eq!((docs(&app), app.session.active_index()), (2, Some(1)));
    }

    /// The document in front before the question is remembered by its uid: when it closed while
    /// the question was open, the closed document's neighbor comes to the front as before.
    #[test]
    fn a_front_document_closed_meanwhile_is_not_brought_back() {
        let mut app = app();
        let [a, _, c] = front_and_modified_background_tab(&mut app);
        let ctx = ready_for_test(&mut app);
        close(&mut app, 1).unwrap();
        assert_eq!(app.closing.as_ref().and_then(|c| c.front), Some(a));
        close(&mut app, 0).unwrap();
        answer(&mut app, &ctx, CloseAnswer::DontSave);
        assert_eq!((docs(&app), front_uid(&app)), (1, Some(c)));
    }

    #[test]
    fn close_all_does_not_remember_a_front_document() {
        let mut app = app();
        front_and_modified_background_tab(&mut app);
        let ctx = ready_for_test(&mut app);
        app.run("file.closeAll", json!({})).unwrap();
        assert!(question_open(&app) && app.closing.as_ref().is_some_and(|c| c.front.is_none()));
        answer(&mut app, &ctx, CloseAnswer::DontSave);
        assert_eq!((docs(&app), app.views.len()), (0, 0));
        assert!(app.closing.is_none());
    }

    #[test]
    fn a_cancelled_save_as_keeps_the_document_open() {
        let mut app = app();
        new_doc(&mut app, true);
        let (ctx, picker) = ready_with_picks(&mut app);
        close(&mut app, 0).unwrap();
        answer(&mut app, &ctx, CloseAnswer::Save);
        picker.cancel();
        poll(&mut app, &ctx);
        pump(&mut app, &ctx);
        assert_eq!(docs(&app), 1);
        assert!(app.closing.is_none() && !question_open(&app));
        assert!(!app.ui.status.is_empty(), "the failure is reported");
    }

    #[test]
    fn close_all_and_quit_ask_once_per_modified_document() {
        let mut app = app();
        new_doc(&mut app, false);
        new_doc(&mut app, true);
        new_doc(&mut app, true);
        let (ctx, picker) = ready_with_picks(&mut app);
        // Close All: asks about each modified document in turn, then closes the rest.
        app.run("file.closeAll", json!({})).unwrap();
        assert_eq!(pending(&app), Some(1));
        answer(&mut app, &ctx, CloseAnswer::DontSave);
        assert_eq!((docs(&app), pending(&app)), (2, Some(1)));
        answer(&mut app, &ctx, CloseAnswer::DontSave);
        assert_eq!((docs(&app), app.views.len(), question_open(&app)), (0, 0, false));
        assert!(picker.written().is_empty());
        // Quit: nothing modified quits at once; otherwise only after every answer.
        new_doc(&mut app, true);
        app.run("app.quit", json!({})).unwrap();
        assert_ne!(app.ui.status, "quit");
        assert_eq!(pending(&app), Some(0));
        answer(&mut app, &ctx, CloseAnswer::Save);
        save_as_out(&mut app, &ctx, &picker);
        assert_eq!(app.ui.status, "quit");
        app.ui.status.clear();
        app.run("app.quit", json!({})).unwrap();
        assert_eq!(app.ui.status, "quit");
    }

    #[test]
    fn the_control_channels_quit_says_when_it_asks_first() {
        // #830: it answered null at once, before the question.
        let mut app = app();
        new_doc(&mut app, true);
        let (ctx, _picker) = ready_with_picks(&mut app);
        let quit = |app: &mut VectorcraftApp| {
            let (req, _rx) = craft::automation::ControlRequest::new("app.quit", json!({}));
            let craft::automation::Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
            r
        };
        assert_eq!(quit(&mut app), json!({"ok": true, "result": {"pending": KIND}}));
        assert_eq!((pending(&app), app.ui.status.as_str()), (Some(0), ""));
        app.closing = None;
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        assert_eq!(quit(&mut app), json!({"ok": true, "result": null}));
        assert_eq!(app.ui.status, "quit");
    }
}
