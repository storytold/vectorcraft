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
}

/// Each frame: the startup question once, the timer and the heartbeat (`now`: app time in
/// seconds) → in how many seconds they want the next frame (none while Data Recovery is off).
pub fn frame(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    if !app.recovery.checked {
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
    let jobs = match recovery::jobs(&mut app.session, store) {
        // A document whose save or last copy is still being written waits for the next round.
        Ok((jobs, _)) => jobs.into_iter().filter(|j| pick(j.uid) && !app.background.busy_with(j.uid)).collect::<Vec<_>>(),
        Err(e) => {
            app.status(format!("Couldn't save recovery data: {e}"));
            return 0;
        }
    };
    let mut started = 0;
    for job in jobs {
        let (uid, label, title) = (job.uid, format!("Saving recovery data for {}", job.title), job.title.clone());
        let (done, store) = (job.clone(), store.clone());
        let work = move |_: Writer| job.write(store.as_ref());
        let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
            let r = r?;
            done.finish(&mut app.session);
            app.status(format!("Saved recovery data for {title}"));
            Ok(r)
        };
        // Its own store, not the file writer: this only decides whether it runs on the worker.
        if let Err(e) = background::run(app, true, label, Some(uid), work, then) {
            app.status(format!("Couldn't save recovery data: {e}"));
        }
        started += 1;
    }
    started
}

/// The startup dialog's OK (Restore) or Discard: every copy it lists.
pub fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    let id = if d.bool("discard") { "file.recovery.discard" } else { "file.recovery.restore" };
    let files: Vec<String> =
        d.fields.get("copies").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c["file"].as_str()).map(str::to_string).collect();
    let (mut done, mut errors) = (vec![], vec![]);
    for file in files {
        match app.run(id, json!({ "file": file })) {
            Ok(r) => {
                if let Some(restored) = r["restored"].as_array().and_then(|v| v.first()) {
                    crate::dialogs::missing_links::after_open(app, restored);
                }
                done.push(r);
            }
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    app.status(recovered_status(id == "file.recovery.discard", done.len()));
    Ok(json!(done))
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
