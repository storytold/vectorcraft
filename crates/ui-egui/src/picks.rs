//! File dialogs that leave the window running (#592).
//!
//! The app asks for a path where it needs one (Open, Save As, Export, Place, Relink, presets…)
//! and goes on with it at once, so a dialog shown on the UI thread holds the window until it
//! closes. On Windows the dialog runs the window's events meanwhile; on Linux nothing does, so a
//! Wayland compositor finds the window not answering and offers to kill it, and on macOS they
//! re-enter the window's event handler, which crashed the app when the panel was resized (#867).
//! There the host shows dialogs on another thread ([`Services::start_pick`]): asking returns no path for now,
//! and when the dialog answers, what asked (the command, the dialog's OK, a panel's button: the
//! *entry*) runs again and gets the picked path at once.

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

/// A dialog shown off the UI thread, waiting for its answer.
struct Waiting {
    request: PickRequest,
    answer: Receiver<PickAnswer>,
    entry: Entry,
}

/// The app's file dialogs ([`VectorcraftApp`]'s `picks`).
#[derive(Default)]
pub(crate) struct Picks {
    /// What is running now and asks for a path, the outermost one.
    entry: Option<Entry>,
    waiting: Option<Waiting>,
    /// A picked answer for the entry running again: the dialog it asks for gets it.
    answer: Option<(PickRequest, PickAnswer)>,
}

impl Picks {
    /// Nothing asking for file dialogs is running.
    pub(crate) fn is_entry_free(&self) -> bool {
        self.entry.is_none()
    }
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
            let Some(mut d) = app.ui.dialog.take() else { return };
            let r = if d.kind == kind { again(app, &mut d) } else { Ok(()) };
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
    if let (Some(entry), Some(start)) = (app.picks.entry.clone(), app.services.start_pick.as_mut()) {
        if app.picks.waiting.is_some() {
            app.status(tl!("A file dialog is open: choose there first"));
            return None;
        }
        if let Some(answer) = start(request.clone()) {
            app.picks.waiting = Some(Waiting { request, answer, entry });
            return None;
        }
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
            // What asked gave up for now: no "cancelled" while the dialog is open.
            if app.ui.status == "cancelled" {
                app.ui.status.clear();
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        // The dialog's thread is gone: as cancelled.
        Err(TryRecvError::Disconnected) => vec![],
    };
    let Some(Waiting { request, entry, .. }) = app.picks.waiting.take() else { return };
    if answer.is_empty() {
        return;
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
        Entry::Call(f) => {
            f(app);
            Ok(())
        }
    };
    if let Err(e) = r {
        app.status(e);
    }
}
