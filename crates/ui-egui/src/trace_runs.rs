//! Image Trace off the UI thread (#3098). A photo or a large image takes seconds to trace, and the
//! window froze while it did. A trace the user asks for (Object › Image Trace › Make and Make and
//! Expand, the task bar's Image Trace, the Image Trace panel's Trace and presets) is prepared at
//! once, so a bad setting fails before anything runs, and traced on a thread of its own ([`start`])
//! while editing goes on: the status bar shows it, and after [`DELAY`] a progress window shows how
//! many of its colours are traced, with Cancel (also Esc, `imageTrace.cancelRuns`) and Continue in
//! Background (`imageTrace.hideRuns`). A new trace of the same object cancels the one before it.
//!
//! When it is done, what it traced is handed to its command with the same parameters
//! ([`Traced::hand_in`]), so the edit is one undo step like any trace, unless the image or the
//! settings changed meanwhile: then nothing changes and the status bar says to trace it again. The
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

use crate::VectorcraftApp;
use crate::export_progress::{self, Pressed};

pub use crate::export_progress::DELAY;

/// What the status bar and the progress window say.
const LABEL: &str = "Tracing the image";

/// What a trace's thread hands back: the run, and what it traced or why it has nothing.
type Done = (TraceRun, Result<Traced, String>);

/// A trace running.
pub struct Running {
    id: u64,
    /// The document it traces in ([`vectorcraft_engine::DocState::uid`]) and the object.
    doc: u64,
    target: NodeId,
    /// Its Cancel.
    token: CancelToken,
    /// A newer trace of the same object took its place: it ends without a word.
    superseded: bool,
    /// Colour layers traced and in all (0 while the image is prepared).
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

    /// The share of its colour layers traced, once they are known.
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
    next_id: u64,
    /// Set by unit tests whose app traces off the UI thread.
    pub(crate) off_the_ui_thread: bool,
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
    let runs = &mut app.trace_runs;
    runs.next_id += 1;
    runs.running.push(Running { id: runs.next_id, doc, target, token, superseded: false, steps, from_panel, hidden: false, done });
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
    if app.ui.dialog.is_some() || app.session.in_interaction() || (app.session.tool_busy() && app.session.pointer_held()) {
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
/// would while the object, its image and the settings are as they were; else why it has none, in
/// the status bar.
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

/// The progress window of the first trace it shows, once that has run for [`DELAY`]. Not while a
/// dialog, the export progress window or a plug-in's is open: never two modals at once.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(first) = app.trace_runs.running.iter().find(|r| r.shown()) else { return };
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|m| {
        let seen = m.get_temp_mut_or_insert_with(egui::Id::unique("trace-run-seen"), || (first.id, now));
        if seen.0 != first.id {
            *seen = (first.id, now);
        }
        seen.1
    });
    let exporting = app.background.jobs.iter().any(|j| j.progress.is_some() && !j.hidden && !j.cancelled());
    if now - since < DELAY || app.ui.dialog.is_some() || exporting || crate::plugin_runs::any_shown(app) {
        return;
    }
    let fraction = first.fraction();
    let t = crate::theme::Tokens::get(ctx);
    let heading = format!("{}…", crate::i18n::msg(LABEL));
    let pressed = export_progress::window(ctx, "trace-run-progress", &heading, |ui| match fraction {
        Some(frac) => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Tracing the colors…")).color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{:.0} %", frac * 100.0)).color(t.text_dim));
                });
            });
            ui.add_space(4.0);
            ui.add(egui::ProgressBar::new(frac).desired_width(360.0).desired_height(6.0).fill(t.accent));
        }
        None => export_progress::busy(ui, tl!("Preparing the image…")),
    });
    let id = match pressed {
        Some(Pressed::Cancel) => "imageTrace.cancelRuns",
        Some(Pressed::Hide) => "imageTrace.hideRuns",
        None => return,
    };
    if let Err(e) = app.run(id, json!({})) {
        app.status(e);
    }
}
