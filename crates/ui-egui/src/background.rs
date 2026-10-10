//! Background Save and Background Export (Preferences → File Handling): a save or an export encodes
//! and writes a snapshot of the document on a worker thread while editing goes on. The status bar
//! shows what is in progress; the result is applied on the UI thread when it arrives (a save marks
//! the document saved as of its snapshot, so edits made meanwhile keep it modified).
//!
//! It needs a thread-safe writer from the host ([`crate::Services::write_shared`], desktop);
//! without one (the web, which downloads) the work runs at once, as with the preference off.

use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::{Value, json};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// Writes a file; callable from any thread.
pub type SharedWriteFn = std::sync::Arc<dyn Fn(&str, &[u8]) -> Result<(), String> + Send + Sync>;

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

/// A job in progress.
pub struct Job {
    id: u64,
    /// What the status bar shows ("Saving Poster.vectorcraft").
    pub label: String,
    /// The document the job reads (`DocState::uid`), if any.
    pub doc: Option<u64>,
    then: Then,
}

/// The worker and the jobs it has not finished.
#[derive(Default)]
pub struct Background {
    worker: Option<(Sender<Task>, Receiver<Done>)>,
    /// Jobs in the order they were started (the worker runs them in that order).
    pub jobs: Vec<Job>,
    next_id: u64,
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
}

/// Run `work` on a worker thread when `enabled` and the host can write from one, else now; `then`
/// gets its result on the UI thread. → `then`'s result, or `{background: true, status}` while the
/// work runs (the status bar shows `label`).
pub fn run(
    app: &mut VectorcraftApp,
    enabled: bool,
    label: String,
    doc: Option<u64>,
    work: impl FnOnce(Writer) -> Result<Value, String> + Send + 'static,
    then: impl FnOnce(&mut VectorcraftApp, Result<Value, String>) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    let mut work: Work = Box::new(work);
    let shared = app.services.write_shared.clone().filter(|_| enabled && app.services.download.is_none());
    if let Some(write) = shared {
        let id = app.background.next_id;
        if let Some(tx) = app.background.worker() {
            match tx.send(Task { id, work, write, scope: ScopeHandle::capture() }) {
                Ok(()) => {
                    let bg = &mut app.background;
                    bg.next_id += 1;
                    bg.jobs.push(Job { id, label: label.clone(), doc, then: Box::new(then) });
                    let status = format!("{label}…");
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
    let services = &mut app.services;
    let r = work(&mut |p: &str, b: &[u8]| crate::io::write_to(services, p, b));
    then(app, r)
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
    while !app.background.jobs.is_empty() {
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

/// Hand a job its result; a failure goes to the status bar (the call that started it returned).
fn apply(app: &mut VectorcraftApp, job: Job, r: Result<Value, String>) {
    if let Err(e) = (job.then)(app, r) {
        app.status(format!("{} failed: {e}", job.label));
    }
}
