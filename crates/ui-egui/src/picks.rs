//! File dialogs that leave the window running (#592, #867).
//!
//! The app asks for a path where it needs one (Open, Save As, Export, Place, Relink, presets…)
//! and goes on with it at once, so a dialog shown on the UI thread holds the window until it
//! closes. On Windows the dialog runs the window's events meanwhile. On Linux nothing does, so a
//! Wayland compositor finds the window not answering and offers to kill it (#592); on macOS the
//! panel's modal loop runs inside winit's event handler, and an event winit had held back meanwhile
//! re-enters it and aborts the app (#867). There the host shows dialogs off the UI thread (Linux)
//! or as sheets (macOS) ([`Services::start_pick`]): asking returns no path for now, and when the
//! dialog answers, what asked (the command, the dialog's OK, a panel's button: the *entry*) runs
//! again, on the document it asked for, and gets the picked path at once. Everything that asks
//! must run as an entry ([`as_entry`]): outside one the dialog is shown in line, which macOS
//! refuses.
//!
//! While a dialog is open ([`VectorcraftApp::file_dialog_open`]) another is refused; on macOS,
//! where it is a sheet on the window, the menu bar does nothing and the app neither quits nor
//! closes documents, as under the modal dialog before.

use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::Value;

use crate::state::Dialog;
use crate::{FilePick, VectorcraftApp};

/// A file dialog to show.
#[derive(Clone, Debug, PartialEq)]
pub enum PickRequest {
    /// An open dialog for one file.
    Open(FilePick),
    /// An open dialog for files to place.
    OpenMany,
    /// A save dialog.
    Save(FilePick),
    /// A folder picker.
    Folder,
}

/// The paths a dialog shown off the UI thread picked (none: cancelled).
pub type PickAnswer = Vec<String>;

/// Shows a dialog off the UI thread; its answer arrives on the receiver. `None` when it can't, and
/// the dialog is then shown in line.
pub type StartPick = Box<dyn FnMut(PickRequest) -> Option<Receiver<PickAnswer>>>;

/// What runs again with the picked path.
#[derive(Clone)]
pub(crate) enum Entry {
    /// A command (`app.run`).
    Command(String, Value),
    /// A dialog's OK (`dialogs::confirm`), on the dialog as it was.
    Confirm(Box<Dialog>),
    /// A panel's or a dialog's button.
    Call(Rc<dyn Fn(&mut VectorcraftApp)>),
}

/// Commands that ask for a file but act on no document: their answer goes on even when the
/// document active at the time has closed meanwhile.
const DOCUMENT_FREE: &[&str] = &[
    "file.open",
    "file.newFromTemplate",
    "ui.installPlugin",
    "window.swatchLibrary.other",
    "window.graphicStyleLibrary.other",
    "shortcuts.import",
    "shortcuts.export",
];

impl Entry {
    /// The document it acts on when it runs again: the active one, unless it acts on none.
    fn document(&self, app: &VectorcraftApp) -> Option<u64> {
        match self {
            Entry::Command(id, _) if DOCUMENT_FREE.contains(&id.as_str()) => None,
            _ => app.session.active().map(|d| d.uid),
        }
    }
}

/// A dialog shown off the UI thread, waiting for its answer.
struct Waiting {
    request: PickRequest,
    answer: Receiver<PickAnswer>,
    entry: Entry,
    /// The active document when it was asked for ([`vectorcraft_engine::DocState::uid`]).
    doc: Option<u64>,
}

/// The app's file dialogs ([`VectorcraftApp`]'s `picks`).
#[derive(Default)]
pub(crate) struct Picks {
    /// What is running now and asks for a path, the outermost one.
    entry: Option<Entry>,
    waiting: Option<Waiting>,
    /// A picked answer for the entry running again: the dialog it asks for gets it.
    answer: Option<(PickRequest, PickAnswer)>,
    /// Another dialog was refused while this one is open ([`busy`]).
    refused: bool,
}

impl Picks {
    /// Nothing asking for file dialogs is running.
    pub(crate) fn is_entry_free(&self) -> bool {
        self.entry.is_none()
    }

    /// A dialog shown off the UI thread is open.
    pub(crate) fn is_waiting(&self) -> bool {
        self.waiting.is_some()
    }

    /// After a panic the guard caught (a bug): no entry is running any more, and an answer kept for
    /// one is stale. The open dialog's answer still arrives.
    pub(crate) fn recover(&mut self) {
        self.entry = None;
        self.answer = None;
    }

    /// The kind of dialog open off the UI thread (`ui.inspect`'s `fileDialog`).
    pub(crate) fn waiting_kind(&self) -> Option<&'static str> {
        self.waiting.as_ref().map(|w| match w.request {
            PickRequest::Open(_) => "open",
            PickRequest::OpenMany => "place",
            PickRequest::Save(_) => "save",
            PickRequest::Folder => "folder",
        })
    }
}

/// What the status bar says when a dialog is asked for while another is open.
pub(crate) fn busy() -> &'static str {
    tl!("A file dialog is open: choose there first")
}

/// What the status bar says when the dialog whose button asked closed before the file dialog
/// answered.
pub(crate) fn dialog_gone() -> &'static str {
    tl!("The dialog was closed before its file dialog answered")
}

/// Run `f` as `entry`: a dialog it asks for off the UI thread runs `entry` again when it
/// answers. Inside another entry, that one runs again instead.
pub(crate) fn as_entry<R>(app: &mut VectorcraftApp, entry: impl FnOnce() -> Entry, f: impl FnOnce(&mut VectorcraftApp) -> R) -> R {
    if app.picks.entry.is_some() {
        return f(app);
    }
    app.picks.entry = Some(entry());
    let r = f(app);
    app.picks.entry = None;
    r
}

/// [`as_entry`] for a panel's or a dialog's button that `f` handles.
pub(crate) fn button(app: &mut VectorcraftApp, f: impl Fn(&mut VectorcraftApp) + 'static) {
    let f = Rc::new(f);
    let again = f.clone();
    as_entry(app, move || Entry::Call(again), |app| f(app));
}

/// A dialog body's action `f` on `d` (the open dialog's copy) that asks for a file dialog: shown
/// off the UI thread, its answer runs `f` again on the open dialog, when it is still of that kind.
pub(crate) fn in_dialog(
    app: &mut VectorcraftApp,
    d: &mut Dialog,
    f: impl Fn(&mut VectorcraftApp, &mut Dialog) -> Result<(), String> + 'static,
) -> Result<(), String> {
    let f = Rc::new(f);
    let (again, kind) = (f.clone(), d.kind.clone());
    let entry = move || {
        Entry::Call(Rc::new(move |app: &mut VectorcraftApp| {
            let Some(mut d) = app.ui.dialog.take_if(|d| d.kind == kind) else {
                app.status(dialog_gone());
                return;
            };
            let r = again(app, &mut d);
            app.ui.dialog = Some(d);
            if let Err(e) = r {
                app.status(e);
            }
        }))
    };
    as_entry(app, entry, |app| f(app, d))
}

/// A dialog body's folder button: the folder picked into field `key` of `d` ([`in_dialog`]).
pub(crate) fn folder_field(app: &mut VectorcraftApp, d: &mut Dialog, key: &'static str) {
    let r = in_dialog(app, d, move |app, d| {
        if let Some(f) = folder(app) {
            d.fields.insert(key.into(), serde_json::json!(f));
        }
        Ok(())
    });
    // Picking a folder doesn't fail.
    let _ = r;
}

/// A path from an open dialog for `pick` (none: cancelled, or shown off the UI thread for now).
pub(crate) fn open(app: &mut VectorcraftApp, pick: &FilePick) -> Option<String> {
    ask(app, PickRequest::Open(pick.clone()))?.into_iter().next()
}

/// A path from a save dialog for `pick`.
pub(crate) fn save(app: &mut VectorcraftApp, pick: &FilePick) -> Option<String> {
    ask(app, PickRequest::Save(pick.clone()))?.into_iter().next()
}

/// A folder from the folder picker.
pub(crate) fn folder(app: &mut VectorcraftApp) -> Option<String> {
    ask(app, PickRequest::Folder)?.into_iter().next()
}

/// Paths from the open dialog for files to place (none: cancelled).
pub(crate) fn open_many(app: &mut VectorcraftApp) -> Vec<String> {
    ask(app, PickRequest::OpenMany).unwrap_or_default()
}

/// Whether the host shows `request`'s kind of dialog at all.
pub(crate) fn can(app: &VectorcraftApp, request: &PickRequest) -> bool {
    let s = &app.services;
    match request {
        PickRequest::Open(_) => s.pick_open.is_some(),
        PickRequest::OpenMany => s.pick_open_multi.is_some() || s.pick_open.is_some(),
        PickRequest::Save(_) => s.pick_save.is_some(),
        PickRequest::Folder => s.pick_folder.is_some(),
    }
}

/// The paths `request`'s dialog picked: the answer kept for it, else shown off the UI thread (no
/// paths for now), else shown in line.
fn ask(app: &mut VectorcraftApp, request: PickRequest) -> Option<PickAnswer> {
    // One for another dialog is dropped: the entry took another way this time.
    if let Some((asked, answer)) = app.picks.answer.take()
        && asked == request
    {
        return Some(answer);
    }
    if !can(app, &request) {
        return None;
    }
    if app.services.start_pick.is_some() && app.picks.waiting.is_some() {
        app.status(busy());
        app.picks.refused = true;
        return None;
    }
    match (app.picks.entry.clone(), app.services.start_pick.as_mut()) {
        (Some(entry), Some(start)) => {
            if let Some(answer) = start(request.clone()) {
                let doc = entry.document(app);
                app.picks.waiting = Some(Waiting { request, answer, entry, doc });
                return None;
            }
        }
        // Shown in line it holds the window (Linux) or is refused (macOS): what asks must run as
        // an entry.
        (None, Some(_)) => log::warn!("a file dialog asked for outside a command, a dialog's OK or a button: {request:?}"),
        _ => {}
    }
    let s = &mut app.services;
    let one = |p: Option<String>| Some(p.into_iter().collect());
    match &request {
        PickRequest::Open(pick) => one(s.pick_open.as_mut().and_then(|f| f(pick))),
        PickRequest::Save(pick) => one(s.pick_save.as_mut().and_then(|f| f(pick))),
        PickRequest::Folder => one(s.pick_folder.as_mut().and_then(|f| f())),
        PickRequest::OpenMany => match (s.pick_open_multi.as_mut(), s.pick_open.as_mut()) {
            (Some(many), _) => Some(many()),
            (None, Some(f)) => one(f(&FilePick::default())),
            (None, None) => None,
        },
    }
}

/// Each frame: when the dialog shown off the UI thread answers with paths, run its entry again
/// with them. While it is open, look again shortly.
pub(crate) fn poll(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(w) = &app.picks.waiting else { return };
    let answer = match w.answer.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => {
            // What asked gave up for now: no "cancelled" while the dialog is open, unless it was
            // another dialog, refused.
            if app.ui.status == "cancelled" {
                app.ui.status = if std::mem::take(&mut app.picks.refused) { busy().into() } else { String::new() };
            }
            // The host wakes the window when the dialog answers; this is in case it can't.
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            return;
        }
        // The dialog's thread is gone: as cancelled.
        Err(TryRecvError::Disconnected) => vec![],
    };
    let Some(Waiting { request, entry, doc, .. }) = app.picks.waiting.take() else { return };
    app.picks.refused = false;
    // Once more: what waited for the dialog (files from the Finder, the menu bar) goes on.
    ctx.request_repaint();
    if answer.is_empty() {
        return;
    }
    // On the document it was asked for: another may be active by now (opened from the Finder, by
    // an agent), or it may be gone.
    if let Some(uid) = doc {
        let Some(i) = app.session.documents().iter().position(|d| d.uid == uid) else {
            app.status(tl!("The document was closed while its file dialog was open"));
            return;
        };
        if app.session.active_index() != Some(i)
            && let Err(e) = app.run("document.activate", serde_json::json!({ "index": i }))
        {
            app.status(e);
            return;
        }
    }
    app.picks.answer = Some((request, answer));
    rerun(app, entry);
    // An entry that took another way this time leaves the answer unused.
    app.picks.answer = None;
}

/// Run `entry` again.
fn rerun(app: &mut VectorcraftApp, entry: Entry) {
    let r = match entry {
        Entry::Command(id, params) => app.run(&id, params).map(|_| ()),
        Entry::Confirm(d) => {
            app.ui.dialog = Some(*d);
            crate::dialogs::confirm(app).map(|_| ())
        }
        // An entry again, so a dialog it asks for next is shown off the UI thread too.
        Entry::Call(f) => {
            let again = f.clone();
            as_entry(app, move || Entry::Call(again), |app| f(app));
            Ok(())
        }
    };
    if let Err(e) = r {
        app.status(e);
    }
}
