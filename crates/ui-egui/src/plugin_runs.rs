//! Heavy plug-in filters, run off the UI thread. A filter whose manifest asks for a heavy budget
//! (`Budget::Heavy`: minutes rather than a minute, for routing, packing or simulation) would freeze
//! the app while it computes, so the app runs it on a thread of its own ([`start`]) while editing
//! goes on: the status bar shows it, and after the export progress window's delay a progress window with Cancel (also Esc,
//! `plugin.cancelRuns`) and Continue in Background (`plugin.hideRuns`). A thread of its own, not
//! the background worker's queue ([`crate::background`]), so saves and exports don't wait behind it.
//!
//! When it is done its output is handed to `plugin.run` with the same parameters and objects
//! ([`FilterRun::hand_in`]), so the edit is one undo step like any run, unless the document or the
//! plug-in changed meanwhile: then nothing changes and the status bar says to run it again. On the
//! web (no threads) it runs at once ([`available`]).
//!
//! A filter of plug-in API level 2 may report how far it has got (`vc_progress`): the progress
//! window then shows a bar with the share done. Its Cancel also makes the filter's `vc_cancelled`
//! return 1, so a filter that checks it stops at once; one that doesn't stops at the next slice of
//! its budget. A light filter that may report its progress (the `progress` permission) and calls
//! no commands runs here too, under its light budget, so its progress and Cancel show while it
//! computes.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::fileio::progress::{self, Progress};
use vectorcraft_engine::cmd::plugin::FilterRun;
use vectorcraft_plugins::{permissions, registry};

use crate::VectorcraftApp;
use crate::export_progress;

/// What a run's thread hands back: the run, and its output or why it has none.
type Done = (FilterRun, Result<Vec<u8>, String>);

/// A heavy filter running.
pub struct Running {
    /// What the status bar and the progress window say: `Running “Route Paths”…`.
    pub label: String,
    /// The filter's name, quoted.
    who: String,
    /// `plugin.run`'s params that apply its output: the objects it was given as `ids`.
    params: Value,
    /// How far it has got, as it reports it, and its Cancel.
    pub progress: Arc<Progress>,
    /// Continue in Background: the progress window no longer shows it.
    pub hidden: bool,
    /// [`export_progress::now`] when it started: the progress window's age.
    started: f64,
    done: Receiver<Done>,
}

impl Running {
    /// Does the progress window show it: neither hidden (Continue in Background) nor cancelled?
    fn shown(&self) -> bool {
        !self.hidden && !self.progress.cancelled()
    }
}

/// The heavy filters running.
#[derive(Default)]
pub struct PluginRuns {
    pub running: Vec<Running>,
}

/// Do heavy filters run off the UI thread here? Not on wasm (no threads).
pub fn available() -> bool {
    !cfg!(target_arch = "wasm32")
}

/// Runs filter `p` (`plugin.run`'s params) on a thread of its own → `{background: true, status}`;
/// where no thread can run it, at once (`plugin.run`'s result).
pub fn start(app: &mut VectorcraftApp, p: Value) -> Result<Value, String> {
    if !available() {
        return app.run("plugin.run", p);
    }
    // Bad parameters and a missing document fail here, before anything runs.
    let run = FilterRun::prepare(&app.session, &p).map_err(|e| e.to_string())?;
    let who = format!("“{}”", run.plugin().manifest().name.trim_end_matches('…'));
    let mut params = p;
    params["ids"] = json!(run.inputs().iter().map(|i| i.0).collect::<Vec<_>>());
    let (tx, done) = channel();
    let shown = Progress::new();
    let reporting = Arc::clone(&shown);
    let job = move || {
        let output = vectorcraft_engine::guard::catch_panic(|| progress::scope(&reporting, || run.run()))
            .map_err(|msg| format!("internal error: {msg} (please report this bug)"))
            .and_then(|r| r.map_err(|e| e.to_string()));
        // The app may have quit meanwhile: nothing to hand it to.
        let _ = tx.send((run, output));
    };
    app.host.jobs().spawn("vectorcraft-plugin", Box::new(job));
    let label = format!("Running {who}…");
    let started = export_progress::now(app);
    let runs = &mut app.plugin_runs;
    runs.running.push(Running { label: label.clone(), who, params, progress: shown, hidden: false, started, done });
    app.status(label.clone());
    Ok(json!({ "background": true, "status": label }))
}

/// Applies the runs that are done (each frame). Not while a dialog is open: its OK acts on the
/// active document, which a run's result changes; the results wait until it is closed.
pub fn poll(app: &mut VectorcraftApp) {
    if app.any_dialog_open() {
        return;
    }
    let mut i = 0;
    while let Some(r) = app.plugin_runs.running.get(i) {
        let done = match r.done.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => {
                i += 1;
                continue;
            }
            Err(TryRecvError::Disconnected) => None,
        };
        let r = app.plugin_runs.running.remove(i);
        // The status bar no longer says it is running.
        if app.ui.status == r.label {
            app.ui.status.clear();
        }
        finish(app, r, done);
    }
}

/// Applies run `r`'s result: its output, as `plugin.run`, while the document and the plug-in are as
/// they were; else why it has none, in the status bar.
fn finish(app: &mut VectorcraftApp, r: Running, done: Option<Done>) {
    // Cancel changes nothing, even when the run ended just before it.
    if r.progress.cancelled() {
        app.status(EngineError::Cancelled.to_string());
        return;
    }
    let (run, output) = match done {
        Some((run, Ok(output))) => (run, output),
        Some((_, Err(e))) => {
            app.status(format!("{} failed: {e}", r.who));
            return;
        }
        None => {
            app.status(format!("{} failed: {e}", r.who, e = "internal error: its thread stopped (please report this bug)"));
            return;
        }
    };
    if !run.current(&app.session) {
        app.status(format!("{} finished, but the document or the plug-in changed meanwhile: run it again", r.who));
        return;
    }
    run.hand_in(&mut app.session, output);
    if let Err(e) = app.run("plugin.run", r.params) {
        app.status(format!("{} failed: {e}", r.who));
    }
}

/// Shows in the status bar the notice of the plug-in run that command `cmd` (`plugin.run` or a
/// plug-in command) applied, with result `r`: `Plug-in “name”: notice`, the name of the plug-in that
/// ran (`pluginName`) escaped and capped as prompts show it. The notice is shown as the run returned
/// it (escaped and capped by the host) and never translated. A result without a notice leaves the
/// status bar as it is. For a `command.batch`, such as an action's steps, the last plug-in run among
/// its steps with a notice shows it.
pub fn show_notice(app: &mut VectorcraftApp, cmd: &str, r: &Value) {
    let r = if cmd == "command.batch" {
        match last_batch_notice(r) {
            Some(step) => step,
            None => return,
        }
    } else if cmd == "plugin.run" || vectorcraft_engine::cmd::plugin::is_command_id(cmd) {
        r
    } else {
        return;
    };
    let (Some(notice), Some(id)) = (r.get("notice").and_then(Value::as_str), r.get("plugin").and_then(Value::as_str)) else { return };
    let name = r
        .get("pluginName")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| registry::get(id).map(|p| p.manifest().name.clone()))
        .map_or_else(|| id.to_string(), |name| name.trim().trim_end_matches('…').to_string());
    let name = permissions::display_text(&name, permissions::MAX_SHOWN_NAME);
    let status = format!("Plug-in “{name}”: {notice}");
    let before = status.strip_suffix(notice).unwrap_or_default().to_string();
    app.status(status);
    app.ui.status_kept = Some((before, notice.to_string(), String::new()));
}

/// The result of the last step of `command.batch` result `r` that a plug-in run with a notice gave
/// (`{plugin, ids, notice}`).
fn last_batch_notice(r: &Value) -> Option<&Value> {
    r.get("results")?.as_array()?.iter().rev().find(|step| {
        step.get("notice").is_some_and(Value::is_string) && step.get("plugin").is_some_and(Value::is_string) && step.get("ids").is_some()
    })
}

/// The progress window's Cancel (also Esc): every filter running in the background stops (at once when it checks
/// `vc_cancelled`, else at the next slice of its budget) and changes nothing → `{cancelled: n}`.
pub fn cancel_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.plugin_runs.running.iter().filter(|r| !r.progress.cancelled()) {
        r.progress.cancel();
        n += 1;
    }
    json!({ "cancelled": n })
}

/// The progress window's Continue in Background: the runs go on, shown in the status bar only →
/// `{hidden: n}`.
pub fn hide_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.plugin_runs.running.iter_mut().filter(|r| r.shown()) {
        r.hidden = true;
        n += 1;
    }
    json!({ "hidden": n })
}

/// Is a heavy filter running that hasn't been cancelled?
pub fn any_running(app: &VectorcraftApp) -> bool {
    app.plugin_runs.running.iter().any(|r| !r.progress.cancelled())
}

/// Is a heavy filter running that the progress window shows?
pub fn any_shown(app: &VectorcraftApp) -> bool {
    app.plugin_runs.running.iter().any(Running::shown)
}

/// The progress window of the first run it shows. `export_progress::show` orders the job tables and
/// decides when a window may appear.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<export_progress::Shown> {
    let first = app.plugin_runs.running.iter().find(|r| r.shown())?;
    Some(export_progress::Shown {
        title: crate::i18n::msg(&first.label).to_string(),
        step: tl!("The plug-in is computing…").to_string(),
        fraction: first.progress.fraction().map(|f| f as f32),
        counts: None,
        queued: None,
        started: first.started,
        cancel: "plugin.cancelRuns",
        hide: Some("plugin.hideRuns"),
    })
}
