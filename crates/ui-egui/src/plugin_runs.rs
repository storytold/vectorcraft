//! Heavy plug-in filters, run off the UI thread. A filter whose manifest asks for a heavy budget
//! (`Budget::Heavy`: minutes rather than a minute, for routing, packing or simulation) would freeze
//! the app while it computes, so the app runs it on a thread of its own ([`start`]) while editing
//! goes on: the status bar shows it, and after the export progress window's delay a progress window with Cancel (also Esc,
//! `plugin.cancelRuns`) and Continue in Background (`plugin.hideRuns`). A thread of its own, not
//! the background worker's queue ([`crate::background`]), so saves and exports don't wait behind it.
//!
//! When it is done its output is handed to `plugin.run` with the same parameters and objects
//! ([`FilterRun::hand_in`]), so the edit is one undo step like any run, unless the document or the
//! plug-in changed meanwhile: then nothing changes and the status bar says to run it again.
//!
//! A filter of plug-in API level 2 may report how far it has got (`vc_progress`): the progress
//! window then shows a bar with the share done. Its Cancel also makes the filter's `vc_cancelled`
//! return 1, so a filter that checks it stops at once; one that doesn't stops at the next slice of
//! its budget. A light filter that may report its progress (the `progress` permission) and calls
//! no commands runs here too, under its light budget, so its progress and Cancel show while it
//! computes.
//!
//! The web build has no threads. There the app runs every filter it starts here in a Web Worker
//! of its own ([`Workers`], [`vectorcraft_plugins::worker`]), and it checks a module being installed
//! in one too ([`check_install`]), so neither runs plug-in code on the page. The page posts the module,
//! the input, the parameters and the limits, and reads the replies each frame ([`poll`]) from a
//! bounded inbox ([`Inbox`]). Cancel terminates the worker at once, and so does a run past its
//! time budget ([`GRACE_MS`] after the worker should have ended it) and a run whose document or
//! plug-in changes meanwhile. A filter that calls commands runs there too: the session holds its
//! undo step and refuses other edits while it runs ([`FilterRun::begin_calls`]), the page makes
//! each command call the worker sends and posts the answer, and the progress window shows at once,
//! with Cancel only. Without [`Workers`] a filter runs at once on the page, as before.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::fileio::progress::{self, Progress};
use vectorcraft_engine::cmd::plugin::{self, FilterRun};
use vectorcraft_plugins::worker::{FromWorker, PROTOCOL, ToWorker, unexpected};
use vectorcraft_plugins::{permissions, registry};

use crate::VectorcraftApp;
use crate::export_progress;

/// What a run's thread hands back: the run, and its output or why it has none.
type Done = (FilterRun, Result<Vec<u8>, String>);

/// How long past a run's time budget, or a check's, the page waits for the worker before it
/// terminates it, in milliseconds. The worker ends a run at its budget itself; this covers one
/// that doesn't answer.
pub const GRACE_MS: f64 = 5_000.0;

/// The most worker runs and install checks at once; one more waits for one of them to end.
pub const MAX_WORKERS: usize = 4;

/// The most messages besides progress a worker's [`Inbox`] holds between two frames. A worker sends
/// one or two (ready, then a command call or its end); more ends what it is doing.
pub const MAX_QUEUED: usize = 8;

/// What a worker posted that the page hasn't read yet: its messages but progress, at most
/// [`MAX_QUEUED`], its latest progress report, and why it failed (its error event, a message the
/// page can't read, or too many messages).
#[derive(Default)]
pub struct Inbox {
    messages: VecDeque<FromWorker>,
    progress: Option<(u64, u64)>,
    error: Option<String>,
}

impl Inbox {
    #[cfg(test)]
    pub(crate) fn has_waiting_call(&self) -> bool {
        self.messages.iter().any(|m| matches!(m, FromWorker::Call { .. }))
    }

    /// Takes message `msg` that the worker posted.
    pub fn push(&mut self, msg: &[u8]) {
        match FromWorker::decode(msg) {
            Some(FromWorker::Progress { done, total }) => self.progress = Some((done, total)),
            Some(m) if self.messages.len() < MAX_QUEUED => self.messages.push_back(m),
            _ => self.fail(unexpected()),
        }
    }

    /// The worker failed, as its error event `why` says.
    pub fn fail(&mut self, why: String) {
        self.error.get_or_insert(why);
    }

    /// Has the worker failed?
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
}

/// A worker's inbox, which its message and error handlers fill.
pub type SharedInbox = Rc<RefCell<Inbox>>;

/// One worker: what the page posts to it, and its end.
pub trait WorkerLink {
    /// Posts message `msg` to it.
    fn post(&mut self, msg: Vec<u8>) -> Result<(), String>;
    /// Ends it at once, whatever it is running.
    fn terminate(&mut self);
}

/// Starts the plug-in workers of the web build ([`crate::Services::plugin_workers`]).
pub trait Workers {
    /// A new worker whose messages and errors go to `inbox`, or why none can start.
    fn start(&self, inbox: SharedInbox) -> Result<Box<dyn WorkerLink>, String>;
    /// The page's clock in milliseconds, which budgets are measured on.
    fn now_ms(&self) -> f64;
}

/// [`crate::Services::plugin_workers`] for `workers`. The plug-ins installed from now on keep their
/// module's bytes, which a worker is given ([`vectorcraft_plugins::keep_modules`]).
pub fn workers(workers: Box<dyn Workers>) -> Option<Box<dyn Workers>> {
    vectorcraft_plugins::keep_modules(true);
    Some(workers)
}

/// A worker the page runs something in.
struct InWorker {
    link: Box<dyn WorkerLink>,
    inbox: SharedInbox,
    /// The request it gets once it is ready.
    request: Option<Vec<u8>>,
    /// When it must have answered, on [`Workers::now_ms`]'s clock.
    deadline: f64,
    /// Its run's output, waiting while another run holds the documents.
    held: Option<Vec<u8>>,
}

impl InWorker {
    /// Starts a worker for `request`, which must be answered within `budget_ms`.
    fn start(app: &VectorcraftApp, request: Vec<u8>, budget_ms: u64) -> Result<InWorker, String> {
        let workers = app.services.plugin_workers.as_ref().ok_or_else(unexpected)?;
        let inbox = SharedInbox::default();
        let link = workers.start(Rc::clone(&inbox))?;
        let deadline = workers.now_ms() + budget_ms as f64 + GRACE_MS;
        Ok(InWorker { link, inbox, request: Some(request), deadline, held: None })
    }

    /// Its next message, posting its request once it is ready → the message, or why it can't go on.
    /// A message waiting in its inbox is read even past the deadline.
    fn next(&mut self, now: f64) -> Result<Option<FromWorker>, String> {
        let mut inbox = self.inbox.borrow_mut();
        if let Some(e) = inbox.error.take() {
            return Err(e);
        }
        if now >= self.deadline && inbox.messages.is_empty() {
            return Err(vectorcraft_plugins::Error::Limit("time budget".into()).to_string());
        }
        match inbox.messages.pop_front() {
            Some(FromWorker::Ready { protocol }) if protocol == PROTOCOL => {
                drop(inbox);
                if now >= self.deadline {
                    return Err(vectorcraft_plugins::Error::Limit("time budget".into()).to_string());
                }
                let request = self.request.take().ok_or_else(unexpected)?;
                self.link.post(request)?;
                Ok(None)
            }
            Some(FromWorker::Ready { .. }) => Err(unexpected()),
            m => Ok(m),
        }
    }

    /// While a dialog is open, its answer waits in its inbox: this posts its request once it is
    /// ready → why it must end now (it failed, or its deadline passed with no final answer waiting),
    /// if it must. A command call waiting is not answered then, so it doesn't keep the run past its
    /// deadline.
    fn watch(&mut self, now: f64) -> Option<String> {
        while matches!(self.inbox.borrow().messages.front(), Some(FromWorker::Ready { .. })) {
            if let Err(e) = self.next(now) {
                return Some(e);
            }
        }
        let mut inbox = self.inbox.borrow_mut();
        if let Some(e) = inbox.error.take() {
            return Some(e);
        }
        let answered = inbox.messages.iter().any(|m| matches!(m, FromWorker::Done(..) | FromWorker::Checked(_)));
        (now >= self.deadline && !answered).then(|| vectorcraft_plugins::Error::Limit("time budget".into()).to_string())
    }
}

impl Drop for InWorker {
    fn drop(&mut self) {
        self.link.terminate();
    }
}

/// Where a filter runs.
enum Backend {
    /// On a thread of its own (desktop).
    Thread(Receiver<Done>),
    /// In a worker (web): the run, unless the session holds it (a filter that calls commands).
    Worker(Box<InWorker>, Option<Box<FilterRun>>),
}

/// A heavy filter running.
pub struct Running {
    /// What the status bar and the progress window say: `Running “Route Paths”…`.
    pub label: String,
    /// The filter's name, quoted.
    who: String,
    /// The plug-in's id, which the lines it logs in a worker carry.
    plugin: String,
    /// The command that applies its output (`plugin.run`, or a plug-in command on the web), and its
    /// params: the objects the run was given as `ids`.
    command: String,
    params: Value,
    /// How far it has got, as it reports it, and its Cancel.
    pub progress: Arc<Progress>,
    /// Continue in Background: the progress window no longer shows it.
    pub hidden: bool,
    /// [`export_progress::now`] when it started: the progress window's age.
    started: f64,
    backend: Backend,
}

impl Running {
    /// Does the progress window show it: neither hidden (Continue in Background) nor cancelled?
    fn shown(&self) -> bool {
        !self.hidden && !self.progress.cancelled()
    }

    /// Does it run a filter that calls commands, whose run the session holds?
    fn holds(&self) -> bool {
        matches!(self.backend, Backend::Worker(_, None))
    }
}

/// An install a worker checks first ([`check_install`]).
struct Check {
    worker: InWorker,
    /// `ui.installPlugin`'s params, to install with once it is checked.
    module: Value,
    hash: String,
    automated: bool,
    open_error: Option<String>,
}

/// The heavy filters running, and the installs being checked.
#[derive(Default)]
pub struct PluginRuns {
    pub running: Vec<Running>,
    checks: Vec<Check>,
    /// Is the app running the command that applies a worker run's output ([`routes`])?
    applying: bool,
}

impl PluginRuns {
    fn in_workers(&self) -> usize {
        self.checks.len() + self.running.iter().filter(|r| matches!(r.backend, Backend::Worker(..))).count()
    }
}

/// Do heavy filters run off the UI thread here? Not on wasm (no threads).
pub fn available() -> bool {
    !cfg!(target_arch = "wasm32")
}

/// Does every filter run in a worker here ([`crate::Services::plugin_workers`], the web build)?
pub fn in_workers(app: &VectorcraftApp) -> bool {
    app.services.plugin_workers.is_some()
}

/// Runs filter `p` (`plugin.run`'s params) on a thread of its own, or in a worker on the web →
/// `{background: true, status}`; where neither can run it, at once (`plugin.run`'s result).
pub fn start(app: &mut VectorcraftApp, p: Value) -> Result<Value, String> {
    if in_workers(app) {
        return start_in_worker(app, "plugin.run", p);
    }
    if !available() {
        return app.run("plugin.run", p);
    }
    // Bad parameters and a missing document fail here, before anything runs.
    let run = FilterRun::prepare(&app.session, &p).map_err(|e| e.to_string())?;
    // Typing Real-time Drawing off left out of the document is kept next, so the plug-in is given it.
    let run = match plugin::keep_unapplied_typing(&mut app.session).map_err(|e| e.to_string())? {
        true => FilterRun::prepare(&app.session, &p).map_err(|e| e.to_string())?,
        false => run,
    };
    let (who, params) = named(&run, p);
    let plugin = run.plugin().id().to_string();
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
    Ok(push(app, Started { who, plugin, command: "plugin.run".into(), params, progress: shown, backend: Backend::Thread(done) }))
}

/// The quoted name of `run`'s filter, and command params `p` that apply its output.
fn named(run: &FilterRun, p: Value) -> (String, Value) {
    let who = format!("“{}”", run.plugin().manifest().name.trim_end_matches('…'));
    let mut params = p;
    params["ids"] = json!(run.inputs().iter().map(|i| i.0).collect::<Vec<_>>());
    (who, params)
}

/// A run being added ([`push`]): the quoted name and the id of its plug-in, the command and params
/// that apply its output, its progress and where it runs.
struct Started {
    who: String,
    plugin: String,
    command: String,
    params: Value,
    progress: Arc<Progress>,
    backend: Backend,
}

/// Adds the run, says so in the status bar → `start`'s result.
fn push(app: &mut VectorcraftApp, s: Started) -> Value {
    let Started { who, plugin, command, params, progress, backend } = s;
    let label = format!("Running {who}…");
    let started = export_progress::now(app);
    let runs = &mut app.plugin_runs;
    runs.running.push(Running { label: label.clone(), who, plugin, command, params, progress, hidden: false, started, backend });
    app.status(label.clone());
    json!({ "background": true, "status": label })
}

/// Does command `id` go to a worker here: a plug-in command, on the web, that the app doesn't run
/// to apply a worker's output ([`start_command`])? A command automation runs directly runs where
/// it is called and returns its result, as `plugin.run` does.
pub fn routes(app: &VectorcraftApp, id: &str) -> bool {
    in_workers(app) && vectorcraft_engine::cmd::plugin::is_command_id(id) && !app.plugin_runs.applying && !app.automation_asked()
}

/// Runs plug-in command `id` (`plugin.<id>.<command>`) with params `p` in a worker (the web) →
/// `{background: true, status}`. It applies as the command, as one undo step named after it.
pub fn start_command(app: &mut VectorcraftApp, id: &str, p: Value) -> Result<Value, String> {
    start_in_worker(app, id, p)
}

/// [`start`] on the web: the run of command `c` (`plugin.run`, or a plug-in command) goes to a
/// worker of its own. A filter that calls commands holds the documents until it ends
/// ([`FilterRun::begin_calls`]).
fn start_in_worker(app: &mut VectorcraftApp, c: &str, p: Value) -> Result<Value, String> {
    // Its output could not apply before the script ends.
    if app.session.script_running() {
        return Err(EngineError::Busy.to_string());
    }
    if app.plugin_runs.in_workers() >= MAX_WORKERS || app.session.plugin_calls_open() {
        return Err(EngineError::PluginBusy.to_string());
    }
    vectorcraft_plugins::safe_mode::check().map_err(|e| e.to_string())?;
    // Bad parameters fail first; then the edit in progress is its own undo step, so the run applies
    // as one of its own, given the document with that edit.
    let run = FilterRun::prepare_for(&app.session, c, &p).map_err(|e| e.to_string())?;
    let run = match app.session.settle_for_plugin_run().map_err(|e| e.to_string())? {
        true => FilterRun::prepare_for(&app.session, c, &p).map_err(|e| e.to_string())?,
        false => run,
    };
    let calls = run.calls();
    let (who, params) = named(&run, p);
    let (job, run) = match calls {
        true => (FilterRun::begin_calls(&mut app.session, c, &params).map_err(|e| e.to_string())?, None),
        false => (run.job(), Some(run)),
    };
    let started = job.plugin.module_bytes().ok_or_else(unexpected).and_then(|module| {
        let request = ToWorker::Run {
            module: module.to_vec(),
            manifest: job.plugin.manifest_text().to_vec(),
            input: job.input,
            params: job.params,
            limits: job.limits.clone(),
            calls,
        };
        InWorker::start(app, request.encode(), job.limits.wall_time_ms)
    });
    let worker = match started {
        Ok(w) => w,
        Err(e) => {
            if calls {
                app.session.end_plugin_calls();
            }
            return Err(e);
        }
    };
    let plugin = job.plugin.id().to_string();
    let backend = Backend::Worker(Box::new(worker), run.map(Box::new));
    Ok(push(app, Started { who, plugin, command: c.to_string(), params, progress: Progress::new(), backend }))
}

/// On the web, checks the module that `ui.installPlugin`'s params `module` give (`dataBase64`) in a
/// worker before the page loads it, when no worker has checked these bytes yet → `{pending: true}`
/// while it checks: once it has, the install goes on as it would have ([`crate::io::open`]), and the
/// page loads the module without running any of its code. `None`: nothing to check here (no
/// workers, bytes already checked, or a module the page refuses before it runs anything).
pub fn check_install(app: &mut VectorcraftApp, module: &Value, automated: bool) -> Option<Result<Value, String>> {
    check_install_with_context(app, module, automated, None)
}

pub(crate) fn check_open_install(app: &mut VectorcraftApp, module: &Value, name: &str) -> Option<Result<Value, String>> {
    let automated = app.automation_asked();
    check_install_with_context(app, module, automated, Some(name.to_string()))
}

fn check_install_with_context(
    app: &mut VectorcraftApp,
    module: &Value,
    automated: bool,
    open_error: Option<String>,
) -> Option<Result<Value, String>> {
    if !in_workers(app) {
        return None;
    }
    let data = module.get("dataBase64")?.as_str()?;
    let limits = vectorcraft_plugins::Limits::default();
    // Too large, or not base64: `plugin.inspect` refuses it before anything runs.
    if data.len() > limits.max_module_bytes.div_ceil(3) * 4 {
        return None;
    }
    let bytes = vectorcraft_format::base64_decode(data)?;
    let hash = vectorcraft_plugins::permissions::sha256_hex(&bytes);
    if app.session.plugin_checked(&hash) {
        return None;
    }
    if app.plugin_runs.in_workers() >= MAX_WORKERS {
        return Some(Err(EngineError::PluginBusy.to_string()));
    }
    let worker = match InWorker::start(app, ToWorker::Check { module: bytes }.encode(), limits.wall_time_ms) {
        Ok(w) => w,
        Err(e) => return Some(Err(e)),
    };
    app.plugin_runs.checks.push(Check { worker, module: module.clone(), hash, automated, open_error });
    Some(Ok(json!({ "pending": true })))
}

/// The page's clock, when workers run plug-ins.
fn now(app: &VectorcraftApp) -> f64 {
    app.services.plugin_workers.as_ref().map_or(0.0, |w| w.now_ms())
}

/// Applies the runs that are done (each frame). Not while a dialog is open: its OK acts on the
/// active document, which a run's result changes; the results wait until it is closed. Worker runs
/// and install checks are read from their inboxes, and their command calls are made; a checked
/// install waits for the dialog too, since it may ask in one. While a dialog is open, a worker
/// still gets its request, and Cancel, a passed deadline and a change to its document still end
/// it. While a script runs, the runs wait as they do for a dialog: every change to the documents
/// is refused until it ends.
pub fn poll(app: &mut VectorcraftApp) {
    poll_checks(app);
    let mut i = 0;
    while let Some(r) = app.plugin_runs.running.get(i) {
        let dialog = app.any_dialog_open() || app.session.script_running();
        let ended = match &r.backend {
            Backend::Thread(_) if dialog => None,
            Backend::Worker(..) if dialog => watch_worker(app, i),
            Backend::Thread(done) => match done.try_recv() {
                Ok(done) => Some(Ended::Thread(Some(Box::new(done)))),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Ended::Thread(None)),
            },
            Backend::Worker(..) => poll_worker(app, i),
        };
        let Some(ended) = ended else {
            i += 1;
            continue;
        };
        let r = app.plugin_runs.running.remove(i);
        // The status bar no longer says it is running.
        if app.ui.status == r.label {
            app.ui.status.clear();
        }
        match ended {
            Ended::Thread(done) => finish(app, r, done.map(|d| *d)),
            Ended::Worker(out) => finish_worker(app, r, out),
        }
    }
}

/// How a run ended.
enum Ended {
    Thread(Option<Box<Done>>),
    /// Its output, or why it has none; `None`: its document or its plug-in changed.
    Worker(Option<Result<Vec<u8>, String>>),
}

/// Worker run `i` while a dialog is open ([`InWorker::watch`]) → how it ended, if Cancel, its
/// deadline or a change to its document or its plug-in ended it.
fn watch_worker(app: &mut VectorcraftApp, i: usize) -> Option<Ended> {
    let t = now(app);
    let r = app.plugin_runs.running.get_mut(i)?;
    if r.progress.cancelled() {
        return Some(Ended::Worker(Some(Err(EngineError::Cancelled.to_string()))));
    }
    let Backend::Worker(worker, run) = &mut r.backend else { return None };
    // As in [`poll_worker`]: its output can't apply.
    if run.as_ref().is_some_and(|run| !run.current(&app.session) || plugin::unapplied_typing(&app.session)) {
        return Some(Ended::Worker(None));
    }
    if worker.held.is_some() {
        return None;
    }
    let failed = {
        let mut inbox = worker.inbox.borrow_mut();
        let at = inbox.messages.iter().position(|message| matches!(message, FromWorker::Done(Err(_), _)));
        at.and_then(|at| inbox.messages.remove(at))
    };
    if let Some(FromWorker::Done(Err(error), lines)) = failed {
        vectorcraft_plugins::worker::log_lines(&r.plugin, &lines);
        return Some(Ended::Worker(Some(Err(error.to_string()))));
    }
    worker.watch(t).map(|e| Ended::Worker(Some(Err(e))))
}

/// Reads the inbox of worker run `i` → how it ended, if it has.
fn poll_worker(app: &mut VectorcraftApp, i: usize) -> Option<Ended> {
    let t = now(app);
    let r = app.plugin_runs.running.get_mut(i)?;
    if r.progress.cancelled() {
        return Some(Ended::Worker(Some(Err(EngineError::Cancelled.to_string()))));
    }
    let Backend::Worker(worker, run) = &mut r.backend else { return None };
    // A run whose document or plug-in changed is stopped at once: its output can't apply.
    if run.as_ref().is_some_and(|run| !run.current(&app.session) || plugin::unapplied_typing(&app.session)) {
        return Some(Ended::Worker(None));
    }
    // An output that waited while another run held the documents applies once they are free.
    if let Some(output) = worker.held.take() {
        if app.session.plugin_calls_open() {
            worker.held = Some(output);
            return None;
        }
        return Some(Ended::Worker(Some(Ok(output))));
    }
    if let Some((done, total)) = worker.inbox.borrow_mut().progress.take() {
        progress::scope(&r.progress, || vectorcraft_doc::cancel::progress(done, total, vectorcraft_plugins::host::PROGRESS_STEP));
    }
    loop {
        let r = app.plugin_runs.running.get_mut(i)?;
        let Backend::Worker(worker, run) = &mut r.backend else { return None };
        let holds = run.is_none();
        match worker.next(t) {
            Err(e) => return Some(Ended::Worker(Some(Err(e)))),
            Ok(None) if worker.inbox.borrow().messages.is_empty() => return None,
            Ok(None) => {}
            Ok(Some(FromWorker::Call { command, args })) if holds => {
                let answer = app.session.plugin_call(&command, &args);
                let r = app.plugin_runs.running.get_mut(i)?;
                let Backend::Worker(worker, _) = &mut r.backend else { return None };
                if let Err(e) = worker.link.post(ToWorker::Answer(answer).encode()) {
                    return Some(Ended::Worker(Some(Err(e))));
                }
            }
            Ok(Some(FromWorker::Done(out, lines))) => {
                vectorcraft_plugins::worker::log_lines(&r.plugin, &lines);
                return match out {
                    // Another run holds the documents: this output waits for it to end.
                    Ok(output) if !holds && app.session.plugin_calls_open() => {
                        worker.held = Some(output);
                        None
                    }
                    out => Some(Ended::Worker(Some(out.map_err(|e| e.to_string())))),
                };
            }
            Ok(Some(_)) => return Some(Ended::Worker(Some(Err(unexpected())))),
        }
    }
}

/// Applies worker run `r`'s output, as [`finish`] does a thread's: through `plugin.run`, in the
/// undo step the session holds for a filter that calls commands. `None`: it stopped because its
/// document or its plug-in changed.
fn finish_worker(app: &mut VectorcraftApp, r: Running, out: Option<Result<Vec<u8>, String>>) {
    let Backend::Worker(worker, run) = r.backend else { return };
    // The worker ends here, whatever it was doing.
    drop(worker);
    let changed = |app: &mut VectorcraftApp| app.status(format!("{} stopped: the document or the plug-in changed meanwhile: run it again", r.who));
    let output = match out {
        Some(Ok(output)) => output,
        failed => {
            if run.is_none() {
                app.session.end_plugin_calls();
            }
            match failed {
                _ if r.progress.cancelled() => app.status(EngineError::Cancelled.to_string()),
                Some(Err(e)) => app.status(format!("{} failed: {e}", r.who)),
                _ => changed(app),
            }
            return;
        }
    };
    let holds = run.is_none();
    match run {
        Some(run) => {
            if !run.current(&app.session) || plugin::unapplied_typing(&app.session) {
                changed(app);
                return;
            }
            (*run).hand_in(&mut app.session, output);
        }
        None => app.session.hand_in_plugin_calls(output),
    }
    app.plugin_runs.applying = true;
    let applied = app.run(&r.command, r.params);
    app.plugin_runs.applying = false;
    if let Err(e) = applied {
        // This run, when the session still holds it, changes nothing; another run that holds the
        // documents goes on.
        if holds {
            app.session.end_plugin_calls();
        }
        app.status(format!("{} failed: {e}", r.who));
    }
}

/// Reads the install checks' inboxes, and goes on with each checked install; while a dialog is
/// open (one an install before it opened to ask, too), a run holds the documents or a script runs,
/// only a failed or overdue check ends ([`InWorker::watch`]), and a checked install waits.
fn poll_checks(app: &mut VectorcraftApp) {
    let t = now(app);
    let mut i = 0;
    loop {
        let dialog = app.any_dialog_open() || app.session.plugin_calls_open() || app.session.script_running();
        let Some(c) = app.plugin_runs.checks.get_mut(i) else { break };
        let checked = loop {
            if dialog {
                if let Some(e) = c.worker.watch(t) {
                    break Some(Err(e));
                }
                // A failed check ends at once; only a checked install waits.
                if !matches!(c.worker.inbox.borrow().messages.front(), Some(FromWorker::Checked(Err(_)))) {
                    break None;
                }
            }
            match c.worker.next(t) {
                Err(e) => break Some(Err(e)),
                Ok(Some(FromWorker::Checked(r))) => break Some(r.map_err(|e| e.to_string())),
                Ok(Some(_)) => break Some(Err(unexpected())),
                Ok(None) if c.worker.inbox.borrow().messages.is_empty() => break None,
                Ok(None) => {}
            }
        };
        let Some(checked) = checked else {
            i += 1;
            continue;
        };
        let c = app.plugin_runs.checks.remove(i);
        let Check { worker, module, hash, automated, open_error } = c;
        drop(worker);
        let r = checked.and_then(|manifest| {
            app.session.hand_in_checked_plugin(&hash, manifest);
            crate::io::install_module_as(app, module, automated)
        });
        if let Err(e) = r {
            if let Some(name) = open_error {
                crate::io::report_open_error(app, &name, &e);
            } else {
                app.status(e);
            }
        }
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
    // Typing started meanwhile with Real-time Drawing off isn't in what the plug-in was given.
    if !run.current(&app.session) || plugin::unapplied_typing(&app.session) {
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
        .map(|name| name.trim().trim_end_matches('…').trim_end().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_string());
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
/// `vc_cancelled`, else at the next slice of its budget; in a worker, at once) and changes nothing → `{cancelled: n}`.
pub fn cancel_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.plugin_runs.running.iter().filter(|r| !r.progress.cancelled()) {
        r.progress.cancel();
        n += 1;
    }
    // A worker run ends now, and one that holds the documents frees them.
    poll(app);
    json!({ "cancelled": n })
}

/// The progress window's Continue in Background: the runs go on, shown in the status bar only →
/// `{hidden: n}`. A run that holds the documents stays shown.
pub fn hide_all(app: &mut VectorcraftApp) -> Value {
    let mut n = 0;
    for r in app.plugin_runs.running.iter_mut().filter(|r| r.shown() && !r.holds()) {
        r.hidden = true;
        n += 1;
    }
    json!({ "hidden": n })
}

/// Is a filter running, or an install being checked? The frames go on then, to read their results
/// and end those past their deadline.
pub fn any_pending(app: &VectorcraftApp) -> bool {
    !app.plugin_runs.running.is_empty() || !app.plugin_runs.checks.is_empty()
}

/// Is a heavy filter running that hasn't been cancelled?
pub fn any_running(app: &VectorcraftApp) -> bool {
    app.plugin_runs.running.iter().any(|r| !r.progress.cancelled())
}

/// Is a heavy filter running that the progress window shows?
pub fn any_shown(app: &VectorcraftApp) -> bool {
    app.plugin_runs.running.iter().any(Running::shown)
}

/// Is a filter running that holds the documents? Every command but Cancel waits for it then.
pub fn holds_documents(app: &VectorcraftApp) -> bool {
    app.session.plugin_calls_open() && app.plugin_runs.running.iter().any(Running::holds)
}

/// The progress window of the first run it shows. `export_progress::show` orders the job tables and
/// decides when a window may appear. The run that holds the documents comes first, since only its
/// Cancel goes through; its window shows at once and has Cancel only.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<export_progress::Shown> {
    let runs = &app.plugin_runs.running;
    let first = runs.iter().find(|r| r.shown() && r.holds()).or_else(|| runs.iter().find(|r| r.shown()))?;
    let holds = first.holds();
    Some(export_progress::Shown {
        title: crate::i18n::msg(&first.label).to_string(),
        step: tl!("The plug-in is computing…").to_string(),
        fraction: first.progress.fraction().map(|f| f as f32),
        counts: None,
        queued: None,
        // A window shows once its job has run `DELAY`: a holding run's counts from `DELAY` earlier.
        started: if holds { first.started - export_progress::DELAY } else { first.started },
        cancel: "plugin.cancelRuns",
        hide: (!holds).then_some("plugin.hideRuns"),
    })
}
