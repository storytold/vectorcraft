//! Background Save and Background Export (Preferences → File Handling): a save or an export encodes
//! and writes a snapshot of the document on a worker thread while editing goes on. The status bar
//! shows what is in progress; the result is applied on the UI thread when it arrives (a save marks
//! the document saved as of its snapshot, so edits made meanwhile keep it modified). Exports and
//! saves show in the progress window ([`crate::export_progress`]) with Cancel and Continue in
//! Background.
//!
//! It needs a thread-safe writer from the host ([`crate::Services::write_shared`], desktop);
//! without one (the web, which downloads) the work runs at once, as with the preference off. Work
//! run at once gives a heavy exporter an export's usual budget ([`run_here`]).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::progress::Progress;
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// Writes a file; callable from any thread.
pub type SharedWriteFn = std::sync::Arc<dyn Fn(&str, &[u8]) -> Result<(), String> + Send + Sync>;

/// Writes the file at a path with what the filler it is given writes into it (atomically, as
/// `vectorcraft_format::write_atomic_with`); callable from any thread.
pub type SharedStreamFn =
    std::sync::Arc<dyn Fn(&str, &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>) -> Result<(), String> + Send + Sync>;

/// A writer for the work: the host's file writer (or browser download) on the UI thread, the
/// shared one on the worker.
pub type Writer<'a> = &'a mut dyn FnMut(&str, &[u8]) -> Result<(), String>;

/// Encodes and writes a snapshot (no access to the app) → the result handed to [`Then`].
type Work = Box<dyn FnOnce(Writer) -> Result<Value, String> + Send>;

/// Applies a result on the UI thread (document state, status, recent files) → the command result.
type Then = Box<dyn FnOnce(&mut VectorcraftApp, Result<Value, String>) -> Result<Value, String>>;

/// A job's id and the work's result, back from the worker.
type Done = (u64, Result<Value, String>);

struct Task {
    id: u64,
    work: Work,
    write: SharedWriteFn,
    /// The automation roots in force where the work was started, in force where it runs.
    scope: ScopeHandle,
}

/// What a job that shows in the progress window does: its Cancel and Continue in Background act on
/// the jobs of its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An export (Background Export): `file.cancelExport`, `file.exportInBackground`.
    Export,
    /// A save (Background Save): `file.cancelSave`, `file.saveInBackground`.
    Save,
}

/// A job in progress.
pub struct Job {
    id: u64,
    /// What the job does, for the progress window and its commands.
    pub kind: Kind,
    /// What the status bar and `ui.inspect` show ("Saving Poster.vectorcraft"); empty for work they
    /// don't show.
    pub label: String,
    /// The document the job reads (`DocState::uid`), if any.
    pub doc: Option<u64>,
    /// How far an export or a save has got, and its Cancel ([`crate::export_progress`] shows it).
    pub progress: Option<Arc<Progress>>,
    /// Continue in Background: the progress dialog no longer shows this job.
    pub hidden: bool,
    /// [`crate::state::UiState::status_sets`] before the job set its status line.
    sets: u64,
    /// The status line the job set ("<label>…"); empty when it set none.
    line: String,
    then: Then,
}

impl Job {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Was the job cancelled (it writes nothing more; its result is still on its way)?
    pub fn cancelled(&self) -> bool {
        self.progress.as_ref().is_some_and(|p| p.cancelled())
    }
}

/// The worker and the jobs it has not finished.
#[derive(Default)]
pub struct Background {
    worker: Option<(Sender<Task>, Receiver<Done>)>,
    /// Jobs in the order they were started (the worker runs them in that order).
    pub jobs: Vec<Job>,
    next_id: u64,
    /// Work started while this is set runs at once, whatever the preferences ([`in_foreground`]).
    foreground: bool,
    /// [`crate::state::UiState::status_sets`] right after Cancel Save set its line, or after a save
    /// it reached set the line that followed it; `None` before any Cancel Save.
    pub(crate) save_cancel_line: Option<u64>,
}

impl Background {
    /// The worker's channels, started on first use (`None` on wasm, which has no threads).
    fn worker(&mut self) -> Option<&Sender<Task>> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        if self.worker.is_none() {
            let (tx, tasks) = channel::<Task>();
            let (done_tx, done) = channel();
            std::thread::Builder::new()
                .name("vectorcraft-save".into())
                .spawn(move || {
                    while let Ok(Task { id, work, write, scope }) = tasks.recv() {
                        let r = scope
                            .run(|| vectorcraft_engine::guard::catch_panic(|| work(&mut |p: &str, b: &[u8]| write(p, b))))
                            .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)")));
                        if done_tx.send((id, r)).is_err() {
                            break;
                        }
                    }
                })
                .ok()?;
            self.worker = Some((tx, done));
        }
        self.worker.as_ref().map(|(tx, _)| tx)
    }

    /// Is a job reading document `uid` still running?
    pub fn busy_with(&self, uid: u64) -> bool {
        self.jobs.iter().any(|j| j.doc == Some(uid))
    }

    /// Cancel every export in progress → how many there were.
    pub fn cancel_exports(&mut self) -> usize {
        self.cancel(Kind::Export)
    }

    /// Hide the exports in progress from the progress dialog (they go on) → how many there were.
    pub fn hide_exports(&mut self) -> usize {
        self.hide(Kind::Export)
    }

    /// Cancel every save in progress that the progress window can show → how many there were.
    pub fn cancel_saves(&mut self) -> usize {
        self.cancel(Kind::Save)
    }

    /// Hide the saves in progress from the progress window (they go on) → how many there were.
    pub fn hide_saves(&mut self) -> usize {
        self.hide(Kind::Save)
    }

    /// Is a job of `kind` in progress that can still be canceled?
    pub fn running(&self, kind: Kind) -> bool {
        self.jobs.iter().any(|j| j.kind == kind && j.progress.is_some() && !j.cancelled())
    }

    fn cancel(&mut self, kind: Kind) -> usize {
        let mut n = 0;
        for p in self.jobs.iter().filter(|j| j.kind == kind && !j.cancelled()).filter_map(|j| j.progress.as_ref()) {
            p.cancel();
            n += 1;
        }
        n
    }

    fn hide(&mut self, kind: Kind) -> usize {
        let mut n = 0;
        for j in self.jobs.iter_mut().filter(|j| j.kind == kind && j.progress.is_some() && !j.hidden && !j.cancelled()) {
            j.hidden = true;
            n += 1;
        }
        n
    }
}

/// Run `work` on a worker thread when `enabled` and the host can write from one, else now; `then`
/// gets its result on the UI thread. → `then`'s result, or `{background: true, status}` while the
/// work runs (the status bar shows `label`). With an empty `label` neither the status bar nor
/// `ui.inspect` shows the work, and the reply has no `status`.
pub fn run(
    app: &mut VectorcraftApp,
    enabled: bool,
    label: String,
    doc: Option<u64>,
    work: impl FnOnce(Writer) -> Result<Value, String> + Send + 'static,
    then: impl FnOnce(&mut VectorcraftApp, Result<Value, String>) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    run_with_progress(app, enabled, label, doc, None, work, then)
}

/// [`run`] for work that reports to `progress` (an export): the progress dialog shows it while it
/// runs in the background, with Cancel.
pub fn run_with_progress(
    app: &mut VectorcraftApp,
    enabled: bool,
    label: String,
    doc: Option<u64>,
    progress: Option<Arc<Progress>>,
    work: impl FnOnce(Writer) -> Result<Value, String> + Send + 'static,
    then: impl FnOnce(&mut VectorcraftApp, Result<Value, String>) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    start(app, enabled, label, doc, progress, Kind::Export, work, then)
}

/// [`run_with_progress`] for a save of document `doc`: the progress window shows it as a save, and
/// its Cancel is `file.cancelSave`.
pub fn run_saving(
    app: &mut VectorcraftApp,
    enabled: bool,
    label: String,
    doc: u64,
    progress: Arc<Progress>,
    work: impl FnOnce(Writer) -> Result<Value, String> + Send + 'static,
    then: impl FnOnce(&mut VectorcraftApp, Result<Value, String>) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    start(app, enabled, label, Some(doc), Some(progress), Kind::Save, work, then)
}

#[allow(clippy::too_many_arguments)]
fn start(
    app: &mut VectorcraftApp,
    enabled: bool,
    label: String,
    doc: Option<u64>,
    progress: Option<Arc<Progress>>,
    kind: Kind,
    work: impl FnOnce(Writer) -> Result<Value, String> + Send + 'static,
    then: impl FnOnce(&mut VectorcraftApp, Result<Value, String>) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    let mut work: Work = Box::new(work);
    let shared = app.services.write_shared.clone().filter(|_| enabled && !app.background.foreground && app.services.download.is_none());
    if let Some(write) = shared {
        let id = app.background.next_id;
        if let Some(tx) = app.background.worker() {
            match tx.send(Task { id, work, write, scope: ScopeHandle::capture() }) {
                Ok(()) => {
                    let (sets, status) = (app.ui.status_sets, if label.is_empty() { String::new() } else { format!("{label}…") });
                    let bg = &mut app.background;
                    bg.next_id += 1;
                    let line = status.clone();
                    bg.jobs.push(Job { id, kind, label: label.clone(), doc, progress, hidden: false, sets, line, then: Box::new(then) });
                    if label.is_empty() {
                        return Ok(json!({ "background": true }));
                    }
                    app.status(status.clone());
                    return Ok(json!({ "background": true, "status": status }));
                }
                // The worker stopped: start a new one next time, and do this one now.
                Err(e) => {
                    app.background.worker = None;
                    work = e.0.work;
                }
            }
        }
    }
    let r = run_here(app, |app| {
        let services = &mut app.services;
        work(&mut |p: &str, b: &[u8]| crate::io::write_to(services, p, b))
    });
    then(app, r)
}

/// Runs `f`, an export done at once on the UI thread, where nothing can cancel it: a heavy exporter
/// gets an export's usual budget in it ([`vectorcraft_plugins::export::at_once`]), so the window is
/// never held for minutes. A control client that asked to wait for the file ([`in_foreground`])
/// gets the heavy budget, as `document.export` does where it is called.
pub(crate) fn run_here<T>(app: &mut VectorcraftApp, f: impl FnOnce(&mut VectorcraftApp) -> T) -> T {
    if app.background.foreground { f(app) } else { vectorcraft_plugins::export::at_once(|| f(app)) }
}

/// Run `f` on the UI thread once job `id` has finished, right after the job's own result is
/// applied, with that applied result (an error when the work failed). Nothing runs when no job
/// `id` is in progress.
pub fn after(app: &mut VectorcraftApp, id: u64, f: impl FnOnce(&mut VectorcraftApp, &Result<Value, String>) + 'static) {
    let Some(job) = app.background.jobs.iter_mut().find(|j| j.id == id) else { return };
    let then = std::mem::replace(&mut job.then, Box::new(|_, r| r));
    job.then = Box::new(move |app, r| {
        let r = then(app, r);
        f(app, &r);
        r
    });
}

/// Run `f` with every save and export it starts written at once, as with the preferences off, so
/// its result is the written file or the error (a control client's `wait: true`). The jobs already
/// running finish first, so none of them writes over its files afterwards.
pub fn in_foreground<T>(app: &mut VectorcraftApp, f: impl FnOnce(&mut VectorcraftApp) -> T) -> T {
    wait_all(app);
    let was = std::mem::replace(&mut app.background.foreground, true);
    // A panic caught further up (a control request's guard) still turns the setting back.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(app)));
    app.background.foreground = was;
    r.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/// Apply the results that arrived (each frame).
pub fn poll(app: &mut VectorcraftApp) {
    while let Some(done) = app.background.worker.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
        finish(app, done);
    }
}

/// Wait for every job and apply its result (before quitting, closing a document, or another save
/// of a document a job is reading).
pub fn wait_all(app: &mut VectorcraftApp) {
    wait_until(app, |_| false);
}

/// Apply the jobs' results as they arrive (in the order the jobs were started) until `ready` holds
/// or no job is left.
pub fn wait_until(app: &mut VectorcraftApp, ready: impl Fn(&VectorcraftApp) -> bool) {
    while !ready(app) && !app.background.jobs.is_empty() {
        let Some(done) = app.background.worker.as_ref().and_then(|(_, rx)| rx.recv().ok()) else {
            // The worker is gone: its jobs can't finish.
            for job in std::mem::take(&mut app.background.jobs) {
                apply(app, job, Err("the background worker stopped".into()));
            }
            app.background.worker = None;
            return;
        };
        finish(app, done);
    }
}

fn finish(app: &mut VectorcraftApp, (id, r): Done) {
    let Some(i) = app.background.jobs.iter().position(|j| j.id == id) else { return };
    let job = app.background.jobs.remove(i);
    apply(app, job, r);
}

/// Does the status bar still show what a job left there? No line has been set since `sets`
/// ([`crate::state::UiState::status_sets`] when the job started), or the one line set since is
/// the job's own `line` ("<label>…"; empty for a job that set none) and it is still shown. A job's
/// result goes to the status bar only then: a line set since (by another command or another job)
/// stays, also when it has the same text as an earlier one.
pub(crate) fn status_unchanged(app: &VectorcraftApp, sets: u64, line: &str) -> bool {
    let now = app.ui.status_sets;
    now == sets || (!line.is_empty() && now == sets.wrapping_add(1) && app.ui.status == line)
}

/// Hand a job its result; a failure goes to the status bar (the call that started it returned)
/// while the status bar still shows the job's own line ([`status_unchanged`]) and the job's
/// callback set no line of its own.
fn apply(app: &mut VectorcraftApp, job: Job, r: Result<Value, String>) {
    let Job { label, sets, line, then, .. } = job;
    let shown = status_unchanged(app, sets, &line);
    let before = app.ui.status_sets;
    if let Err(e) = then(app, r)
        && shown
        && app.ui.status_sets == before
    {
        app.status(format!("{label} failed: {e}"));
    }
}
