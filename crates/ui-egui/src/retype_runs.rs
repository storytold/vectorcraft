//! Retype's search of the installed fonts off the UI thread. Type › Retype and the Retype panel's
//! Find Matching Fonts read installed font files that nothing has loaded yet, which can take
//! seconds. Asked for by the user, the search is prepared at once, so bad params and a selection
//! with nothing to match fail before anything runs, and runs on a thread of its own ([`start`]):
//! the status bar shows it, and after [`DELAY`] a progress window shows how many files are read,
//! with Cancel (also Esc, `retype.cancelSearch`). When it is done, the command runs with the fonts
//! it loaded, unless the document or its selection changed meanwhile: then nothing changes and the
//! status bar says to run it again. The control channel, MCP and scripts run the commands at once,
//! and so does every search on the web, in unit tests that don't set `off_the_ui_thread`, and with
//! `VECTORCRAFT_SYNC_OPEN` set ([`available`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_doc::cancel::{self, CancelToken, ProgressSink};
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::{RetypeFound, RetypeRun};

use crate::VectorcraftApp;
use crate::export_progress::{self, Pressed};

pub use crate::export_progress::DELAY;

/// What the status bar and the progress window say.
const LABEL: &str = "Finding matching fonts";

/// A search running.
pub struct Running {
    id: u64,
    run: RetypeRun,
    /// Its Cancel.
    token: CancelToken,
    /// The Retype panel started it, and lists what its command finds.
    from_panel: bool,
    /// Files read so far and in all (0 in all until the files are listed).
    steps: Arc<[AtomicU64; 2]>,
    done: Receiver<Result<RetypeFound, String>>,
}

/// The search running, if any.
#[derive(Default)]
pub struct RetypeRuns {
    pub running: Option<Running>,
    next_id: u64,
    /// Set by unit tests whose app searches off the UI thread.
    pub(crate) off_the_ui_thread: bool,
}

/// Does `app` search off the UI thread? Where documents open off it ([`crate::opening::available`]),
/// and in the unit tests that set `off_the_ui_thread`.
pub fn available(app: &VectorcraftApp) -> bool {
    app.retype_runs.off_the_ui_thread || crate::opening::available()
}

/// Runs `command` (`type.retype`, or `type.retypeFonts` for the Retype panel when `from_panel`)
/// with params `p`, searching the installed fonts on a thread of its own → `{background: true,
/// status}`. Where no thread can run it, or no installed font needs reading, at once (what the
/// command, or for the panel `retype.find`, returns). Unless its params are bad, it stops the
/// search before it.
pub fn start(app: &mut VectorcraftApp, command: &str, p: Value, from_panel: bool) -> Result<Value, String> {
    let at_once = |app: &mut VectorcraftApp, p: Value| {
        if from_panel {
            let installed = p.get("installed").cloned().unwrap_or(Value::Null);
            app.run("retype.find", json!({ "installed": installed }))
        } else {
            app.run(command, p)
        }
    };
    if !available(app) {
        stop(app);
        return at_once(app, p);
    }
    // Bad params and nothing to match fail here, before anything runs; the search running goes on.
    let prepared = RetypeRun::prepare(&app.session, command, &p).map_err(|e| e.to_string())?;
    // The search before this one stops, so it cannot finish after this request.
    stop(app);
    let Some(run) = prepared else {
        return at_once(app, p);
    };
    let (tx, done) = channel();
    let token = CancelToken::new();
    let scoped = token.clone();
    let steps = Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
    let counts = Arc::clone(&steps);
    let search = run.search();
    std::thread::Builder::new()
        .name("vectorcraft-retype".into())
        .spawn(move || {
            let sink: ProgressSink = std::rc::Rc::new(move |done, total, _| {
                let [done_now, total_now] = &*counts;
                done_now.store(done, Ordering::Relaxed);
                total_now.store(total, Ordering::Relaxed);
            });
            let r = vectorcraft_engine::guard::catch_panic(|| cancel::scope(&scoped, Some(sink), || search.run()))
                .map_err(|msg| format!("internal error: {msg} (please report this bug)"))
                .and_then(|r| r.map_err(|e| e.to_string()));
            // The app may have quit meanwhile: nothing to hand it to.
            let _ = tx.send(r);
        })
        .map_err(|e| e.to_string())?;
    let runs = &mut app.retype_runs;
    runs.next_id += 1;
    runs.running = Some(Running { id: runs.next_id, run, token, from_panel, steps, done });
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    let status = format!("{}…", crate::i18n::msg(LABEL));
    app.status(status.clone());
    Ok(json!({ "background": true, "status": status }))
}

/// Stops the search running, if any, without a word.
fn stop(app: &mut VectorcraftApp) {
    if let Some(r) = app.retype_runs.running.take() {
        r.token.cancel();
        if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
            app.ui.status.clear();
        }
    }
}

/// Runs the command of the search that is done (each frame). Not while a dialog is open, nor while
/// an interaction is open or a tool drags with the pointer button held, as [`crate::trace_runs::poll`].
pub fn poll(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_some() || app.session.in_interaction() || (app.session.tool_busy() && app.session.pointer_held()) {
        return;
    }
    let Some(r) = &app.retype_runs.running else { return };
    let done = match r.done.try_recv() {
        Ok(done) => Some(done),
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => None,
    };
    let Some(r) = app.retype_runs.running.take() else { return };
    if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
        app.ui.status.clear();
    }
    finish(app, r, done);
}

/// Waits for the search running, then does what [`poll`] does once it is done.
#[cfg(test)]
pub(crate) fn wait(app: &mut VectorcraftApp) {
    let Some(r) = app.retype_runs.running.take() else { return };
    let done = r.done.recv().ok();
    if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
        app.ui.status.clear();
    }
    finish(app, r, done);
}

/// Runs search `r`'s command with the fonts it loaded, while its document and selection are as
/// they were; else why not, in the status bar.
fn finish(app: &mut VectorcraftApp, r: Running, done: Option<Result<RetypeFound, String>>) {
    // Cancel changes nothing, even when the search ended just before it.
    if r.token.is_cancelled() {
        app.status(EngineError::Cancelled.to_string());
        return;
    }
    let found = match done {
        Some(Ok(found)) => found,
        Some(Err(e)) => {
            app.status(e);
            return;
        }
        None => {
            app.status("Retype failed: internal error: its thread stopped (please report this bug)");
            return;
        }
    };
    if !r.run.current(&app.session) {
        app.status("Retype finished, but the document or its selection changed meanwhile: run it again");
        return;
    }
    // The command compares the faces found without reading the installed fonts again.
    found.hand_in(&mut app.session);
    let result = if r.from_panel { crate::panels::retype::search(app, r.run.params()) } else { app.run(r.run.command(), r.run.params().clone()) };
    if let Err(e) = result {
        app.status(e);
    }
}

/// The progress window's Cancel (also Esc): the search stops before its next file and changes
/// nothing → `{cancelled: n}`.
pub fn cancel(app: &mut VectorcraftApp) -> Value {
    let n = app.retype_runs.running.as_ref().filter(|r| !r.token.is_cancelled()).map_or(0, |r| {
        r.token.cancel();
        1
    });
    json!({ "cancelled": n })
}

/// Stops the Retype panel's search running, without a word: Reset Retype Panel clears the list
/// the search would fill, and a search `retype.find` runs at once fills it.
pub fn drop_panel_search(app: &mut VectorcraftApp) {
    if app.retype_runs.running.as_ref().is_some_and(|r| r.from_panel)
        && let Some(r) = app.retype_runs.running.take()
    {
        r.token.cancel();
        if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
            app.ui.status.clear();
        }
    }
}

/// Is a search running that hasn't been canceled?
pub fn any_running(app: &VectorcraftApp) -> bool {
    app.retype_runs.running.as_ref().is_some_and(|r| !r.token.is_cancelled())
}

/// The progress window of the search running, once it has run for [`DELAY`]. Not while a dialog
/// or another progress window is open: never two modals at once.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(r) = app.retype_runs.running.as_ref().filter(|r| !r.token.is_cancelled()) else { return };
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|m| {
        let seen = m.get_temp_mut_or_insert_with(egui::Id::unique("retype-run-seen"), || (r.id, now));
        if seen.0 != r.id {
            *seen = (r.id, now);
        }
        seen.1
    });
    let exporting = app.background.jobs.iter().any(|j| j.progress.is_some() && !j.hidden && !j.cancelled());
    let other = exporting
        || crate::plugin_runs::any_shown(app)
        || crate::trace_runs::any_shown(app)
        || crate::opening::any_shown(app)
        || crate::placing::any_shown(app);
    if now - since < DELAY || app.ui.dialog.is_some() || other {
        return;
    }
    let [done, total] = &*r.steps;
    let (done, total) = (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
    let heading = format!("{}…", crate::i18n::msg(LABEL));
    let pressed = export_progress::window_cancel(ctx, "retype-run-progress", &heading, |ui| {
        if total > 0 {
            export_progress::share(ui, tl!("Reading the installed fonts…"), done.min(total) as f32 / total as f32);
        } else {
            export_progress::busy(ui, tl!("Reading the installed fonts…"));
        }
    });
    if pressed == Some(Pressed::Cancel)
        && let Err(e) = app.run("retype.cancelSearch", json!({}))
    {
        app.status(e);
    }
}
