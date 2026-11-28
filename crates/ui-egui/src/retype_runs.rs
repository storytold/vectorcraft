//! Retype's search of the installed fonts off the UI thread. Type › Retype and the Retype panel's
//! Find Matching Fonts read installed font files that nothing has loaded yet, which can take
//! seconds. Asked for by the user, the search is prepared at once, so bad params and a selection
//! with nothing to match fail before anything runs, and runs on a thread of its own ([`start`]):
//! the status bar shows it, and after the progress window's delay
//! ([`crate::export_progress::DELAY`]) the window shows how many files are read, with Cancel (also
//! Esc, `retype.cancelSearch`). When it is done, the command runs with the fonts it loaded, unless
//! the document or its selection changed meanwhile: then nothing changes and the status bar says to
//! run it again. The control channel, MCP and scripts run the commands at once, and so does every
//! search on the web, in unit tests that don't set `off_the_ui_thread`, and with
//! `VECTORCRAFT_SYNC_OPEN` set ([`available`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_doc::cancel::{self, CancelToken, ProgressSink};
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::{RetypeFound, RetypeRun};

use crate::{VectorcraftApp, export_progress};

/// What the status bar and the progress window say.
const LABEL: &str = "Finding matching fonts";

/// A search running.
pub struct Running {
    /// [`crate::export_progress::now`] when it started: the progress window's delay counts from it.
    pub(crate) started: f64,
    run: RetypeRun,
    /// Its Cancel.
    token: CancelToken,
    /// The Retype panel started it, and lists what its command finds.
    from_panel: bool,
    /// Files read so far and in all (0 in all until the files are listed).
    steps: Arc<[AtomicU64; 2]>,
    done: Receiver<Result<RetypeFound, String>>,
}

impl Running {
    /// The share of the files read, once they are listed.
    fn fraction(&self) -> Option<f32> {
        let [done, total] = &*self.steps;
        let (done, total) = (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
        (total > 0).then(|| done.min(total) as f32 / total as f32)
    }
}

/// The search running, if any.
#[derive(Default)]
pub struct RetypeRuns {
    pub running: Option<Running>,
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
    let started = export_progress::now(app);
    app.retype_runs.running = Some(Running { started, run, token, from_panel, steps, done });
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
    if app.any_dialog_open() || app.session.in_interaction() || (app.session.tool_busy() && app.session.pointer_held()) {
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
    // A command that failed before it took the faces found leaves them for no later command.
    RetypeFound::drop_handed(&mut app.session);
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

/// The progress window of the search running; `export_progress::show` decides when it shows and
/// runs `retype.cancelSearch` when Cancel is pressed. The search has no Continue in Background.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<export_progress::Shown> {
    let r = app.retype_runs.running.as_ref().filter(|r| !r.token.is_cancelled())?;
    Some(export_progress::Shown {
        title: format!("{}…", crate::i18n::msg(LABEL)),
        step: tl!("Reading the installed fonts…").to_string(),
        fraction: r.fraction(),
        counts: None,
        queued: None,
        started: r.started,
        cancel: "retype.cancelSearch",
        hide: None,
    })
}
