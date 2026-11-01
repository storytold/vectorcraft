//! Heavy plug-in filters, run off the UI thread. A filter whose manifest asks for a heavy budget
//! (`Budget::Heavy`: minutes rather than a minute, for routing, packing or simulation) would freeze
//! the app while it computes, so the app runs it on a thread of its own ([`start`]) while editing
//! goes on: the status bar shows it, and after [`DELAY`] a progress window with Cancel (also Esc,
//! `plugin.cancelRuns`) and Continue in Background (`plugin.hideRuns`). A thread of its own, not
//! the background worker's queue ([`crate::background`]), so saves and exports don't wait behind it.
//!
//! When it is done its output is handed to `plugin.run` with the same parameters and objects
//! ([`FilterRun::hand_in`]), so the edit is one undo step like any run, unless the document or the
//! plug-in changed meanwhile: then nothing changes and the status bar says to run it again. On the
//! web (no threads) it runs at once ([`available`]).

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_doc::cancel::{self, CancelToken};
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::plugin::FilterRun;

use crate::VectorcraftApp;
use crate::export_progress::{self, Pressed};

pub use crate::export_progress::DELAY;

/// What a run's thread hands back: the run, and its output or why it has none.
type Done = (FilterRun, Result<Vec<u8>, String>);

/// A heavy filter running.
pub struct Running {
    id: u64,
    /// What the status bar and the progress window say: `Running “Route Paths”…`.
    pub label: String,
    /// The filter's name, quoted.
    who: String,
    /// `plugin.run`'s params that apply its output: the objects it was given as `ids`.
    params: Value,
    /// Its Cancel.
    token: CancelToken,
    /// Continue in Background: the progress window no longer shows it.
    pub hidden: bool,
    done: Receiver<Done>,
}

impl Running {
    /// Does the progress window show it: neither hidden (Continue in Background) nor cancelled?
    fn shown(&self) -> bool {
        !self.hidden && !self.token.is_cancelled()
    }
}

/// The heavy filters running.
#[derive(Default)]
pub struct PluginRuns {
    pub running: Vec<Running>,
    next_id: u64,
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
    let token = CancelToken::new();
    let stop = token.clone();
    std::thread::Builder::new()
        .name("vectorcraft-plugin".into())
        .spawn(move || {
            let output = vectorcraft_engine::guard::catch_panic(|| cancel::scope(&stop, None, || run.run()))
                .map_err(|msg| format!("internal error: {msg} (please report this bug)"))
                .and_then(|r| r.map_err(|e| e.to_string()));
            // The app may have quit meanwhile: nothing to hand it to.
            let _ = tx.send((run, output));
        })
        .map_err(|e| e.to_string())?;
    let label = format!("Running {who}…");
    let runs = &mut app.plugin_runs;
    runs.next_id += 1;
    runs.running.push(Running { id: runs.next_id, label: label.clone(), who, params, token, hidden: false, done });
    app.status(label.clone());
    Ok(json!({ "background": true, "status": label }))
}

/// Applies the runs that are done (each frame). Not while a dialog is open: its OK acts on the
/// active document, which a run's result changes; the results wait until it is closed.
pub fn poll(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_some() {
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
    if r.token.is_cancelled() {
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

/// The progress window's Cancel (also Esc): every heavy filter running stops at the next slice of
/// its budget and changes nothing → `{cancelled: n}`.
pub fn cancel_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.plugin_runs.running.iter().filter(|r| !r.token.is_cancelled()) {
        r.token.cancel();
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
    app.plugin_runs.running.iter().any(|r| !r.token.is_cancelled())
}

/// Is a heavy filter running that the progress window shows?
pub fn any_shown(app: &VectorcraftApp) -> bool {
    app.plugin_runs.running.iter().any(Running::shown)
}

/// The progress window of the first run it shows, once that has run for [`DELAY`]. Not while a
/// dialog or the export progress window is open: never two modals at once.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(first) = app.plugin_runs.running.iter().find(|r| r.shown()) else { return };
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|m| {
        let seen = m.get_temp_mut_or_insert_with(egui::Id::unique("plugin-run-seen"), || (first.id, now));
        if seen.0 != first.id {
            *seen = (first.id, now);
        }
        seen.1
    });
    let exporting = app.background.jobs.iter().any(|j| j.progress.is_some() && !j.hidden && !j.cancelled());
    if now - since < DELAY || app.ui.dialog.is_some() || exporting {
        return;
    }
    let heading = crate::i18n::msg(&first.label);
    let pressed = export_progress::window(ctx, "plugin-run-progress", &heading, |ui| export_progress::busy(ui, tl!("The plug-in is computing…")));
    let id = match pressed {
        Some(Pressed::Cancel) => "plugin.cancelRuns",
        Some(Pressed::Hide) => "plugin.hideRuns",
        None => return,
    };
    if let Err(e) = app.run(id, json!({})) {
        app.status(e);
    }
}
