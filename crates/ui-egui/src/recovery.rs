//! Data Recovery in the app: the timer that keeps recovery copies of modified documents every
//! Preferences → File Handling → Data Recovery interval (written in the background like Background
//! Save, the status bar showing it), and the dialog the first frame shows when the last session
//! left copies behind (a crash). The copies themselves are the engine's
//! ([`vectorcraft_engine::cmd::recovery`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_engine::cmd::recovery::{self, RecoveryStore};

use crate::VectorcraftApp;
use crate::background::{self, Writer};
use crate::state::Dialog;

/// Dialog kind of the startup question. Fields: `copies` (`[{file, title, saved}]`, from
/// `file.recovery.list`) and `discard` (Discard instead of Restore).
pub const KIND: &str = "recovery";

/// The timer's state.
#[derive(Default)]
pub struct Timer {
    /// The first frame looked for copies left behind.
    checked: bool,
    /// When copies were last written (app time, s); the interval runs from there.
    last: Option<f64>,
    /// When the heartbeat was last refreshed (app time, s).
    beat: f64,
    /// The recovery folders that failure messages named, latest last, at most [`NAMED`]: replies to
    /// automation clients hide those outside their read roots.
    pub named: Vec<String>,
    /// The restored documents whose linked files are still to be asked about, next first
    /// ([`ask_next`]).
    links: Vec<LinkQuestions>,
}

/// The questions about the linked files of a restored document (by uid): the missing files, then
/// the modified images (ids) to offer for update ([`crate::dialogs::missing_links::ask_next`]).
struct LinkQuestions {
    uid: u64,
    missing: Vec<Value>,
    modified: Vec<Value>,
}

/// How many recovery folders [`Timer::named`] keeps.
const NAMED: usize = 8;

/// Each frame: the startup question once no other dialog is open, the timer and the heartbeat
/// (`now`: app time in seconds) → in how many seconds they want the next frame (none while Data Recovery is off).
pub fn frame(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    // A dialog already open at launch (the Safe Mode choice) is answered first.
    if !app.recovery.checked && app.ui.dialog.is_none() {
        app.recovery.checked = true;
        offer(app);
    }
    tick(app, now);
    if !(0.0..recovery::HEARTBEAT_EVERY).contains(&(now - app.recovery.beat)) {
        app.recovery.beat = now;
        // Copies another app removed while this one's timers were paused (it took this one for
        // gone) are written again at once.
        let lost = recovery::heartbeat(&mut app.session);
        if !lost.is_empty()
            && let Some(store) = recovery::store(&app.session)
        {
            write(app, &store, |uid| lost.contains(&uid));
        }
    }
    let interval = f64::from(app.session.prefs.autosave_interval.max(1)) * 60.0;
    let next = app.recovery.last.map(|t| t + interval - now)?;
    Some(next.min(app.recovery.beat + recovery::HEARTBEAT_EVERY - now).max(1.0))
}

/// Ask about the copies a crash left behind, if any (the Recover Documents dialog). → whether it
/// asked.
pub fn offer(app: &mut VectorcraftApp) -> bool {
    if recovery::store(&app.session).is_none() || app.ui.dialog.is_some() {
        return false;
    }
    let Ok(r) = app.session.execute("file.recovery.list", &json!({})) else { return false };
    // Neither this app's own nor kept by another VectorCraft that is running.
    let left: Vec<Value> = r["copies"].as_array().into_iter().flatten().filter(|c| c["open"] == false && c["running"] == false).cloned().collect();
    if left.is_empty() {
        return false;
    }
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "copies": left })));
    true
}

/// Start a recovery copy of every document that needs one when the interval has passed since the
/// last (the first call only starts the clock) → how many were started.
pub fn tick(app: &mut VectorcraftApp, now: f64) -> usize {
    let prefs = &app.session.prefs;
    let Some(store) = recovery::store(&app.session).filter(|_| prefs.autosave_recovery) else {
        app.recovery.last = None;
        return 0;
    };
    let interval = f64::from(prefs.autosave_interval.max(1)) * 60.0;
    match app.recovery.last {
        Some(t) if now >= t && now - t < interval => return 0,
        Some(t) if now >= t => app.recovery.last = Some(now),
        // First call, or a clock that went back: the interval starts now.
        _ => {
            app.recovery.last = Some(now);
            return 0;
        }
    }
    write(app, &store, |_| true)
}

/// Start a recovery copy of every document that needs one and `pick` picks (by uid) → how many
/// were started.
fn write(app: &mut VectorcraftApp, store: &Arc<dyn RecoveryStore>, pick: impl Fn(u64) -> bool) -> usize {
    recovery::forget_clean(&mut app.session);
    // The folder `store` writes to, named in a failure even when the preference changes meanwhile.
    let place = recovery::folder(&app.session);
    let jobs = match recovery::jobs(&mut app.session, store) {
        // A document whose save or last copy is still being written waits for the next round.
        Ok((jobs, _)) => jobs.into_iter().filter(|j| pick(j.uid) && !app.background.busy_with(j.uid)).collect::<Vec<_>>(),
        Err(e) => {
            failed(app, place.as_ref(), &e.to_string());
            return 0;
        }
    };
    let mut started = 0;
    for job in jobs {
        let (uid, label, title) = (job.uid, format!("Saving recovery data for {}", job.title), job.title.clone());
        let (done, at, saving) = (job.clone(), place.clone(), label.clone());
        let work = move |_: Writer| job.write();
        // A failure is reported here, so the job's result is not an error: that would show as
        // "<label> failed: <error>" over this line, or as the round's message when it ran here.
        let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| match (r, at) {
            (Ok(r), _) => {
                done.finish(&mut app.session);
                app.status(format!("Saved recovery data for {title}"));
                Ok(r)
            }
            (Err(e), Some(at)) => {
                copy_failed(app, &title, &at, &e);
                Ok(Value::Null)
            }
            // A store the host installed has no folder to name: the job's own line.
            (Err(e), None) => {
                app.status(format!("{saving} failed: {e}"));
                log::warn!("vectorcraft: {}", app.ui.status);
                Ok(Value::Null)
            }
        };
        // Its own store, not the file writer: this only decides whether it runs on the worker.
        if let Err(e) = background::run(app, true, label, Some(uid), work, then) {
            failed(app, place.as_ref(), &e);
        }
        started += 1;
    }
    started
}

/// Report a recovery round that couldn't start (error `e`) in the status bar and the log, with
/// the recovery folder in use (`place`, from [`recovery::folder`]) and whether the preference or
/// the app set it.
fn failed(app: &mut VectorcraftApp, place: Option<&(String, bool)>, e: &str) {
    match place {
        Some((folder, true)) => app.status(format!(
            "Couldn't save recovery data: {e}. Copies go to {folder}, the folder set in Preferences; to use another folder, set Preferences → File Handling → Data Recovery → Folder (recoveryFolder)"
        )),
        Some((folder, false)) => app.status(format!(
            "Couldn't save recovery data: {e}. Copies go to {folder}, the default recovery folder; to use another folder, set Preferences → File Handling → Data Recovery → Folder (recoveryFolder)"
        )),
        None => app.status(format!("Couldn't save recovery data: {e}")),
    }
    if let Some((folder, _)) = place {
        named(app, folder);
    }
    log::warn!("vectorcraft: {}", app.ui.status);
}

/// [`failed`] for the copy of document `title` that couldn't be written to folder `place`.
fn copy_failed(app: &mut VectorcraftApp, title: &str, place: &(String, bool), e: &str) {
    match place {
        (folder, true) => app.status(format!(
            "Couldn't save recovery data for {title}: {e}. Copies go to {folder}, the folder set in Preferences; to use another folder, set Preferences → File Handling → Data Recovery → Folder (recoveryFolder)"
        )),
        (folder, false) => app.status(format!(
            "Couldn't save recovery data for {title}: {e}. Copies go to {folder}, the default recovery folder; to use another folder, set Preferences → File Handling → Data Recovery → Folder (recoveryFolder)"
        )),
    }
    named(app, &place.0);
    log::warn!("vectorcraft: {}", app.ui.status);
}

/// Record that the status text named recovery folder `folder` ([`Timer::named`]).
fn named(app: &mut VectorcraftApp, folder: &str) {
    let named = &mut app.recovery.named;
    named.retain(|f| f != folder);
    named.push(folder.to_string());
    if named.len() > NAMED {
        named.remove(0);
    }
}

/// The startup dialog's OK (Restore) or Discard: every copy it lists.
///
/// After Restore the status bar shows how many documents were restored, followed by the first of
/// the restored documents' notes: for each document in the dialog's order, the lines about its
/// linked files, then what reading the copy left out, then its other warnings, each named with
/// its document's title when more than one was restored. Hovering the message lists them all. The
/// questions about the documents' linked files are asked one document at a time ([`ask_next`]).
/// When a copy can't be restored, the status bar shows the error, and its hover lists the count
/// and the notes of the documents that were restored.
pub fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    let discard = d.bool("discard");
    let id = if discard { "file.recovery.discard" } else { "file.recovery.restore" };
    let files: Vec<String> =
        d.fields.get("copies").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c["file"].as_str()).map(str::to_string).collect();
    let (mut done, mut errors, mut opened) = (vec![], vec![], vec![]);
    for file in files {
        match app.run(id, json!({ "file": file })) {
            Ok(r) => {
                for restored in r["restored"].as_array().into_iter().flatten() {
                    opened.push(restored_notes(app, restored));
                }
                let failed = r["failed"].as_array().into_iter().flatten();
                errors.extend(failed.filter_map(|f| f["error"].as_str()).map(str::to_string));
                done.push(r);
            }
            Err(e) => errors.push(e),
        }
    }
    if discard {
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        app.status(recovered_status(true, done.len()));
        return Ok(json!(done));
    }
    let summary = recovered_status(false, opened.len());
    let titled = opened.len() > 1;
    let restored = !opened.is_empty();
    let notes: Vec<String> = opened
        .into_iter()
        .flat_map(|(title, notes)| {
            notes.into_iter().map(move |n| if titled { crate::i18n::fill_once("{path}: {e}", &[("path", &title), ("e", &n)]) } else { n })
        })
        .collect();
    let result = if errors.is_empty() {
        let status = match notes.first() {
            Some(first) => crate::io::opened_after_notice(&summary, notes.len(), first),
            None => summary,
        };
        crate::io::show_with_notes(app, status, notes);
        Ok(json!(done))
    } else {
        let e = errors.join("; ");
        if restored {
            crate::io::show_with_notes(app, e.clone(), std::iter::once(summary).chain(notes).collect());
        }
        Err(e)
    };
    crate::dialogs::settle(app);
    result
}

/// A document Restore just opened (`file.recovery.restore`'s entry `r`, the current document) →
/// its title and notes ([`confirm`]). Its Missing Fonts dialog and the questions about its linked
/// files wait for their turn.
fn restored_notes(app: &mut VectorcraftApp, r: &Value) -> (String, Vec<String>) {
    crate::dialogs::missing_fonts::after_open(app, r);
    let (mut notes, missing, modified) = crate::dialogs::missing_links::questions(app, r);
    let uid = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| app.session.documents().get(i)).map(|st| st.uid);
    if let Some(uid) = uid.filter(|_| !missing.is_empty() || !modified.is_empty()) {
        app.recovery.links.push(LinkQuestions { uid, missing, modified });
    }
    notes.extend(crate::io::open_notes(app, r));
    (r["title"].as_str().unwrap_or_default().to_string(), notes)
}

/// With no dialog open: the questions about the linked files of the next restored document that
/// is still open, with that document made the current one (the answers change it).
pub(crate) fn ask_next(app: &mut VectorcraftApp) {
    while app.ui.dialog.is_none() && !app.recovery.links.is_empty() {
        let q = app.recovery.links.remove(0);
        let Some(i) = app.session.documents().iter().position(|st| st.uid == q.uid) else { continue };
        crate::chrome::show_document(app, i);
        crate::dialogs::missing_links::ask_next(app, q.missing, q.modified, Vec::new());
    }
}

/// The status bar's line for `n` recovered documents discarded or restored.
pub(crate) fn recovered_status(discarded: bool, n: usize) -> String {
    let (one, other) = if discarded {
        ("Discarded {n} recovered document", "Discarded {n} recovered documents")
    } else {
        ("Restored {n} recovered document: save it to keep it", "Restored {n} recovered documents: save them to keep them")
    };
    crate::i18n::msg_n(n as u64, one, other, &[])
}
