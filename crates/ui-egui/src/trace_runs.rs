//! Image Trace off the UI thread (#3098). A photo or a large image takes seconds to trace, and the
//! window froze while it did. A trace the user asks for (Object › Image Trace › Make and Make and
//! Expand, the task bar's Image Trace, the Image Trace panel's Trace and presets) is prepared at
//! once, so a bad setting fails before anything runs, and traced on a thread of its own ([`start`])
//! while editing goes on: the status bar shows it, and after the progress window's delay it shows how
//! many of its colours are traced, with Cancel (also Esc, `imageTrace.cancelRuns`) and Continue in
//! Background (`imageTrace.hideRuns`). A new trace of the same object cancels the one before it.
//!
//! When it is done, what it traced is handed to its command with the same parameters
//! ([`Traced::hand_in`]), so the edit is one undo step like any trace, unless the image or the
//! settings changed meanwhile or File › Revert replaced the document's content: then nothing
//! changes and the status bar says to trace it again. The
//! control channel, MCP and scripts run the commands at once, and so does every trace on the web,
//! in unit tests that don't set `off_the_ui_thread`, and with `VECTORCRAFT_SYNC_OPEN` set
//! ([`available`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_doc::cancel::CancelToken;
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::{TraceRun, Traced};

use crate::{VectorcraftApp, export_progress};

/// What the status bar and the progress window say.
const LABEL: &str = "Tracing the image";

/// What a trace's thread hands back: the run, and what it traced or why it has nothing.
type Done = (TraceRun, Result<Traced, String>);

/// A trace running.
pub struct Running {
    /// [`crate::export_progress::now`] when it started: the progress window's delay counts from it.
    pub(crate) started: f64,
    /// The document it traces in ([`vectorcraft_engine::DocState::uid`]) and the object.
    doc: u64,
    target: NodeId,
    /// Its Cancel.
    token: CancelToken,
    /// A newer trace of the same object took its place: it ends without a word.
    superseded: bool,
    /// How far the trace has got and in all, as the trace counts it (color layers, or finer steps for Overlapping
    /// with Fills; 0 in all while the image is prepared).
    steps: Arc<[AtomicU64; 2]>,
    /// The Image Trace panel started it, and takes its result ([`crate::panels::image_trace::traced`]).
    from_panel: bool,
    /// Continue in Background: the progress window no longer shows it.
    pub hidden: bool,
    done: Receiver<Done>,
}

impl Running {
    /// Does the progress window show it: neither hidden (Continue in Background) nor cancelled?
    fn shown(&self) -> bool {
        !self.hidden && !self.token.is_cancelled()
    }

    /// The share of the trace done, once its steps are known.
    fn fraction(&self) -> Option<f32> {
        let [done, total] = &*self.steps;
        let (done, total) = (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
        (total > 0).then(|| done.min(total) as f32 / total as f32)
    }
}

/// The traces running.
#[derive(Default)]
pub struct TraceRuns {
    pub running: Vec<Running>,
    /// Set by unit tests whose app traces off the UI thread.
    pub(crate) off_the_ui_thread: bool,
    /// The status bar's message the latest Image Trace with notes left, with
    /// [`crate::state::UiState::status_sets`] then.
    pub(crate) notes_status: Option<(String, u64)>,
}

/// The status bar's line after an Image Trace with `n` notes, the first of them `first`.
pub(crate) fn traced_with_notes(n: usize, first: &str) -> String {
    crate::i18n::msg_n(n as u64, "Traced with {n} note: {first}", "Traced with {n} notes: {first}", &[("first", first)])
}

/// Does `app` trace off the UI thread? Where documents open off it ([`crate::opening::available`]),
/// and in the unit tests that set `off_the_ui_thread`.
pub fn available(app: &VectorcraftApp) -> bool {
    app.trace_runs.off_the_ui_thread || crate::opening::available()
}

/// Traces with `command` (`imageTrace.make` or `imageTrace.makeAndExpand`) and its params `p` on a
/// thread of its own → `{background: true, status}`; `from_panel` when the Image Trace panel asks.
/// Where no thread can run it, at once (the command's result).
pub fn start(app: &mut VectorcraftApp, command: &str, p: Value, from_panel: bool) -> Result<Value, String> {
    if !available(app) {
        return app.run(command, p);
    }
    // Bad settings and nothing to trace fail here, before anything runs.
    let run = TraceRun::prepare(&app.session, command, &p).map_err(|e| e.to_string())?;
    let doc = app.session.doc().map_err(|e| e.to_string())?.uid;
    let target = run.target();
    for r in app.trace_runs.running.iter_mut().filter(|r| r.doc == doc && r.target == target) {
        r.token.cancel();
        r.superseded = true;
    }
    let (tx, done) = channel();
    let token = CancelToken::new();
    let stop = token.clone();
    let steps = Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
    let counts = Arc::clone(&steps);
    std::thread::Builder::new()
        .name("vectorcraft-trace".into())
        .spawn(move || {
            let [done_now, total_now] = &*counts;
            let report = |done: u64, total: u64| {
                done_now.store(done, Ordering::Relaxed);
                total_now.store(total, Ordering::Relaxed);
                !stop.is_cancelled()
            };
            let traced = vectorcraft_engine::guard::catch_panic(|| run.run(&report))
                .map_err(|msg| format!("internal error: {msg} (please report this bug)"))
                .and_then(|r| r.map_err(|e| e.to_string()));
            // The app may have quit meanwhile: nothing to hand it to.
            let _ = tx.send((run, traced));
        })
        .map_err(|e| e.to_string())?;
    let started = crate::export_progress::now();
    app.trace_runs.running.push(Running { started, doc, target, token, superseded: false, steps, from_panel, hidden: false, done });
    // This frame may have asked for its next one before the trace started (`tick`): ask now.
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    let status = format!("{}…", crate::i18n::msg(LABEL));
    app.status(status.clone());
    Ok(json!({ "background": true, "status": status }))
}

/// Applies the traces that are done (each frame), in the order they were started. Not while a
/// dialog is open (its OK acts on the active document, which a trace's result changes), nor while
/// an interaction is open or a tool drags with the pointer button held, as [`crate::placing::poll`].
pub fn poll(app: &mut VectorcraftApp) {
    if app.any_dialog_open() || app.session.in_interaction() || (app.session.tool_busy() && app.session.pointer_held()) {
        return;
    }
    let mut i = 0;
    while let Some(r) = app.trace_runs.running.get(i) {
        let done = match r.done.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => {
                i += 1;
                continue;
            }
            Err(TryRecvError::Disconnected) => None,
        };
        let r = app.trace_runs.running.remove(i);
        // The status bar no longer says it is tracing.
        if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
            app.ui.status.clear();
        }
        finish(app, r, done);
    }
}

/// Applies trace `r`'s result in its document, which becomes the active one again, as its command
/// would while the object, its image and the settings are as they were and the document was not
/// reverted; else why it has none, in the status bar.
fn finish(app: &mut VectorcraftApp, r: Running, done: Option<Done>) {
    // Cancel changes nothing, even when the trace ended just before it.
    if r.token.is_cancelled() {
        if !r.superseded {
            app.status(EngineError::Cancelled.to_string());
        }
        return;
    }
    let (run, traced) = match done {
        Some((run, Ok(traced))) => (run, traced),
        Some((_, Err(e))) => {
            app.status(format!("Image Trace failed: {e}"));
            return;
        }
        None => {
            app.status("Image Trace failed: internal error: its thread stopped (please report this bug)");
            return;
        }
    };
    let Some(i) = app.session.documents().iter().position(|st| st.uid == r.doc) else {
        app.status("Image Trace finished, but its document was closed");
        return;
    };
    app.session.set_active(i);
    if run.reverted(&app.session) {
        app.status("Image Trace finished, but its document was reverted: trace it again");
        return;
    }
    if !run.current(&app.session) {
        app.status("Image Trace finished, but the image or its settings changed meanwhile: trace it again");
        return;
    }
    traced.hand_in(&mut app.session);
    match app.run(run.command(), run.params().clone()) {
        Ok(v) if r.from_panel => crate::panels::image_trace::traced(app, &v),
        Ok(_) => {}
        Err(e) => app.status(format!("Image Trace failed: {e}")),
    }
}

/// The progress window's Cancel (also Esc): every trace running stops at its next colour and
/// changes nothing → `{cancelled: n}`.
pub fn cancel_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.trace_runs.running.iter().filter(|r| !r.token.is_cancelled()) {
        r.token.cancel();
        n += 1;
    }
    json!({ "cancelled": n })
}

/// The progress window's Continue in Background: the traces go on, shown in the status bar only
/// → `{hidden: n}`.
pub fn hide_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.trace_runs.running.iter_mut().filter(|r| r.shown()) {
        r.hidden = true;
        n += 1;
    }
    json!({ "hidden": n })
}

/// Is a trace running that hasn't been cancelled?
pub fn any_running(app: &VectorcraftApp) -> bool {
    app.trace_runs.running.iter().any(|r| !r.token.is_cancelled())
}

/// Is a trace running that the progress window shows?
pub fn any_shown(app: &VectorcraftApp) -> bool {
    app.trace_runs.running.iter().any(Running::shown)
}

/// The progress window of the first trace it shows; `export_progress::show` decides when it
/// shows and runs the command of the button pressed.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<export_progress::Shown> {
    let first = app.trace_runs.running.iter().find(|r| r.shown())?;
    let fraction = first.fraction();
    let step = if fraction.is_some() { tl!("Tracing the colors…") } else { tl!("Preparing the image…") };
    Some(export_progress::Shown {
        title: format!("{}…", crate::i18n::msg(LABEL)),
        step: step.to_string(),
        fraction,
        counts: None,
        queued: None,
        started: first.started,
        cancel: "imageTrace.cancelRuns",
        hide: Some("imageTrace.hideRuns"),
    })
}
