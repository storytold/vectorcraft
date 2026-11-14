//! Running a script: `script.run`, `script.stop` and the job behind them.
//!
//! A run works on its own copy of the open documents and the session state commands change (the
//! working copy). Between two slices of a run, the session holds the state as it was before the
//! run: what is drawn, what a read-only command returns and what Data Recovery copies never show a
//! script's unfinished work. Each slice swaps the working copy in, advances the interpreter by one
//! measured slice of fuel and swaps it out. A commit makes the working copy the session's state,
//! with one undo step for each document whose art changed. A rollback drops the working copy.
//!
//! While a job exists, [`Session::execute`] refuses any command that can edit, select, save or give
//! a document a new revision, and the tool events are refused the same way (the mutation lease).
//! The script's own commands run through [`Session::execute`] inside a slice, each with its own
//! parameter checks, enabled rule, undo history and journal entry, and a command that fails leaves
//! no trace of itself. Inside a run, `edit.undo` reaches only steps the script made.
//!
//! Files a script's commands saved or exported before it failed stay written, and the failure names
//! them. Ids given out in a run are never given out again for a document, also after an in-run
//! undo or a rollback.
//!
//! The values here are VectorCraft's own limits: [`MAX_SOURCE_BYTES`], [`DEFAULT_FUEL`],
//! [`MAX_FUEL`], [`SLICE_FUEL`], [`HEAP_BYTES`], [`MAX_HOST_CALLS`], [`GROWTH_OBJECTS`],
//! [`GROWTH_BYTES`], [`MAX_CONTINUES`], [`MAX_LOG_LINES`], [`MAX_LOG_BYTES`], [`MAX_ARGS_BYTES`],
//! and the 64 levels, 16 MiB and 1,048,576 entries of [`vectorcraft_js::DataLimits`]'s default,
//! and the console's ([`console`]).
//!
//! Before a run starts, its preprocessor lines are read ([`include`]): included files take the
//! place of their lines, and an error in one is reported at its own file, line and column.
//! `$.evalFile(path)` runs another file in the same global environment and gives its last value,
//! `$.fileName` is the file of the code that reads it (the file being run for code that `eval` or
//! `Function` compiled) and `$.includePath` the `#includepath` list.
//!
//! A run writes to the Script Console with `$.write` and `$.writeln`, and `vc.log` adds a line
//! there too. The session's console gets a start line, the run's output, its uncaught error, an end
//! line and a report line; the run's report holds its own output and error, its time, its host
//! calls by kind and its slowest commands. The rest of `$`, and the global `localize` and
//! `toSource`, are in [`dollar`].
//!
//! `alert`, `confirm` and `prompt` open dialogs ([`dialogs`]): in the app the run waits for the
//! person's answer, and a run with nobody to ask takes its answers from `script.run`'s `answers`.
//!
//! `File` and `Folder` read files inside the script's own folder ([`files`]).
//!
//! A script with `#targetengine name` runs in that named engine, which the session keeps with the
//! globals its runs leave, at most [`MAX_ENGINES`] of them with [`ENGINE_HEAP_BYTES`] of heap each
//! (VectorCraft's own limits). A script without the line, or with `main`, gets a new realm for each
//! run. An engine keeps no document and no run: its host functions reach the run through a link
//! that holds the run's host only while the run goes on. An error, a limit or a stop puts the
//! documents back and keeps the engine's globals; an internal error drops the engine.

pub mod console;
pub mod dialogs;
pub mod dollar;
pub mod include;

pub use dialogs::{DialogAnswer, DialogKind, ScriptDialog};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{
    Data, DataLimits, HostError, HostFunction, HostItem, Interrupt, LimitKind, Limits, Options, Progress, Realm, RunError, RunErrorKind,
};

use crate::{
    Clipboard, DocState, DrawMode, EngineError, Journal, NodeId, PaintDefaults, Result, Session, cmd, command_specs, guard, waits_for_tool_edit,
};

pub mod files;
pub mod folder;

/// The most bytes of source a run takes, as a file or as `source`.
pub const MAX_SOURCE_BYTES: usize = 8 << 20;
/// The instructions a run may execute when `maxFuel` is left out.
pub const DEFAULT_FUEL: u64 = 1_000_000_000;
/// The largest `maxFuel`.
pub const MAX_FUEL: u64 = 100_000_000_000;
/// The instructions one slice of a run executes before the interface draws again.
pub const SLICE_FUEL: u64 = 500_000;
/// The heap one run may use.
#[cfg(not(target_arch = "wasm32"))]
pub const HEAP_BYTES: usize = 256 << 20;
/// The heap one run may use in a wasm build.
#[cfg(target_arch = "wasm32")]
pub const HEAP_BYTES: usize = 128 << 20;
/// The most `vc.run` calls and getter reads of one run.
pub const MAX_HOST_CALLS: u64 = 2_000_000;
/// The objects a run may make before it is asked to continue or stop.
pub const GROWTH_OBJECTS: u64 = 100_000;
/// The bytes of new document data a run may keep before it is asked to continue or stop.
pub const GROWTH_BYTES: u64 = 512 << 20;
/// How many times a person may let a run go past its growth allowance; each time adds one more.
pub const MAX_CONTINUES: u32 = 8;
/// The most `vc.log` entries of one run.
pub const MAX_LOG_LINES: usize = 10_000;
/// The most bytes of `vc.log` entries of one run.
pub const MAX_LOG_BYTES: usize = 4 << 20;
/// The most bytes of `args` given to one run, as JSON.
pub const MAX_ARGS_BYTES: usize = 1 << 20;
/// The `vc.run` calls one slice makes before the interface draws again.
const SLICE_HOST_CALLS: u32 = 256;
/// The bytes that may cross between a script and the host in one run.
const MAX_BRIDGE_BYTES: u64 = 4 << 30;

/// The version `vc.api.version` gives.
pub const API_VERSION: &str = "1.0";
/// The features `vc.api.has` knows.
pub const API_FEATURES: &[&str] = &["run", "document", "commands", "log", "args", "console", "dialogs", "dollar"];
/// The most commands a run's report lists in `slowestCommands`.
pub const SLOWEST_COMMANDS: usize = 5;

/// What the step of a run's one undo step is called.
pub const STEP_LABEL: &str = "Run Script";

/// `id`, as `vc.run` is given it, is one a script may not run: a command that would leave the run's
/// transaction, change something outside the documents that a rollback could not put back, or clear
/// the undo steps from before the run, which the run keeps.
pub fn refused_to_scripts(id: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "script.",
        "plugin.",
        "prefs.",
        "actionSet.",
        "shortcutSet.",
        "help.",
        "print.",
        "library.",
        "importMatches.",
        "edit.undoGroup.",
        "file.recovery.",
        "stroke.widthProfile.",
    ];
    const IDS: &[&str] = &[
        "text.addFontFiles",
        "text.findFontFiles",
        "text.rescanFonts",
        "links.findFiles",
        "file.place.queue",
        "color.loadProfile",
        "color.removeProfile",
        "file.print",
        "view.proofSetup",
        "view.proofColors",
        "view.overprintPreview",
        "view.separationsPreview",
        "edit.clearHistory",
        "swatch.defaults.save",
        "swatch.defaults.reset",
    ];
    let saved_preset = (id.contains("resets.") || id.contains(".preset.")) && !id.ends_with(".list") && !id.ends_with(".apply");
    id != "script.check" && (saved_preset || PREFIXES.iter().any(|p| id.starts_with(p)) || IDS.contains(&id))
}

/// The error for a script's command that [`refused_to_scripts`] names.
pub(crate) fn refused_text(id: &str) -> String {
    format!("a script cannot run `{id}`")
}

/// A request to run a script.
pub(crate) struct Request {
    /// The run's name; unless `name_given`, a `#script` line in the script replaces it.
    pub name: String,
    pub name_given: bool,
    /// The file the script was read from, which its included files are read beside.
    pub file: Option<String>,
    pub source: String,
    pub args: Map<String, Value>,
    pub fuel: u64,
    /// The locale the run starts with (`$.locale`).
    pub locale: String,
    /// The answers to the dialogs of a run with nobody to ask, in order.
    pub answers: Vec<DialogAnswer>,
}

thread_local! {
    /// The running jobs of this thread's sessions. A job is held here, and not in its session,
    /// because it holds the interpreter, which stays on the thread that started it.
    static JOBS: RefCell<HashMap<u64, Job>> = RefCell::new(HashMap::new());
}

static NEXT_JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A session's hold on its running job: while one exists the lease is held. Dropping it ends the job.
/// It keeps the run's number and its name as console lines give it, so the run's console entry can
/// end when the job is gone.
#[derive(Debug)]
pub struct JobKey {
    id: u64,
    run: u64,
    name: Arc<str>,
}

impl Drop for JobKey {
    fn drop(&mut self) {
        let _ = JOBS.try_with(|jobs| {
            if let Ok(mut jobs) = jobs.try_borrow_mut() {
                jobs.remove(&self.id);
            }
        });
    }
}

fn with_job<T>(key: &JobKey, f: impl FnOnce(&mut Job) -> T) -> Option<T> {
    JOBS.with(|jobs| jobs.borrow_mut().get_mut(&key.id).map(f))
}

/// The most named engines (`#targetengine`) a session keeps.
pub const MAX_ENGINES: usize = 8;
/// The heap a named engine may use, with what it keeps between runs.
pub const ENGINE_HEAP_BYTES: usize = 64 << 20;
/// The engine of a script without `#targetengine`, which `#targetengine main` also names: a new
/// one for each run, never kept.
pub const MAIN_ENGINE: &str = "main";

/// The run a script's host functions act for. A named engine keeps its functions between runs, so
/// they find the run through this link, which holds the run's [`Host`] only while the run goes on.
type Link = Rc<RefCell<Option<Rc<Host>>>>;

/// The host of the run `link` holds, or an error when no run holds it.
fn current(link: &Link) -> std::result::Result<Rc<Host>, HostError> {
    link.borrow().clone().ok_or_else(|| HostError::Error(NO_RUN.to_string()))
}

/// A named engine: its interpreter, with the globals its runs left (`None` while a run has it),
/// the link its host functions use, its heap after its last run, and the source maps of the code
/// its earlier runs compiled.
struct Engine {
    name: String,
    realm: Option<Realm>,
    link: Link,
    heap: usize,
    maps: Rc<RefCell<KeptMaps>>,
}

/// A named engine taken for a run: its realm (`None` for an engine made now), its link and its
/// kept source maps.
type TakenEngine = (Option<Realm>, Link, Rc<RefCell<KeptMaps>>);

/// The most bytes of source maps ([`include::SourceMap::bytes`]) a named engine keeps from its
/// earlier runs.
pub const KEPT_MAP_BYTES: usize = 8 << 20;

/// A source map an engine kept from an earlier run: the realm's tag for the source
/// ([`Realm::error_tag`]), the map, and for the run's own script, that run's name, which stands for
/// a script given as text.
struct KeptMap {
    tag: u32,
    map: include::SourceMap,
    script: Option<Arc<str>>,
    bytes: usize,
}

/// The source maps a named engine kept from its earlier runs, by tag, oldest first, in at most
/// [`KEPT_MAP_BYTES`]: the oldest go first to make room, and a map larger than that is not kept.
#[derive(Default)]
struct KeptMaps {
    maps: std::collections::VecDeque<KeptMap>,
    bytes: usize,
}

impl KeptMaps {
    fn keep(&mut self, tag: u32, map: include::SourceMap, script: Option<Arc<str>>) {
        let bytes = map.bytes() + script.as_ref().map_or(0, |s| s.len());
        let limit = kept_map_bytes();
        if bytes > limit || self.maps.back().is_some_and(|m| m.tag >= tag) {
            return;
        }
        while self.bytes + bytes > limit {
            let Some(old) = self.maps.pop_front() else { break };
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        self.maps.push_back(KeptMap { tag, map, script, bytes });
    }

    fn get(&self, tag: u32) -> Option<&KeptMap> {
        self.maps.binary_search_by_key(&tag, |m| m.tag).ok().and_then(|i| self.maps.get(i))
    }
}

/// The bytes of source maps a named engine keeps.
fn kept_map_bytes() -> usize {
    #[cfg(test)]
    if let Some(n) = TEST_KEPT_MAP_BYTES.with(Cell::get) {
        return n;
    }
    KEPT_MAP_BYTES
}

/// The named engines of one thread's sessions, by their key, each in the order they were made.
struct ThreadEngines {
    thread: std::thread::ThreadId,
    by_key: HashMap<u64, Vec<Engine>>,
}

impl ThreadEngines {
    fn new() -> Self {
        let thread = std::thread::current().id();
        released().insert(thread, Vec::new());
        Self { thread, by_key: HashMap::new() }
    }
}

impl Drop for ThreadEngines {
    fn drop(&mut self) {
        released().remove(&self.thread);
    }
}

thread_local! {
    /// The named engines of this thread's sessions. They are held here, and not in their session,
    /// because they hold interpreters, which stay on this thread.
    static ENGINES: RefCell<ThreadEngines> = RefCell::new(ThreadEngines::new());
}

static NEXT_ENGINES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// For each thread that holds engines, the keys of the ones sessions let go of on another thread.
/// The thread drops them the next time it uses its engines; a thread that has ended has no entry,
/// since its engines ended with it.
static RELEASED_ENGINES: std::sync::LazyLock<std::sync::Mutex<HashMap<std::thread::ThreadId, Vec<u64>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn released() -> std::sync::MutexGuard<'static, HashMap<std::thread::ThreadId, Vec<u64>>> {
    RELEASED_ENGINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Lets the engines with key `id` go, for `thread`, which holds them, to drop.
fn release_engines(thread: std::thread::ThreadId, id: u64) {
    if let Some(ids) = released().get_mut(&thread) {
        ids.push(id);
    }
}

/// A session's hold on its named engines: their key, and the thread that holds them once a run
/// made one. Dropping it drops them. The engines stay on that thread, so a session used on another
/// thread lets them go and starts there with none.
#[derive(Debug)]
pub struct EnginesKey(std::sync::Mutex<(u64, Option<std::thread::ThreadId>)>);

impl EnginesKey {
    pub(crate) fn new() -> Self {
        Self(std::sync::Mutex::new((NEXT_ENGINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed), None)))
    }

    /// The key of the session's engines on this thread.
    fn here(&self) -> u64 {
        let mut key = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let thread = std::thread::current().id();
        match key.1 {
            Some(t) if t != thread => {
                release_engines(t, key.0);
                *key = (NEXT_ENGINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed), Some(thread));
            }
            Some(_) => {}
            None => key.1 = Some(thread),
        }
        key.0
    }
}

impl Drop for EnginesKey {
    fn drop(&mut self) {
        let (id, thread) = *self.0.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = thread.filter(|t| *t != std::thread::current().id()) {
            release_engines(t, id);
            return;
        }
        let gone = ENGINES.try_with(|engines| engines.try_borrow_mut().ok().and_then(|mut e| e.by_key.remove(&id)));
        drop(gone);
    }
}

fn with_engines<T>(key: &EnginesKey, f: impl FnOnce(&mut Vec<Engine>) -> T) -> T {
    let id = key.here();
    ENGINES.with(|engines| {
        let gone: Vec<Vec<Engine>> = {
            let mut here = engines.borrow_mut();
            let ids = released().get_mut(&here.thread).map(std::mem::take).unwrap_or_default();
            ids.iter().filter_map(|id| here.by_key.remove(id)).collect()
        };
        drop(gone);
        f(engines.borrow_mut().by_key.entry(id).or_default())
    })
}

/// Tests: the named engines this thread holds, for every session.
#[cfg(test)]
pub(crate) fn engines_on_this_thread() -> usize {
    ENGINES.with(|engines| engines.borrow().by_key.values().map(Vec::len).sum())
}

/// Tests: whether engines can still be let go for `thread`, which holds an entry while it lives.
#[cfg(test)]
pub(crate) fn releases_kept_for(thread: std::thread::ThreadId) -> bool {
    released().contains_key(&thread)
}

/// The heap of a named engine.
fn engine_heap() -> usize {
    #[cfg(test)]
    if let Some(n) = TEST_ENGINE_HEAP.with(Cell::get) {
        return n;
    }
    ENGINE_HEAP_BYTES
}

/// The state a script changes that a rollback puts back.
struct World {
    docs: Vec<DocState>,
    active: Option<usize>,
    paint: PaintDefaults,
    fill_active: bool,
    draw_mode: DrawMode,
    draw_inside: Option<NodeId>,
    clipboard: Clipboard,
    untitled_counter: u32,
    menu: cmd::menucmds::MenuState,
    last_solid: crate::Color,
    last_gradient: crate::GradientPaint,
    recent_colors: Vec<crate::Color>,
    pending_paint: Option<crate::Paint>,
    choices: Choices,
    journal: Journal,
    /// The preferences when the run started.
    start_prefs: crate::Prefs,
    /// The Caps Lock state when the run started.
    start_caps_lock: bool,
}

/// The choices outside the documents that a script's commands can change: the preferences, the
/// global edit options, the recent URLs, the chosen gradient stop, freeform point and appearance
/// item, the Eyedropper's sample, the envelope defaults and the Caps Lock state.
#[derive(Clone)]
struct Choices {
    prefs: crate::Prefs,
    caps_lock: bool,
    eyedropper_sample: Option<Arc<vectorcraft_doc::Node>>,
    global_edit_options: cmd::globaledit::Options,
    recent_urls: Vec<String>,
    gradient_stop: Option<(usize, cmd::gradient::StopOwner)>,
    freeform_point: Option<(usize, cmd::gradient::StopOwner)>,
    active_appearance_item: Option<cmd::appearance::ActiveItem>,
    envelope_defaults: Option<(vectorcraft_doc::live::EnvelopeOptions, f64)>,
}

impl Choices {
    fn of(s: &Session) -> Self {
        Self {
            prefs: s.prefs.clone(),
            caps_lock: s.caps_lock,
            eyedropper_sample: s.eyedropper_sample.clone(),
            global_edit_options: s.global_edit_options.clone(),
            recent_urls: s.recent_urls.clone(),
            gradient_stop: s.gradient_stop,
            freeform_point: s.freeform_point,
            active_appearance_item: s.active_appearance_item.clone(),
            envelope_defaults: s.envelope_defaults,
        }
    }

    fn put(self, s: &mut Session) {
        s.prefs = self.prefs;
        s.caps_lock = self.caps_lock;
        s.eyedropper_sample = self.eyedropper_sample;
        s.global_edit_options = self.global_edit_options;
        s.recent_urls = self.recent_urls;
        s.gradient_stop = self.gradient_stop;
        s.freeform_point = self.freeform_point;
        s.active_appearance_item = self.active_appearance_item;
        s.envelope_defaults = self.envelope_defaults;
    }

    fn swap(&mut self, s: &mut Session) {
        use std::mem::swap;
        swap(&mut self.prefs, &mut s.prefs);
        swap(&mut self.caps_lock, &mut s.caps_lock);
        swap(&mut self.eyedropper_sample, &mut s.eyedropper_sample);
        swap(&mut self.global_edit_options, &mut s.global_edit_options);
        swap(&mut self.recent_urls, &mut s.recent_urls);
        swap(&mut self.gradient_stop, &mut s.gradient_stop);
        swap(&mut self.freeform_point, &mut s.freeform_point);
        swap(&mut self.active_appearance_item, &mut s.active_appearance_item);
        swap(&mut self.envelope_defaults, &mut s.envelope_defaults);
    }
}

/// The preferences after a run: `current`, the session's, which the app may have changed while the
/// run went on, with each preference the run changed from `start` set to the run's value.
fn merged_prefs(start: &crate::Prefs, run: &crate::Prefs, current: &crate::Prefs) -> crate::Prefs {
    let (Ok(Value::Object(start)), Ok(Value::Object(run)), Ok(Value::Object(mut out))) =
        (serde_json::to_value(start), serde_json::to_value(run), serde_json::to_value(current))
    else {
        return current.clone();
    };
    for (key, value) in run {
        if start.get(&key) != Some(&value) {
            out.insert(key, value);
        }
    }
    serde_json::from_value(Value::Object(out)).unwrap_or_else(|_| current.clone())
}

impl World {
    /// A working copy of the session's state: its documents, each with an undo history of its own
    /// that starts empty, and an empty journal.
    fn working(s: &Session) -> Self {
        let mut docs = s.docs.clone();
        for st in &mut docs {
            st.history.undo.clear();
            st.history.redo.clear();
        }
        Self {
            docs,
            active: s.active,
            paint: s.paint.clone(),
            fill_active: s.fill_active,
            draw_mode: s.draw_mode,
            draw_inside: s.draw_inside,
            clipboard: s.clipboard.clone(),
            untitled_counter: s.untitled_counter,
            menu: s.menu.clone(),
            last_solid: s.last_solid,
            last_gradient: s.last_gradient.clone(),
            recent_colors: s.recent_colors.clone(),
            pending_paint: s.pending_paint.clone(),
            choices: Choices::of(s),
            journal: Journal::default(),
            start_prefs: s.prefs.clone(),
            start_caps_lock: s.caps_lock,
        }
    }

    /// Exchanges this state with the session's.
    fn swap(&mut self, s: &mut Session) {
        use std::mem::swap;
        swap(&mut self.docs, &mut s.docs);
        swap(&mut self.active, &mut s.active);
        swap(&mut self.paint, &mut s.paint);
        swap(&mut self.fill_active, &mut s.fill_active);
        swap(&mut self.draw_mode, &mut s.draw_mode);
        swap(&mut self.draw_inside, &mut s.draw_inside);
        swap(&mut self.clipboard, &mut s.clipboard);
        swap(&mut self.untitled_counter, &mut s.untitled_counter);
        swap(&mut self.menu, &mut s.menu);
        swap(&mut self.last_solid, &mut s.last_solid);
        swap(&mut self.last_gradient, &mut s.last_gradient);
        swap(&mut self.recent_colors, &mut s.recent_colors);
        swap(&mut self.pending_paint, &mut s.pending_paint);
        self.choices.swap(s);
        swap(&mut self.journal, &mut s.journal);
    }
}

/// The commands that can change the clipboard and still fail, and that do not put it back
/// themselves. The clipboard can hold a great deal, so a savepoint copies it only for these.
const CAN_FAIL_AFTER_COPYING: &[&str] = &["action.batch", "edit.cut"];

/// The commands that remove a document's undo history, or the document: a savepoint keeps a whole
/// copy of the histories for these. Any other command that removes or changes a step under the
/// newest undo or redo step and then fails ends the run ([`HISTORY_LOST`]).
const DROP_HISTORY: &[&str] = &["file.close", "file.closeAll", "document.close", "file.revert"];

/// A document's state before a command: everything but its undo history, which `mark` gives back,
/// or `whole` for the commands in [`DROP_HISTORY`].
struct SavedDoc {
    state: DocState,
    mark: crate::history::HistoryMark,
    whole: Option<crate::History>,
}

/// What one command may need to put back when it fails inside a run: the state of [`World`] the
/// command can reach.
pub(crate) struct Savepoint {
    docs: Vec<SavedDoc>,
    active: Option<usize>,
    paint: PaintDefaults,
    fill_active: bool,
    draw_mode: DrawMode,
    draw_inside: Option<NodeId>,
    clipboard: Option<Clipboard>,
    untitled_counter: u32,
    menu: cmd::menucmds::MenuState,
    last_solid: crate::Color,
    last_gradient: crate::GradientPaint,
    recent_colors: Vec<crate::Color>,
    pending_paint: Option<crate::Paint>,
    choices: Choices,
    journal: usize,
}

impl Savepoint {
    /// The state before command `id`. Until [`Savepoint::keep`] or [`Savepoint::restore`], the
    /// documents' undo histories let no step go.
    pub(crate) fn take(s: &mut Session, id: &str) -> Self {
        let whole = DROP_HISTORY.contains(&id);
        let docs = s
            .docs
            .iter_mut()
            .map(|st| {
                let history = std::mem::take(&mut st.history);
                let state = st.clone();
                st.history = history;
                let whole = whole.then(|| st.history.clone());
                SavedDoc { state, mark: st.history.mark(), whole }
            })
            .collect();
        Self {
            docs,
            active: s.active,
            paint: s.paint.clone(),
            fill_active: s.fill_active,
            draw_mode: s.draw_mode,
            draw_inside: s.draw_inside,
            clipboard: CAN_FAIL_AFTER_COPYING.contains(&id).then(|| s.clipboard.clone()),
            untitled_counter: s.untitled_counter,
            menu: s.menu.clone(),
            last_solid: s.last_solid,
            last_gradient: s.last_gradient.clone(),
            recent_colors: s.recent_colors.clone(),
            pending_paint: s.pending_paint.clone(),
            choices: Choices::of(s),
            journal: s.journal.len(),
        }
    }

    /// Keeps what the command did: the histories' limits apply again.
    pub(crate) fn keep(self, s: &mut Session) {
        for saved in &self.docs {
            if let Some(st) = s.docs.iter_mut().find(|d| d.uid == saved.state.uid) {
                st.history.release(&saved.mark);
            }
        }
    }

    /// Puts back the state before the command. → `false` when an undo history could not be given
    /// back as it was (a step under the newest one was removed or changed in place, or its
    /// document was closed); the rest is put back all the same.
    pub(crate) fn restore(self, s: &mut Session) -> bool {
        let mut exact = true;
        let mut docs = Vec::with_capacity(self.docs.len());
        for SavedDoc { mut state, mark, whole } in self.docs {
            let now = s.docs.iter_mut().find(|d| d.uid == state.uid).map(|d| std::mem::take(&mut d.history));
            state.history = match (whole, now) {
                (Some(history), _) => history,
                (None, Some(mut history)) => {
                    exact &= history.put_back(mark);
                    history
                }
                (None, None) => {
                    exact = false;
                    let mut history = crate::History::default();
                    history.put_back(mark);
                    history
                }
            };
            docs.push(state);
        }
        s.docs = docs;
        s.active = self.active;
        s.paint = self.paint;
        s.fill_active = self.fill_active;
        s.draw_mode = self.draw_mode;
        s.draw_inside = self.draw_inside;
        if let Some(clipboard) = self.clipboard {
            s.clipboard = clipboard;
        }
        s.untitled_counter = self.untitled_counter;
        s.menu = self.menu;
        s.last_solid = self.last_solid;
        s.last_gradient = self.last_gradient;
        s.recent_colors = self.recent_colors;
        s.pending_paint = self.pending_paint;
        self.choices.put(s);
        s.journal.truncate(self.journal);
        s.sync_front_profiles();
        exact
    }
}

/// The counters and the session a script's host functions share. Between slices `session` holds a
/// spare session; during a slice it holds the real one, because the interpreter cannot be given a
/// borrow of it.
struct Host {
    session: Rc<RefCell<Session>>,
    pause: Rc<Cell<bool>>,
    /// The run is sliced by frames, so the person at the computer can be asked to continue.
    framed: bool,
    calls: Cell<u64>,
    calls_this_slice: Cell<u32>,
    objects: Cell<u64>,
    bytes_kept: Cell<u64>,
    bridge: Cell<u64>,
    allowance_objects: Cell<u64>,
    allowance_bytes: Cell<u64>,
    continues: Cell<u32>,
    /// Past the growth allowance, waiting for the person's answer.
    waiting: Cell<bool>,
    log: RefCell<Vec<Value>>,
    log_bytes: Cell<usize>,
    written: RefCell<Vec<String>>,
    /// The run's number in its session, and the script's name, which its console lines carry.
    run: u64,
    name: Arc<str>,
    /// The run's own output and error, for its report.
    console: RefCell<console::Console>,
    /// The line `$.write` has not ended yet.
    pending: RefCell<console::Pending>,
    counts: Counts,
    /// Each command `vc.run` ran: its calls, its total time and its longest call, in nanoseconds.
    commands: RefCell<HashMap<&'static str, (u64, u128, u128)>>,
    /// The time the run's slices took, in nanoseconds.
    run_nanos: Cell<u128>,
    /// `alert`, `confirm` and `prompt`: their answers and the list the report gives.
    dialogs: dialogs::Dialogs,
    /// The files the run reads: its includes and `$.evalFile`'s.
    files: RefCell<include::Files>,
    /// What `File` and `Folder` have open and have read.
    disk: RefCell<files::Disk>,
    /// Where the lines of the script's expanded source came from.
    script_map: RefCell<include::SourceMap>,
    /// The same for each source `$.evalFile` gave, in order.
    eval_maps: RefCell<Vec<include::SourceMap>>,
    /// The file, line and column of each error `$.evalFile` raised for a file it could not run, in
    /// order: the error's mark ([`HostError::Marked`]) is its index.
    eval_failures: RefCell<Vec<(Arc<str>, u32, u32)>>,
    /// The realm's tag for the script ([`Realm::script_tag`]) once the run has started it, and
    /// [`vectorcraft_js::UNKNOWN_SOURCE`] before or when the script did not compile.
    script_tag: Cell<u32>,
    /// The run's name as its report gives it, which [`Host::name`] shortens for console lines.
    report_name: Arc<str>,
    /// The source maps the named engine of the run kept from its earlier runs; `None` for a run
    /// without one.
    kept_maps: Option<Rc<RefCell<KeptMaps>>>,
    /// When the run started, where the platform has a clock.
    started: Option<std::time::Instant>,
    /// The time since [`Host::started`] at which a `$.sleep` ends.
    wake_at: Cell<Option<std::time::Duration>>,
    /// Set while a host function runs where the run can stop at the call ([`Realm::wait_flag`]).
    can_wait: Rc<Cell<bool>>,
    /// What the last `#strict` line of the file `$.evalFile` last gave set (`None` without one).
    eval_strict: Cell<Option<bool>>,
    /// The values of `$` that belong to the run.
    dollar: dollar::RunState,
    /// The run's `args`, which `vc.args` gives, and their size as `vc.args` charges it.
    args: RefCell<(Data, u64)>,
}

/// A run's host calls by kind, as its report gives them in `hostCallCounts`.
#[derive(Default)]
struct Counts {
    vc_run: Cell<u64>,
    document_reads: Cell<u64>,
    command_list_reads: Cell<u64>,
    vc_log: Cell<u64>,
    console_writes: Cell<u64>,
    /// Calls of `File` and `Folder` that reach the host ([`files`]).
    file_calls: Cell<u64>,
    /// `execute` calls, which open nothing.
    file_executes: Cell<u64>,
}

impl Counts {
    fn json(&self) -> Value {
        json!({
            "vcRun": self.vc_run.get(),
            "documentReads": self.document_reads.get(),
            "commandListReads": self.command_list_reads.get(),
            "vcLog": self.vc_log.get(),
            "consoleWrites": self.console_writes.get(),
            "fileCalls": self.file_calls.get(),
            "fileExecutes": self.file_executes.get(),
        })
    }
}

fn bump(c: &Cell<u64>) {
    c.set(c.get().saturating_add(1));
}

/// Now, where the platform has a clock (not in a wasm build).
fn clock() -> Option<std::time::Instant> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(std::time::Instant::now())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// Whole milliseconds, rounded down.
fn millis(nanos: u128) -> u64 {
    u64::try_from(nanos / 1_000_000).unwrap_or(u64::MAX)
}

/// The [`SLOWEST_COMMANDS`] commands of `times` that took longest in all, longest first and then by
/// id.
pub(crate) fn slowest(times: &HashMap<&'static str, (u64, u128, u128)>) -> Vec<console::Slow> {
    let mut all: Vec<(&&str, &(u64, u128, u128))> = times.iter().collect();
    all.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| a.0.cmp(b.0)));
    all.into_iter()
        .take(SLOWEST_COMMANDS)
        .map(|(id, &(calls, total, max))| console::Slow { id: (*id).to_string(), calls, total_ms: millis(total), max_ms: millis(max) })
        .collect()
}

impl Host {
    /// Gives the named engine of the run the source maps of the code the run compiled: its script
    /// and each file `$.evalFile` gave. A run that never started its script compiled nothing.
    fn keep_maps(&self) {
        let (Some(kept), tag) = (&self.kept_maps, self.script_tag.get()) else { return };
        if tag == vectorcraft_js::UNKNOWN_SOURCE {
            return;
        }
        let mut kept = kept.borrow_mut();
        kept.keep(tag, std::mem::take(&mut *self.script_map.borrow_mut()), Some(self.report_name.clone()));
        for (n, map) in std::mem::take(&mut *self.eval_maps.borrow_mut()).into_iter().enumerate() {
            match u32::try_from(n + 1).ok().and_then(|n| tag.checked_add(n)).filter(|t| *t != vectorcraft_js::UNKNOWN_SOURCE) {
                Some(t) => kept.keep(t, map, None),
                None => break,
            }
        }
    }

    /// The file and line that line `line` of the source with tag `tag` came from, as `$.line` and
    /// `$.stack` give them: the run's script, a file it included or a file `$.evalFile` gave, in
    /// this run or in an earlier run of its named engine, with the run's name standing for a script
    /// given as text. Code that `eval` or `Function` compiled has no map and keeps its own line,
    /// named `[eval]`; code whose map is gone, such as one the named engine let go to stay within
    /// [`KEPT_MAP_BYTES`], is at line 0 of `[unknown]`.
    fn source_place(&self, tag: u32, line: u32) -> (String, u32) {
        if tag == vectorcraft_js::UNKNOWN_SOURCE {
            return ("[eval]".to_string(), line);
        }
        let named = |map: &include::SourceMap, script: Option<&Arc<str>>| {
            let (index, file, line, _) = map.locate(line, 1);
            match script {
                Some(name) if index == 0 && file.is_empty() => (name.to_string(), line),
                _ => (file.to_string(), line),
            }
        };
        let script = self.script_tag.get();
        let placed = match tag.checked_sub(script).filter(|_| script != vectorcraft_js::UNKNOWN_SOURCE) {
            Some(0) => self.script_map.try_borrow().ok().map(|m| named(&m, Some(&self.report_name))),
            Some(n) => self.eval_maps.try_borrow().ok().and_then(|maps| maps.get(n as usize - 1).map(|m| named(m, None))),
            None => {
                self.kept_maps.as_ref().and_then(|k| k.try_borrow().ok()).and_then(|k| k.get(tag).map(|kept| named(&kept.map, kept.script.as_ref())))
            }
        };
        placed.unwrap_or_else(|| ("[unknown]".to_string(), 0))
    }

    /// `$.fileName` read by the code at `site`: the file that code is in, by the source map of the
    /// source it came from (this run's script, a file `$.evalFile` gave, or code an earlier run of
    /// the named engine compiled), so a function keeps the file it was defined in. Code from a file
    /// `#include` inserted is in that file. A script given as text has no name, and neither has
    /// code whose map the named engine no longer keeps. Code that `eval` or `Function` compiled has
    /// no map, and is given the file being run now.
    fn file_name_at(&self, site: vectorcraft_js::CallSite) -> String {
        let name = |m: &include::SourceMap| m.locate(site.line, site.column).1.to_string();
        let tag = site.source;
        if tag == vectorcraft_js::UNKNOWN_SOURCE {
            return self.files.borrow().file_name();
        }
        let script = self.script_tag.get();
        match tag.checked_sub(script).filter(|_| script != vectorcraft_js::UNKNOWN_SOURCE) {
            Some(0) => name(&self.script_map.borrow()),
            Some(n) => self.eval_maps.borrow().get(n as usize - 1).map(name).unwrap_or_default(),
            None => {
                let kept = self.kept_maps.as_ref().map(|k| k.borrow());
                kept.as_ref().and_then(|k| k.get(tag)).map(|k| name(&k.map)).unwrap_or_default()
            }
        }
    }

    /// Adds a line to the run's console and, during a slice, to the session's.
    fn line(&self, entry: console::Entry, cut: bool) {
        let line = console::ConsoleLine { run: self.run, name: self.name.clone(), entry };
        if let Ok(mut s) = self.session.try_borrow_mut() {
            s.script_console.push(line.clone(), cut);
        }
        self.console.borrow_mut().push(line, cut);
    }

    /// `$.write` and `$.writeln`: `text`, then with `end` a line break.
    fn write(&self, text: &str, end: bool) {
        let mut pending = self.pending.borrow_mut();
        pending.write(text, |line, cut| self.line(console::Entry::Output(line), cut));
        if end {
            pending.end_line(|line, cut| self.line(console::Entry::Output(line), cut));
        }
    }

    /// The time since the run started; none without a clock.
    fn elapsed(&self) -> Option<std::time::Duration> {
        #[cfg(test)]
        if TEST_NO_CLOCK.with(Cell::get) {
            return None;
        }
        #[cfg(test)]
        if let Some(d) = TEST_ELAPSED.with(Cell::get) {
            return Some(d);
        }
        self.started.map(|s| s.elapsed())
    }

    /// `$.sleep`: ends the slice, and the run does not go on until `d` has passed. Without a clock
    /// it does not wait. A getter, a conversion, a built-in's callback or a file `$.evalFile` runs
    /// cannot stop at the call, so a wait there is an error, with or without a clock.
    fn sleep(&self, d: std::time::Duration) -> std::result::Result<(), HostError> {
        if d.is_zero() {
            return Ok(());
        }
        if !self.can_wait.get() {
            return Err(HostError::Error(
                "$.sleep: a run cannot wait inside a callback of a built-in function, a getter, a conversion or a file $.evalFile runs; call it from the script's own code"
                    .into(),
            ));
        }
        if let Some(now) = self.elapsed() {
            self.wake_at.set(Some(now.saturating_add(d)));
            self.pause.set(true);
        }
        Ok(())
    }

    /// The time left of a `$.sleep`, or none when the run is not asleep.
    fn asleep(&self) -> Option<std::time::Duration> {
        let wake = self.wake_at.get()?;
        let left = self.elapsed().map_or(std::time::Duration::ZERO, |now| wake.saturating_sub(now));
        if left.is_zero() {
            self.wake_at.set(None);
            return None;
        }
        Some(left)
    }

    fn over(&self, limit: &str) -> HostError {
        HostError::Abort(limit.to_string())
    }

    /// `error`, marked as an error at `file`, `line` and `column`, the place [`located`] gives when
    /// the run ends with it uncaught.
    fn placed(&self, error: HostError, file: Arc<str>, line: u32, column: u32) -> HostError {
        let mut places = self.eval_failures.borrow_mut();
        let Ok(mark) = u32::try_from(places.len()) else { return error };
        places.push((file, line, column));
        HostError::Marked { error: Box::new(error), mark }
    }

    fn charge_call(&self, bytes: u64) -> std::result::Result<(), HostError> {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        if calls > MAX_HOST_CALLS {
            return Err(self.over(OVER_CALLS));
        }
        let bridge = self.bridge.get().saturating_add(bytes);
        self.bridge.set(bridge);
        if bridge > MAX_BRIDGE_BYTES {
            return Err(self.over(OVER_BRIDGE));
        }
        let n = self.calls_this_slice.get() + 1;
        self.calls_this_slice.set(n);
        if n >= SLICE_HOST_CALLS {
            self.pause.set(true);
        }
        Ok(())
    }

    /// Counts what a command added to the documents, past which the person is asked or, without
    /// anyone to ask, the run ends. A run that goes on past twice its allowance within one slice
    /// ends too.
    fn charge_growth(&self, objects: u64, bytes: u64) -> std::result::Result<(), HostError> {
        self.objects.set(self.objects.get().saturating_add(objects));
        self.bytes_kept.set(self.bytes_kept.get().saturating_add(bytes));
        let (o, b) = (self.objects.get(), self.bytes_kept.get());
        let (limit_o, limit_b) = (self.allowance_objects.get(), self.allowance_bytes.get());
        if o <= limit_o && b <= limit_b {
            return Ok(());
        }
        let may_ask = self.framed && self.continues.get() < MAX_CONTINUES;
        if !may_ask || o > limit_o.saturating_mul(2) || b > limit_b.saturating_mul(2) {
            return Err(self.over(OVER_GROWTH));
        }
        self.waiting.set(true);
        self.pause.set(true);
        Ok(())
    }
}

/// A run in progress.
struct Job {
    realm: Realm,
    host: Rc<Host>,
    /// The link the realm's host functions find `host` through.
    link: Link,
    /// The named engine the realm belongs to, which gets it back when the run ends.
    engine: Option<String>,
    /// Between slices the working copy; during the last slice's commit the state before the run.
    world: World,
    /// The documents open when the run began, by [`DocState::uid`].
    start_uids: Vec<u64>,
    name: String,
    /// The file-access scope `script.run` came in, which every slice runs in.
    scope: crate::file_access::ScopeHandle,
}

/// The `Data` for a JSON value, within `limits`. When it does not fit, the error is the place in
/// `v` where it stopped fitting, such as `.layers[2].name`, and empty for `v` itself.
pub(crate) fn to_data(v: &Value, limits: &DataLimits, depth: usize, used: &mut (usize, usize)) -> std::result::Result<Data, String> {
    if depth > limits.depth {
        return Err(String::new());
    }
    used.0 = used.0.saturating_add(16);
    used.1 = used.1.saturating_add(1);
    if used.0 > limits.bytes || used.1 > limits.entries {
        return Err(String::new());
    }
    Ok(match v {
        Value::Null => Data::Null,
        Value::Bool(b) => Data::Bool(*b),
        Value::Number(n) => Data::Number(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => {
            used.0 = used.0.saturating_add(s.len());
            if used.0 > limits.bytes {
                return Err(String::new());
            }
            Data::String(s.clone())
        }
        Value::Array(a) => Data::Array(
            a.iter()
                .enumerate()
                .map(|(i, x)| to_data(x, limits, depth + 1, used).map_err(|place| format!("[{i}]{place}")))
                .collect::<std::result::Result<_, _>>()?,
        ),
        Value::Object(o) => Data::Object(
            o.iter()
                .map(|(k, x)| {
                    used.0 = used.0.saturating_add(k.len());
                    to_data(x, limits, depth + 1, used).map(|d| (k.clone(), d)).map_err(|place| format!(".{k}{place}"))
                })
                .collect::<std::result::Result<_, _>>()?,
        ),
    })
}

/// The JSON for a value a script gave; a whole number within a double's exact range is an integer,
/// which is how commands read ids and counts.
fn to_json(d: &Data) -> Value {
    match d {
        Data::Undefined | Data::Null => Value::Null,
        Data::Bool(b) => Value::Bool(*b),
        Data::Number(n) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            if n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_992.0 {
                json!(*n as i64)
            } else {
                serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number)
            }
        }
        Data::String(s) => Value::String(s.clone()),
        Data::Array(a) => Value::Array(a.iter().map(to_json).collect()),
        Data::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}

/// A step into a JSON value: a key or an index.
enum Step<'a> {
    Name(&'a str),
    Index(usize),
}

/// The place in `v` that is more than [`DataLimits`]' depth levels deep, such as `.layers[2].name`,
/// or `None`. It walks `v` with a list of its own, so any depth fits on the stack.
pub(crate) fn too_deep(v: &Value) -> Option<String> {
    let limit = DataLimits::default().depth;
    // `steps[i - 1]` is the entry that holds entry `i` and the step into it; entry 0 is `v`.
    let mut steps: Vec<(usize, Step<'_>)> = Vec::new();
    let mut todo = vec![(v, 0usize, 0usize)];
    while let Some((v, depth, entry)) = todo.pop() {
        if depth > limit {
            let mut place = Vec::new();
            let mut at = entry;
            while let Some((parent, step)) = at.checked_sub(1).and_then(|i| steps.get(i)) {
                place.push(match step {
                    Step::Name(k) => format!(".{k}"),
                    Step::Index(i) => format!("[{i}]"),
                });
                at = *parent;
            }
            place.reverse();
            return Some(place.concat());
        }
        let children: Box<dyn Iterator<Item = (Step<'_>, &Value)>> = match v {
            Value::Array(a) => Box::new(a.iter().enumerate().map(|(i, x)| (Step::Index(i), x))),
            Value::Object(o) => Box::new(o.iter().map(|(k, x)| (Step::Name(k.as_str()), x))),
            _ => continue,
        };
        for (step, child) in children {
            steps.push((entry, step));
            todo.push((child, depth + 1, steps.len()));
        }
    }
    None
}

/// Estimated bytes of `d`.
fn data_size(d: &Data) -> u64 {
    match d {
        Data::Undefined | Data::Null | Data::Bool(_) => 4,
        Data::Number(_) => 8,
        Data::String(s) => s.len() as u64 + 2,
        Data::Array(a) => 8 + a.iter().map(data_size).sum::<u64>(),
        Data::Object(o) => 8 + o.iter().map(|(k, v)| k.len() as u64 + 2 + data_size(v)).sum::<u64>(),
    }
}

/// Brings each open document's next id up to the highest one the session has given out for it.
fn raise_ids(s: &mut Session) {
    let marks = std::mem::take(&mut s.script_high_water);
    for st in &mut s.docs {
        if let Some(&mark) = marks.get(&st.uid)
            && mark > st.doc.peek_next_id()
        {
            st.change_unsaved(|d| Arc::make_mut(d).raise_next_id(mark));
        }
    }
    s.script_high_water = marks;
}

/// Every object in `d`: all its art ([`crate::Document::art_roots`]), opacity masks included.
fn all_objects(d: &crate::Document) -> u64 {
    d.art_roots().map(|n| n.count_with_masks() as u64).sum()
}

fn note_ids(s: &mut Session) {
    for st in &s.docs {
        let mark = s.script_high_water.entry(st.uid).or_insert(0);
        *mark = (*mark).max(st.doc.peek_next_id());
    }
}

/// Runs command `id` inside a slice. A command that fails puts everything back it had done.
/// → its result, the objects it added, the bytes of document data it added and the state before
/// it, for a result that cannot cross to the script.
fn step(s: &mut Session, host: &Host, id: &str, params: &Value) -> Result<(Value, u64, u64, Option<Savepoint>)> {
    raise_ids(s);
    let mutating = waits_for_tool_edit(id);
    let save = mutating.then(|| Savepoint::take(s, id));
    let before: Vec<(u64, Arc<crate::Document>, u64)> =
        if mutating { s.docs.iter().map(|d| (d.uid, d.doc.clone(), d.doc.peek_next_id())).collect() } else { Vec::new() };
    crate::file_access::start_recording_writes();
    let r = s.execute(id, params);
    // A file a command wrote before it failed is on disk all the same.
    let mut written = host.written.borrow_mut();
    for path in crate::file_access::finish_recording_writes(r.is_ok()) {
        if !written.contains(&path) {
            written.push(path);
        }
    }
    drop(written);
    match r {
        Ok(v) => {
            note_ids(s);
            let (mut objects, mut bytes) = (0, 0);
            for st in &s.docs {
                match before.iter().find(|(uid, ..)| *uid == st.uid) {
                    Some((_, d, ids)) if !Arc::ptr_eq(d, &st.doc) => {
                        objects += st.doc.peek_next_id().saturating_sub(*ids);
                        bytes += d.retained_bytes_each(&st.doc).1 as u64;
                    }
                    Some(_) => {}
                    // A document the command opened or made counts whole.
                    None if mutating => {
                        objects += all_objects(&st.doc);
                        bytes += crate::Document::new(1.0, 1.0).retained_bytes_each(&st.doc).1 as u64;
                    }
                    None => {}
                }
            }
            Ok((v, objects, bytes, save))
        }
        Err(e) => {
            if let Some(save) = save
                && !save.restore(s)
            {
                return Err(EngineError::Internal { cmd: id.to_string(), msg: HISTORY_LOST.to_string() });
            }
            Err(e)
        }
    }
}

fn vc_run(host: &Rc<Host>, args: Vec<Data>) -> std::result::Result<Data, HostError> {
    let Some(Data::String(id)) = args.first() else { return Err(HostError::Type(NO_COMMAND_ID.to_string())) };
    if args.len() > 2 {
        return Err(HostError::Type(TOO_MANY_ARGUMENTS.to_string()));
    }
    let id = id.clone();
    let params = match args.get(1) {
        None | Some(Data::Undefined) => json!({}),
        Some(d @ Data::Object(_)) => to_json(d),
        Some(_) => return Err(HostError::Type(PARAMS_NOT_OBJECT.to_string())),
    };
    let bytes = args.iter().map(data_size).sum();
    host.charge_call(bytes)?;
    bump(&host.counts.vc_run);
    let session = host.session.clone();
    let started = clock();
    let r = {
        let mut s = session.borrow_mut();
        step(&mut s, host, &id, &params)
    };
    // Only commands that exist are timed, so the table holds at most one row per command.
    if let (Some(started), Some(spec)) = (started, crate::find_command(&id)) {
        let nanos = started.elapsed().as_nanos();
        let mut times = host.commands.borrow_mut();
        let row = times.entry(spec.id).or_insert((0, 0, 0));
        *row = (row.0 + 1, row.1.saturating_add(nanos), row.2.max(nanos));
    }
    let (value, objects, added, save) = match r {
        Ok(done) => done,
        // A bug is not something a script can handle, and neither is a request to stop.
        Err(e @ EngineError::Internal { .. }) => return Err(HostError::Abort(e.to_string())),
        Err(EngineError::Cancelled) => return Err(HostError::Abort("canceled".into())),
        Err(e) => return Err(HostError::Error(e.to_string())),
    };
    let crossed = host.charge_growth(objects, added).and_then(|()| {
        let data = to_data(&value, &result_limits(), 0, &mut (0, 0))
            .map_err(|place| HostError::Range(format!("vc.run: the result{place} is too large for a script")))?;
        host.charge_call(data_size(&data))?;
        Ok(data)
    });
    // A command whose result does not reach the script is put back, like one that failed.
    if let Some(save) = save {
        let mut s = session.borrow_mut();
        if crossed.is_ok() {
            save.keep(&mut s);
        } else if !save.restore(&mut s) {
            return Err(HostError::Abort(HISTORY_LOST.to_string()));
        }
    }
    crossed
}

/// A bounded snapshot of the active document, or `null` with none open. A snapshot past the limits
/// of a crossing throws a `RangeError`.
fn vc_document(host: &Host) -> std::result::Result<Data, HostError> {
    let s = host.session.borrow();
    let Some(st) = s.active() else { return Ok(Data::Null) };
    let ids: Vec<u64> = st.selection.objects.iter().take(1_000).map(|i| i.0).collect();
    let v = json!({
        "name": st.title(),
        "path": st.path,
        "modified": st.is_dirty(),
        "artboards": st.doc.artboards.len(),
        "layers": st.doc.layers.len(),
        "selection": ids,
        "selectionCount": st.selection.objects.len(),
    });
    to_data(&v, &DataLimits::default(), 0, &mut (0, 0)).map_err(|place| HostError::Range(format!("vc.document{place} is too large for a script")))
}

/// Every command a script may run: its id, label and parameters.
fn vc_commands() -> Data {
    Data::Array(
        command_specs()
            .iter()
            .filter(|c| !refused_to_scripts(c.id))
            .map(|c| {
                Data::Object(vec![
                    ("id".into(), Data::String(c.id.into())),
                    ("label".into(), Data::String(c.label.into())),
                    ("params".into(), Data::String(c.params.into())),
                ])
            })
            .collect(),
    )
}

fn vc_log(host: &Host, args: Vec<Data>) -> std::result::Result<Data, HostError> {
    let entry = args.first().unwrap_or(&Data::Undefined);
    let bytes = data_size(entry) as usize;
    if host.log.borrow().len() >= MAX_LOG_LINES || host.log_bytes.get().saturating_add(bytes) > MAX_LOG_BYTES {
        return Err(HostError::Range(LOG_FULL.to_string()));
    }
    host.log_bytes.set(host.log_bytes.get() + bytes);
    let value = to_json(entry);
    let text = match &value {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    host.log.borrow_mut().push(value);
    each_line(&text, |line, cut| host.line(console::Entry::Log(line), cut));
    Ok(Data::Undefined)
}

/// Calls `done` with each console line of `text` and whether that line was cut, one at a time.
fn each_line(text: &str, mut done: impl FnMut(String, bool)) {
    let mut pending = console::Pending::default();
    pending.write(text, &mut done);
    pending.end_line(done);
}

/// The `$` object: `$.write(...values)` writes the values to the Script Console as `String(value)`
/// gives them, one after another, and `$.writeln(...values)` does the same and ends the line.
/// `$.evalFile(path[, timeout])` runs a file and gives its last value, `$.fileName` is the file the
/// code reading it is in ([`Host::file_name_at`]) and `$.includePath` the `#includepath` list. Its
/// other members are [`dollar::members`].
fn dollar_object(link: &Link, info: dollar::RunInfo, localization: &Rc<vectorcraft_js::Localization>) -> Vec<(String, HostItem)> {
    let writer = |end: bool| {
        let l = link.clone();
        HostItem::TextFunction(function(move |a| {
            let h = current(&l)?;
            let text: String = a.iter().map(|d| if let Data::String(s) = d { s.as_str() } else { "" }).collect();
            h.charge_call(text.len() as u64 + 2)?;
            bump(&h.counts.console_writes);
            h.write(&text, end);
            Ok(Data::Undefined)
        }))
    };
    let getter = |read: fn(&include::Files) -> String| {
        let l = link.clone();
        HostItem::Getter(function(move |_| {
            let h = current(&l)?;
            let text = read(&h.files.borrow());
            h.charge_call(text.len() as u64 + 2)?;
            Ok(Data::String(text))
        }))
    };
    let file_name = {
        let l = link.clone();
        HostItem::SiteGetter(Rc::new(move |site| {
            let h = current(&l)?;
            let text = h.file_name_at(site);
            h.charge_call(text.len() as u64 + 2)?;
            Ok(Data::String(text))
        }))
    };
    let eval_file = {
        let (l, end, strict) = (link.clone(), link.clone(), link.clone());
        HostItem::Evaluator {
            source: function(move |a| eval_file(&*current(&l)?, a)),
            end: Rc::new(move || {
                if let Ok(h) = current(&end) {
                    h.files.borrow_mut().end_eval();
                }
            }),
            strict_writes: Rc::new(move || current(&strict).ok().and_then(|h| h.eval_strict.get())),
        }
    };
    let mut items = vec![
        ("write".into(), writer(false)),
        ("writeln".into(), writer(true)),
        ("evalFile".into(), eval_file),
        ("fileName".into(), file_name),
        ("includePath".into(), getter(include::Files::include_path)),
    ];
    items.extend(dollar::members(link, info, localization));
    items
}

/// `$.evalFile(path[, timeout])`: the source of the file `path` names, relative to the file being
/// run, with its preprocessor lines read, which the interpreter then runs. `timeout`, in
/// milliseconds, is taken and not waited on: the file is read at once.
fn eval_file(host: &Host, args: Vec<Data>) -> std::result::Result<Data, HostError> {
    let Some(Data::String(path)) = args.first().filter(|p| !matches!(p, Data::String(s) if s.is_empty())) else {
        return Err(HostError::Type(EVAL_NO_PATH.to_string()));
    };
    match args.get(1) {
        None | Some(Data::Undefined) => {}
        Some(Data::Number(ms)) if ms.is_finite() && *ms >= 0.0 => {}
        Some(Data::Number(_)) => return Err(HostError::Range(EVAL_TIMEOUT.to_string())),
        Some(_) => return Err(HostError::Type(EVAL_TIMEOUT.to_string())),
    }
    if args.len() > 2 {
        return Err(HostError::Type(EVAL_ARGUMENTS.to_string()));
    }
    host.charge_call(path.len() as u64 + 2)?;
    let expanded = host.files.borrow_mut().eval_file(path).map_err(|f| {
        let message = if f.line > 0 { format!("{}:{}:{}: {}", f.file, f.line, f.column, f.message) } else { f.message };
        let error = match f.kind {
            include::FailKind::Limit => HostError::Abort(message),
            include::FailKind::Syntax => HostError::Syntax(message),
            include::FailKind::Error => HostError::Error(message),
        };
        if f.line > 0 { host.placed(error, f.file, f.line, f.column) } else { error }
    })?;
    // A file without preprocessor lines is parsed here first, so its syntax error names its place.
    let options = vectorcraft_js::Options { max_source_units: include::MAX_TOTAL_BYTES, ..vectorcraft_js::Options::default() };
    if let Err(e) = vectorcraft_js::parse(&expanded.source, &options) {
        host.files.borrow_mut().end_eval();
        let (_, file, line, column) = expanded.map.locate(e.line, e.column);
        let message = format!("{file}:{line}:{column}: {}", e.message);
        return Err(host.placed(HostError::Syntax(message), file, line, column));
    }
    host.eval_maps.borrow_mut().push(expanded.map);
    host.eval_strict.set(expanded.strict);
    Ok(Data::String(expanded.source))
}

fn function(f: impl Fn(Vec<Data>) -> std::result::Result<Data, HostError> + 'static) -> HostFunction {
    Rc::new(f)
}

/// The `vc` object, made once for each realm: its members act for the run `link` holds when they
/// are called.
fn vc_object(link: &Link) -> Vec<(String, HostItem)> {
    let args = {
        let l = link.clone();
        function(move |_| {
            let h = current(&l)?;
            let (args, size) = &*h.args.borrow();
            h.charge_call(*size)?;
            Ok(args.clone())
        })
    };
    let run = {
        let l = link.clone();
        function(move |a| vc_run(&current(&l)?, a))
    };
    let document = {
        let l = link.clone();
        function(move |_| {
            let h = current(&l)?;
            h.charge_call(64)?;
            bump(&h.counts.document_reads);
            vc_document(&h)
        })
    };
    let commands = {
        let l = link.clone();
        let list = Rc::new(vc_commands());
        let size = data_size(&list);
        function(move |_| {
            let h = current(&l)?;
            h.charge_call(size)?;
            bump(&h.counts.command_list_reads);
            Ok((*list).clone())
        })
    };
    let log = {
        let l = link.clone();
        function(move |a| {
            let h = current(&l)?;
            h.charge_call(data_size(a.first().unwrap_or(&Data::Undefined)))?;
            bump(&h.counts.vc_log);
            vc_log(&h, a)
        })
    };
    let has = function(|a| Ok(Data::Bool(matches!(a.first(), Some(Data::String(f)) if API_FEATURES.contains(&f.as_str())))));
    vec![
        ("run".into(), HostItem::Function(run)),
        ("document".into(), HostItem::Getter(document)),
        ("commands".into(), HostItem::Getter(commands)),
        ("log".into(), HostItem::Function(log)),
        ("args".into(), HostItem::FrozenGetter(args)),
        (
            "api".into(),
            HostItem::Object(vec![("version".into(), HostItem::Value(Data::String(API_VERSION.into()))), ("has".into(), HostItem::Function(has))]),
        ),
    ]
}

/// A plain-words kind of a run's failure, as the report gives it.
fn kind_id(kind: &RunErrorKind) -> &'static str {
    match kind {
        RunErrorKind::Syntax => "syntax",
        RunErrorKind::Thrown => "thrown",
        RunErrorKind::Limit(LimitKind::Interrupted) => "stopped",
        RunErrorKind::Limit(_) => "limit",
        RunErrorKind::Internal => "internal",
    }
}

/// `message`, with what to do about a failure that comes from a script written for another program.
fn explained(kind: &RunErrorKind, message: &str) -> String {
    const HOST_OBJECTS: &[&str] = &["app", "Window", "BridgeTalk", "activeDocument", "documents"];
    let names_host_object =
        message.strip_prefix("ReferenceError: ").is_some_and(|m| HOST_OBJECTS.iter().any(|o| m.split_whitespace().next() == Some(o)));
    if *kind == RunErrorKind::Thrown && names_host_object {
        format!("{message} (scripts here use the `vc` object; objects other applications give their scripts are not available)")
    } else {
        message.to_string()
    }
}

/// The error of a run that stopped in its preprocessor lines (`f`), as the realm gives its own.
fn preprocessor_error(f: include::Failure) -> (RunError, Option<Arc<str>>) {
    let (kind, message) = match f.kind {
        include::FailKind::Syntax if f.line > 0 => {
            (RunErrorKind::Syntax, format!("SyntaxError: {} (line {}, column {})", f.message, f.line, f.column))
        }
        include::FailKind::Syntax => (RunErrorKind::Syntax, format!("SyntaxError: {}", f.message)),
        include::FailKind::Error => (RunErrorKind::Thrown, format!("Error: {}", f.message)),
        include::FailKind::Limit => (RunErrorKind::Limit(LimitKind::Host), f.message),
    };
    let file = (f.index > 0).then_some(f.file);
    (RunError { kind, message, line: f.line, column: f.column }, file)
}

/// `e`, the error `realm` raised in the run of `host`, at the line and column of the file it is in,
/// with that file's name unless it is the run's own script.
fn located(host: &Host, realm: &Realm, e: &RunError) -> (RunError, Option<Arc<str>>) {
    let map = |m: &include::SourceMap| {
        let (index, file, line, column) = m.locate(e.line, e.column);
        let message = if e.kind == RunErrorKind::Syntax { relocated(&e.message, e.line, e.column, line, column) } else { e.message.clone() };
        (RunError { kind: e.kind.clone(), message, line, column }, index, file)
    };
    // An error `$.evalFile` raised for a file it could not run, and no script caught, is at that
    // file's place.
    if let Some((file, line, column)) = realm.error_mark().and_then(|m| host.eval_failures.borrow().get(m as usize).cloned()) {
        let message = if e.kind == RunErrorKind::Syntax { relocated(&e.message, e.line, e.column, line, column) } else { e.message.clone() };
        return (RunError { kind: e.kind.clone(), message, line, column }, Some(file));
    }
    if e.line == 0 {
        return (e.clone(), None);
    }
    // An error in code from a source whose map is not at hand, such as code that `eval` compiled,
    // is given without a place.
    let unplaced = || (RunError { kind: e.kind.clone(), message: e.message.clone(), line: 0, column: 0 }, None);
    match realm.error_source() {
        0 => {
            let (e, index, file) = map(&host.script_map.borrow());
            (e, (index > 0).then_some(file))
        }
        n if n != vectorcraft_js::UNKNOWN_SOURCE => match host.eval_maps.borrow().get(n as usize - 1) {
            Some(m) => {
                let (e, _, file) = map(m);
                (e, Some(file))
            }
            None => unplaced(),
        },
        // Code an earlier run of the named engine compiled, such as a function it defined, is at
        // its place in that run's files: the run's script, a file it included, or a file
        // `$.evalFile` gave. A script given as text is named by that run's name.
        _ => {
            let kept = host.kept_maps.as_ref().map(|k| k.borrow());
            match kept.as_ref().and_then(|k| k.get(realm.error_tag())) {
                Some(kept) => {
                    let (e, index, file) = map(&kept.map);
                    let file = match &kept.script {
                        Some(name) if index == 0 && file.is_empty() => name.clone(),
                        _ => file,
                    };
                    (e, Some(file))
                }
                None => unplaced(),
            }
        }
    }
}

/// A syntax error's `message`, which ends with its place `(line L, column C)`, with the place
/// given as `line`, `column`.
fn relocated(message: &str, old_line: u32, old_column: u32, line: u32, column: u32) -> String {
    let old = format!(" (line {old_line}, column {old_column})");
    match message.strip_suffix(&old) {
        Some(m) => format!("{m} (line {line}, column {column})"),
        None => message.to_string(),
    }
}

impl Session {
    /// Is a script running? While one is, the commands that change anything are refused.
    pub fn script_running(&self) -> bool {
        self.script_key.is_some()
    }

    /// The named engine of the running script.
    fn running_engine(&self) -> Option<String> {
        self.script_key.as_ref().and_then(|k| with_job(k, |j| j.engine.clone())).flatten()
    }

    /// The named engines (`#targetengine`) this session keeps, in the order they were made: each
    /// one's name, its heap after its last run in bytes, and whether a run has it now.
    pub fn script_engines(&self) -> Vec<(String, usize, bool)> {
        with_engines(&self.script_engines, |engines| engines.iter().map(|e| (e.name.clone(), e.heap, e.realm.is_none())).collect())
    }

    /// Drops the named engine `name` and the globals its runs left; a later run that names it gets a
    /// new one.
    pub fn drop_script_engine(&mut self, name: &str) -> Result<()> {
        if name == MAIN_ENGINE {
            return Err(EngineError::Other(DROP_MAIN.to_string()));
        }
        if self.running_engine().as_deref() == Some(name) {
            return Err(EngineError::Busy);
        }
        let gone = with_engines(&self.script_engines, |engines| {
            let at = engines.iter().position(|e| e.name == name)?;
            Some(engines.remove(at))
        });
        match gone {
            Some(engine) => {
                drop(engine);
                Ok(())
            }
            None => Err(EngineError::Other(format!("there is no script engine named {name:?}"))),
        }
    }

    /// The realm of the named engine `name` for a run, with the link its host functions use and the
    /// source maps it kept: the realm its last run left, or `None` for an engine made now. → the
    /// kind and text of the failure when the engine cannot be had.
    fn take_engine(&mut self, name: &str) -> std::result::Result<TakenEngine, (include::FailKind, String)> {
        if cfg!(target_arch = "wasm32") {
            return Err((include::FailKind::Error, NO_ENGINES_ON_WEB.to_string()));
        }
        with_engines(&self.script_engines, |engines| {
            if let Some(e) = engines.iter_mut().find(|e| e.name == name) {
                let realm = e.realm.take().ok_or((include::FailKind::Error, ENGINE_BUSY.to_string()))?;
                return Ok((Some(realm), e.link.clone(), e.maps.clone()));
            }
            if engines.len() >= MAX_ENGINES {
                return Err((
                    include::FailKind::Limit,
                    format!("a session keeps at most {MAX_ENGINES} script engines; drop one with script.engine.drop"),
                ));
            }
            let link: Link = Rc::new(RefCell::new(None));
            let maps = Rc::new(RefCell::new(KeptMaps::default()));
            engines.push(Engine { name: name.to_string(), realm: None, link: link.clone(), heap: 0, maps: maps.clone() });
            Ok((None, link, maps))
        })
    }

    /// The end of a run in engine `engine` (`None` for a realm that is not kept): the link no longer
    /// holds the run, and the engine keeps `realm`, with the source maps of the code the run
    /// compiled, or is dropped without one.
    fn return_engine(&self, engine: Option<&str>, link: &Link, realm: Option<Realm>) {
        let host = link.borrow_mut().take();
        if let (Some(h), Some(_)) = (&host, &realm) {
            h.keep_maps();
        }
        drop(host);
        let Some(name) = engine else { return };
        let mut realm = realm;
        let heap = realm.as_mut().map(Realm::collect_garbage);
        let gone = with_engines(&self.script_engines, |engines| {
            let at = engines.iter().position(|e| e.name == name)?;
            match (realm.take(), engines.get_mut(at)) {
                (Some(r), Some(e)) => {
                    e.realm = Some(r);
                    e.heap = heap.unwrap_or(0);
                    None
                }
                _ => Some(engines.remove(at)),
            }
        });
        drop(gone);
        drop(realm);
    }

    /// The end of `job`: its engine gets its realm back, with a run that was stopped where it was
    /// ended, unless the run ended with an internal error, which drops the engine.
    fn end_job(&self, job: Job, internal: bool) {
        let Job { mut realm, link, engine, host, .. } = job;
        drop(host);
        realm.end();
        self.return_engine(engine.as_deref(), &link, (!internal).then_some(realm));
    }

    /// The flag that stops the script running in this session from any thread, at its next
    /// instruction.
    pub fn script_interrupt(&self) -> Interrupt {
        self.script_stop.clone()
    }

    /// The report of the run that ended last (`script.run`'s result, or its failure).
    pub fn script_report(&self) -> Option<&Value> {
        self.script_report.as_ref()
    }

    /// The Script Console: the lines of this session's runs, oldest first.
    pub fn script_console(&self) -> &console::Console {
        &self.script_console
    }

    /// Empties the Script Console and sets its counts to zero. The last run's report, and the
    /// report of a running script, keep their lines. → the lines removed.
    pub fn clear_script_console(&mut self) -> usize {
        let n = self.script_console.lines().len();
        self.script_console.clear();
        n
    }

    /// The error text of the last run when it failed: its name, place, message and kind, and the
    /// files it left written.
    pub fn script_failure_text(&self) -> Option<String> {
        self.script_report.as_ref().filter(|r| r["ok"] == false).map(failure_text)
    }

    /// Has the running script used up its growth allowance, so a person must answer? A run asked
    /// to stop no longer waits: the next [`Session::script_pump`] ends it.
    pub fn script_waiting(&self) -> bool {
        !self.script_stop.is_set() && self.script_key.as_ref().and_then(|k| with_job(k, |j| j.host.waiting.get())).unwrap_or(false)
    }

    /// The time left of the running script's `$.sleep`, or none when it is not asleep. A run asked
    /// to stop is not asleep: the next [`Session::script_pump`] ends it.
    pub fn script_sleep_left(&self) -> Option<std::time::Duration> {
        if self.script_stop.is_set() {
            return None;
        }
        self.script_key.as_ref().and_then(|k| with_job(k, |j| j.host.asleep())).flatten()
    }

    /// The dialog (`alert`, `confirm` or `prompt`) the running script waits at for the person's
    /// answer. A run asked to stop no longer waits: the next [`Session::script_pump`] ends it.
    pub fn script_dialog(&self) -> Option<ScriptDialog> {
        if self.script_stop.is_set() {
            return None;
        }
        self.script_key.as_ref().and_then(|k| with_job(k, |j| j.host.dialogs.waiting())).flatten()
    }

    /// Gives the dialog the running script waits at its answer; the next [`Session::script_pump`]
    /// goes on from the call. → false when nothing waits, the answer is for another kind of
    /// dialog, or a prompt's text is longer than [`dialogs::MAX_PROMPT_BYTES`].
    pub fn script_answer(&mut self, answer: DialogAnswer) -> bool {
        let Some(k) = self.script_key.as_ref().filter(|_| !self.script_stop.is_set()) else { return false };
        with_job(k, |j| j.host.dialogs.answer(&mut j.realm, answer)).unwrap_or(false)
    }

    /// The name of the running script.
    pub fn script_name(&self) -> Option<String> {
        self.script_key.as_ref().and_then(|k| with_job(k, |j| j.name.clone()))
    }

    /// Lets the run go past its growth allowance by one more allowance.
    pub fn script_continue(&mut self) {
        if let Some(k) = &self.script_key {
            with_job(k, |j| {
                let h = &j.host;
                if h.waiting.replace(false) {
                    h.continues.set(h.continues.get() + 1);
                    h.allowance_objects.set(h.allowance_objects.get().saturating_add(growth_objects()));
                    h.allowance_bytes.set(h.allowance_bytes.get().saturating_add(GROWTH_BYTES));
                }
            });
        }
    }

    /// Asks the running script to stop; it is rolled back at the next slice. False with none running.
    pub fn script_request_stop(&mut self) -> bool {
        if self.script_key.is_none() {
            return false;
        }
        self.script_stop.set();
        true
    }

    /// Stops the running script at once and puts everything back, with the report of a stopped run
    /// ([`Self::script_report`]). False with none running.
    pub fn script_abandon(&mut self) -> bool {
        let Some(key) = self.script_key.take() else { return false };
        let Some(job) = JOBS.with(|jobs| jobs.borrow_mut().remove(&key.id)) else {
            self.script_gone(key, console::Ending::Stopped);
            return true;
        };
        drop(key);
        let kind = LimitKind::Interrupted;
        let e = RunError { kind: RunErrorKind::Limit(kind), message: format!("limit reached: {kind:?}"), line: 0, column: 0 };
        let _ = self.script_failed(job, &e);
        true
    }

    /// Ends the run whose job is no longer held: its console entry ends with `ending`, with no
    /// report line, since its time and host calls went with the job, and the session's state is
    /// put back as at the end of any run.
    fn script_gone(&mut self, key: JobKey, ending: console::Ending) {
        let line = console::ConsoleLine { run: key.run, name: key.name.clone(), entry: console::Entry::End(ending) };
        self.script_console.push(line, false);
        drop(key);
        // The realm of the job's named engine went with the job, so the engine is dropped.
        let lost: Vec<Engine> = with_engines(&self.script_engines, |engines| {
            let (lost, kept) = std::mem::take(engines).into_iter().partition(|e| e.realm.is_none());
            *engines = kept;
            lost
        });
        drop(lost);
        raise_ids(self);
        self.script_stop.clear();
        self.reset_tool();
        cmd::compositefonts::sync(self);
    }

    /// A script can't start while a document holds a drag in progress or an undo group, which its
    /// one undo step could not take in.
    fn settle_for_script(&mut self) -> Result<()> {
        self.finish_tool_edit_later()?;
        if self.active().is_some() {
            cmd::settle_interaction(self)?;
        }
        if self.explicit_undo_group.is_some() || self.docs.iter().any(|d| d.undo_group.is_some() || d.interaction.is_some()) {
            return Err(EngineError::Other(OPEN_EDIT.to_string()));
        }
        Ok(())
    }

    /// Begins a run of `request`. → the run's name.
    pub(crate) fn script_start(&mut self, request: Request) -> Result<String> {
        self.settle_for_script()?;
        // In a build where a fault ends the program, every unsaved document has a copy first.
        if self.script_copy_first && self.docs.iter().any(DocState::is_dirty) && cmd::recovery::save_copies(self).is_err() {
            return Err(EngineError::Other(NO_RECOVERY_COPY.to_string()));
        }
        self.reset_tool();
        let open: Vec<u64> = self.docs.iter().map(|d| d.uid).collect();
        self.script_high_water.retain(|uid, _| open.contains(uid));
        self.script_stop.clear();
        self.script_report = None;
        let mut files = include::Files::new(request.file.as_deref(), include::Files::jsinclude_env());
        let expanded = files.expand_script(&request.source);
        let name = match &expanded {
            Ok(include::Expanded { name: Some(name), .. }) if !request.name_given => name.clone(),
            _ => request.name,
        };
        // The engine named by `#targetengine`, other than `main`: its realm, kept from its last run,
        // or none for an engine this run makes.
        let named = match &expanded {
            Ok(include::Expanded { engine: Some((engine, line, column)), .. }) if engine != MAIN_ENGINE => Some((engine.clone(), *line, *column)),
            _ => None,
        };
        let (expanded, kept) = match named {
            None => (expanded, None),
            Some((engine, line, column)) => match self.take_engine(&engine) {
                Ok(kept) => (expanded, Some((engine, kept))),
                Err((kind, message)) => {
                    let file = Arc::from(files.file_name());
                    (Err(include::Failure { kind, message, index: 0, file, line, column }), None)
                }
            },
        };
        let heap_bytes = if kept.is_some() { engine_heap() } else { HEAP_BYTES };
        let limits = Limits { fuel: request.fuel, heap_bytes, ..Limits::default() };
        let (engine, realm, link, kept_maps) = match kept {
            None => (None, None, Rc::new(RefCell::new(None)), None),
            Some((engine, (realm, link, maps))) => (Some(engine), realm, link, Some(maps)),
        };
        // A realm made here gets the script extensions, `$` and the dialogs; a kept one has them
        // from its first run, and they act for the run that calls them.
        let fresh = realm.is_none();
        let options = Options { max_source_units: include::MAX_TOTAL_BYTES.saturating_mul(2), ..Options::default() };
        let strict = options.strict;
        let mut realm = match realm {
            Some(mut realm) => {
                realm.set_limits(limits);
                realm
            }
            None => Realm::with_options(options, limits),
        };
        realm.share_interrupt(&self.script_stop);
        realm.set_data_limits(DataLimits::default());
        self.script_runs += 1;
        let host = Rc::new(Host {
            session: Rc::new(RefCell::new(Session::new())),
            pause: realm.pause_flag(),
            framed: self.script_frames,
            calls: Cell::new(0),
            calls_this_slice: Cell::new(0),
            objects: Cell::new(0),
            bytes_kept: Cell::new(0),
            bridge: Cell::new(0),
            allowance_objects: Cell::new(growth_objects()),
            allowance_bytes: Cell::new(GROWTH_BYTES),
            continues: Cell::new(0),
            waiting: Cell::new(false),
            log: RefCell::new(Vec::new()),
            log_bytes: Cell::new(0),
            written: RefCell::new(Vec::new()),
            run: self.script_runs,
            name: console::line_name(&name),
            console: RefCell::default(),
            pending: RefCell::default(),
            counts: Counts::default(),
            commands: RefCell::default(),
            run_nanos: Cell::new(0),
            dialogs: dialogs::Dialogs::new(self.script_frames, &realm, request.answers),
            disk: RefCell::new(files::Disk::new(files.scope())),
            files: RefCell::new(files),
            script_map: RefCell::default(),
            eval_maps: RefCell::default(),
            eval_failures: RefCell::default(),
            script_tag: Cell::new(vectorcraft_js::UNKNOWN_SOURCE),
            report_name: Arc::from(name.as_str()),
            kept_maps,
            started: clock(),
            wake_at: Cell::new(None),
            can_wait: realm.wait_flag(),
            eval_strict: Cell::new(None),
            dollar: dollar::RunState::new(&request.locale),
            args: RefCell::new((Data::Undefined, 0)),
        });
        *link.borrow_mut() = Some(host.clone());
        let args = match to_data(&Value::Object(request.args), &DataLimits::default(), 0, &mut (0, 0)) {
            Ok(args) => args,
            Err(place) => {
                self.return_engine(engine.as_deref(), &link, (!fresh).then_some(realm));
                return Err(EngineError::BadParams { cmd: "script.run".into(), msg: format!("`args{place}` is nested more than 64 levels deep") });
            }
        };
        let size = data_size(&args);
        *host.args.borrow_mut() = (args, size);
        let line = console::ConsoleLine { run: host.run, name: host.name.clone(), entry: console::Entry::Start };
        self.script_console.push(line, false);
        let engine_name = match &expanded {
            Ok(include::Expanded { engine: Some((name, ..)), .. }) => Some(name.clone()),
            _ => None,
        };
        let info = dollar::RunInfo { strict, engine: engine_name };
        let localization = realm.localization();
        dollar::start_run(&localization, &request.locale);
        let started = match expanded {
            Err(f) => Err(preprocessor_error(f)),
            Ok(expanded) => {
                *host.script_map.borrow_mut() = expanded.map;
                realm.set_strict_writes(expanded.strict == Some(true));
                if fresh {
                    let l = link.clone();
                    realm.set_source_locator(Rc::new(move |tag, line| match current(&l) {
                        Ok(h) => h.source_place(tag, line),
                        Err(_) => ("[unknown]".to_string(), 0),
                    }));
                }
                (if fresh { realm.add_script_extensions() } else { Ok(()) })
                    .and_then(|()| if fresh { realm.define_host_object("vc", vc_object(&link)) } else { Ok(()) })
                    .and_then(|()| if fresh { realm.define_host_object("$", dollar_object(&link, info, &localization)) } else { Ok(()) })
                    .and_then(|()| if fresh { dialogs::install(&mut realm, &link) } else { Ok(()) })
                    .and_then(|()| if fresh { files::install(&mut realm, &link) } else { realm.reset_folder_current() })
                    .and_then(|()| {
                        let r = realm.start(&expanded.source);
                        // Source that does not compile defines no code, so its map is not kept.
                        if !matches!(&r, Err(e) if e.kind == RunErrorKind::Syntax) {
                            host.script_tag.set(realm.script_tag());
                        }
                        r
                    })
                    .map_err(|e| located(&host, &realm, &e))
            }
        };
        if let Err((e, file)) = started {
            // An engine this run made is not kept: only a run that starts makes one. An internal
            // error drops a kept one.
            let keep = !fresh && e.kind != RunErrorKind::Internal;
            self.return_engine(engine.as_deref(), &link, keep.then_some(realm));
            let mut report = failure_report(&name, &e, file.as_deref(), &host, 0, 0);
            close_run(self, &host, &mut report, ending(&e.kind));
            let message = failure_text(&report);
            self.script_report = Some(report);
            return Err(EngineError::Other(message));
        }
        let scope = crate::file_access::ScopeHandle::capture();
        let (run, line_name) = (host.run, host.name.clone());
        let job = Job { realm, host, link, engine, world: World::working(self), start_uids: open, name: name.clone(), scope };
        let id = NEXT_JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        JOBS.with(|jobs| jobs.borrow_mut().insert(id, job));
        self.script_key = Some(JobKey { id, run, name: line_name });
        Ok(name)
    }

    /// Advances the running script by one slice. `None` while it is still running; otherwise the
    /// run has ended: its report, or the error that ended it, with the documents put back.
    pub fn script_pump(&mut self) -> Option<Result<Value>> {
        let key = self.script_key.take()?;
        let Some(mut job) = JOBS.with(|jobs| jobs.borrow_mut().remove(&key.id)) else {
            self.script_gone(key, console::Ending::Internal);
            return Some(Err(EngineError::Other(JOB_GONE.to_string())));
        };
        if let Some(e) = job.host.dialogs.take_failure() {
            return Some(self.script_failed(job, &e));
        }
        // A canceled request ends a sleep or a wait at this pump.
        if vectorcraft_doc::cancel::check().is_err() {
            self.script_stop.set();
        }
        if (job.host.waiting.get() || job.host.asleep().is_some() || job.host.dialogs.is_waiting()) && !self.script_stop.is_set() {
            JOBS.with(|jobs| jobs.borrow_mut().insert(key.id, job));
            self.script_key = Some(key);
            return None;
        }
        // A stop while a dialog waits ends the run there; the interpreter cannot go on from the call.
        if job.realm.waits() {
            let kind = LimitKind::Interrupted;
            let e = RunError { kind: RunErrorKind::Limit(kind), message: format!("limit reached: {kind:?}"), line: 0, column: 0 };
            return Some(self.script_failed(job, &e));
        }
        job.host.calls_this_slice.set(0);
        let outcome = run_slice(self, &mut job);
        match outcome {
            Err(panic) => {
                let e = RunError { kind: RunErrorKind::Internal, message: panic, line: 0, column: 0 };
                Some(self.script_failed(job, &e))
            }
            Ok(Err(e)) => Some(self.script_failed(job, &e)),
            Ok(Ok(Progress::Paused)) => {
                JOBS.with(|jobs| jobs.borrow_mut().insert(key.id, job));
                self.script_key = Some(key);
                None
            }
            Ok(Ok(Progress::Done(_))) if job.host.dialogs.unused() > 0 => {
                let n = job.host.dialogs.unused();
                let kind = LimitKind::Host;
                let message = format!("`answers` has {n} answer{} the script did not use", if n == 1 { "" } else { "s" });
                Some(self.script_failed(job, &RunError { kind: RunErrorKind::Limit(kind), message, line: 0, column: 0 }))
            }
            Ok(Ok(Progress::Done(_))) => {
                let r = self.script_commit(job);
                self.trim_histories();
                Some(r)
            }
        }
    }

    fn script_failed(&mut self, job: Job, e: &RunError) -> Result<Value> {
        job.host.dialogs.stopped();
        let fuel_left = job.realm.fuel_left();
        let (e, file) = located(&job.host, &job.realm, e);
        let mut report = failure_report(&job.name, &e, file.as_deref(), &job.host, fuel_left, job.realm.limits().fuel.saturating_sub(fuel_left));
        close_run(self, &job.host, &mut report, ending(&e.kind));
        self.end_job(job, e.kind == RunErrorKind::Internal);
        raise_ids(self);
        self.script_stop.clear();
        self.reset_tool();
        cmd::compositefonts::sync(self);
        let message = failure_text(&report);
        self.script_report = Some(report);
        Err(EngineError::Other(message))
    }

    /// Makes the working copy the session's state: one undo step for each document whose art
    /// changed, nothing for the others.
    fn script_commit(&mut self, mut job: Job) -> Result<Value> {
        // The working copy goes into the session; `job.world` holds the state before the run.
        job.world.swap(self);
        self.prefs = merged_prefs(&job.world.start_prefs, &self.prefs, &job.world.choices.prefs);
        // The Caps Lock state takes the run's value only when the run changed it.
        if self.caps_lock == job.world.start_caps_lock {
            self.caps_lock = job.world.choices.caps_lock;
        }
        note_ids(self);
        raise_ids(self);
        let mut start = std::mem::take(&mut job.world.docs);
        for st in &self.docs {
            let changed = start.iter().find(|b| b.uid == st.uid).is_none_or(|b| !st.same_content(&b.doc));
            if changed && (!crate::sanity::doc_sane(&st.doc, &st.selection) || st.doc.too_deep(None)) {
                job.world.docs = start;
                job.world.swap(self);
                let e = RunError { kind: RunErrorKind::Internal, message: INVALID_RESULT.to_string(), line: 0, column: 0 };
                return self.script_failed(job, &e);
            }
        }
        let mut changed_names = Vec::new();
        for st in &mut self.docs {
            let Some(b) = start.iter_mut().find(|b| b.uid == st.uid) else {
                // A document the script opened or made starts with no undo steps.
                st.history = crate::History { limit: st.history.limit, ..crate::History::default() };
                st.revision += 1;
                changed_names.push(st.title());
                continue;
            };
            let changed = !st.same_content(&b.doc);
            st.history = std::mem::take(&mut b.history);
            st.recovery.clone_from(&b.recovery);
            if changed {
                st.push_undo(STEP_LABEL.to_string(), b.doc.clone(), Arc::new(b.selection.clone()));
                changed_names.push(st.title());
            }
            if changed || st.revision != b.revision {
                st.revision = st.revision.max(b.revision) + 1;
            }
            st.sync_new_art();
        }
        // A document a frontend opened while the run went on stays open.
        for st in start.iter().filter(|b| !job.start_uids.contains(&b.uid)) {
            self.docs.push(st.clone());
        }
        // A document the script closed is gone now, with its recovery copy.
        for b in start.iter().filter(|b| job.start_uids.contains(&b.uid) && !self.docs.iter().any(|d| d.uid == b.uid)) {
            cmd::recovery::forget_closed(self, b);
        }
        // A document the script saved has nothing left to recover.
        cmd::recovery::forget_clean(self);
        let entries: Vec<_> = self.journal.iter().cloned().collect();
        self.journal = std::mem::take(&mut job.world.journal);
        for (id, p) in entries {
            self.journal.push((id, p));
        }
        self.script_stop.clear();
        self.reset_tool();
        cmd::compositefonts::sync(self);
        let host = &job.host;
        let mut report = json!({
            "ok": true,
            "name": job.name,
            "changed": changed_names,
            "written": *host.written.borrow(),
            "log": *host.log.borrow(),
            "fuelUsed": job.realm.limits().fuel - job.realm.fuel_left(),
            "hostCalls": host.calls.get(),
        });
        close_run(self, host, &mut report, console::Ending::Finished);
        self.end_job(job, false);
        self.script_report = Some(report.clone());
        Ok(report)
    }

    /// Runs the running script until it ends.
    pub(crate) fn script_run_to_end(&mut self) -> Result<Value> {
        loop {
            if let Some(result) = self.script_pump() {
                return result;
            }
            if self.script_waiting() || self.script_dialog().is_some() {
                self.script_stop.set();
            }
            // A `$.sleep` waits here, where nothing is drawn, instead of turning the loop over.
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(left) = self.script_sleep_left() {
                std::thread::sleep(left.min(std::time::Duration::from_millis(100)));
            }
        }
    }
}

const OPEN_EDIT: &str = "a drag or an undo group is open; finish it before running a script";
const INVALID_RESULT: &str = "the script left a document that cannot be kept";
const JOB_GONE: &str = "the script's run is no longer available";
const HISTORY_LOST: &str = "a failed command changed undo steps that could not be put back, so the script was stopped";
// Text a script reads from the errors it catches, or that ends its run with its name and place.
const OVER_CALLS: &str = "the script made more host calls than one run may make";
const OVER_BRIDGE: &str = "the script passed more data to and from commands than one run may pass";
const OVER_GROWTH: &str = "the script added more objects and data to the documents than one run may add";
const NO_COMMAND_ID: &str = "vc.run: give a command id, as a string";
const TOO_MANY_ARGUMENTS: &str = "vc.run: takes a command id and an object of parameters";
const PARAMS_NOT_OBJECT: &str = "vc.run: the parameters must be an object";
const LOG_FULL: &str = "vc.log: the log of this run is full";
const NO_RECOVERY_COPY: &str = "a recovery copy could not be written, so the script was not run";
const EVAL_NO_PATH: &str = "$.evalFile requires a file path";
const EVAL_TIMEOUT: &str = "$.evalFile: the timeout must be a finite number of milliseconds, 0 or more";
const EVAL_ARGUMENTS: &str = "$.evalFile: takes a file path and a timeout";
const NO_RUN: &str = "this function's script run has ended";
const DROP_MAIN: &str = "the main script engine is new for each run and is never kept, so it cannot be dropped";
const NO_ENGINES_ON_WEB: &str = "named script engines (#targetengine) are not available on the web";
const ENGINE_BUSY: &str = "the script engine is in use by another run";

/// The report of a run that failed; `file` is the included file the error is in.
fn failure_report(name: &str, e: &RunError, file: Option<&str>, host: &Host, fuel_left: u64, fuel_used: u64) -> Value {
    let mut report = json!({
        "ok": false,
        "kind": kind_id(&e.kind),
        "message": explained(&e.kind, &e.message),
        "name": name,
        "line": e.line,
        "column": e.column,
        "written": *host.written.borrow(),
        "log": *host.log.borrow(),
        "fuelLeft": fuel_left,
        "fuelUsed": fuel_used,
        "hostCalls": host.calls.get(),
    });
    if let (Some(file), Some(o)) = (file, report.as_object_mut()) {
        o.insert("file".into(), json!(file));
    }
    report
}

/// How the run that failed with `kind` ended, as its console end line gives it.
fn ending(kind: &RunErrorKind) -> console::Ending {
    match kind {
        RunErrorKind::Syntax | RunErrorKind::Thrown => console::Ending::Error,
        RunErrorKind::Limit(LimitKind::Fuel) => console::Ending::Fuel,
        RunErrorKind::Limit(LimitKind::Host) => console::Ending::HostLimit,
        RunErrorKind::Limit(LimitKind::Interrupted) => console::Ending::Stopped,
        RunErrorKind::Limit(_) => console::Ending::Limit,
        RunErrorKind::Internal => console::Ending::Internal,
    }
}

/// Adds a line of `host`'s run to the session's console and to the run's own.
fn line_to(s: &mut Session, host: &Host, entry: console::Entry, cut: bool) {
    let line = console::ConsoleLine { run: host.run, name: host.name.clone(), entry };
    s.script_console.push(line.clone(), cut);
    host.console.borrow_mut().push(line, cut);
}

/// The error of the failed run `report` describes, as its console lines give it; `name` is the
/// run's name in those lines ([`console::line_name`]).
fn error_text(name: &str, report: &Value) -> String {
    let (line, column) = (report["line"].as_u64().unwrap_or(0), report["column"].as_u64().unwrap_or(0));
    let message = report["message"].as_str().unwrap_or_default();
    match report["file"].as_str() {
        Some(file) if line > 0 => format!("{name}: {file}:{line}:{column}: {message}"),
        _ if line > 0 => format!("{name}:{line}:{column}: {message}"),
        _ => format!("{name}: location unavailable: {message}"),
    }
}

/// The console lines of the error of the failed run `report` describes, each as the console keeps
/// it, cut to its limit.
pub(crate) fn error_lines(report: &Value) -> Vec<String> {
    let name = console::line_name(report["name"].as_str().unwrap_or_default());
    let mut lines = vec![];
    each_line(&error_text(&name, report), |line, _| lines.push(line));
    lines
}

/// Ends the console lines of `host`'s run: the line `$.write` left unended, then for a failed run
/// (`report["ok"]` false) its error as `name:line:column: message`, then an end line and a report
/// line in the session's console. Adds the run's time, host calls by kind, slowest commands,
/// console and dialogs to `report`.
fn close_run(s: &mut Session, host: &Host, report: &mut Value, ending: console::Ending) {
    let mut pending = std::mem::take(&mut *host.pending.borrow_mut());
    if !pending.is_empty() {
        let mut last = None;
        pending.end_line(|line, cut| last = Some((line, cut)));
        if let Some((line, cut)) = last {
            line_to(s, host, console::Entry::Output(line), cut);
        }
    }
    if report["ok"] == false {
        each_line(&error_text(&host.name, report), |line, cut| line_to(s, host, console::Entry::Error(line), cut));
    }
    let timing = clock().is_some();
    let ms = millis(host.run_nanos.get());
    let slow = slowest(&host.commands.borrow());
    let slow_json: Vec<Value> = slow.iter().map(|c| json!({"id": c.id, "calls": c.calls, "totalMs": c.total_ms, "maxMs": c.max_ms})).collect();
    let line = |entry| console::ConsoleLine { run: host.run, name: host.name.clone(), entry };
    s.script_console.push(line(console::Entry::End(ending)), false);
    let summary = console::Entry::Report { ms: timing.then_some(ms), host_calls: host.calls.get(), slowest: slow };
    s.script_console.push(line(summary), false);
    if let Some(o) = report.as_object_mut() {
        o.insert("runTimeMs".into(), json!(if timing { ms } else { 0 }));
        o.insert("timingAvailable".into(), json!(timing));
        o.insert("hostCallCounts".into(), host.counts.json());
        o.insert("slowestCommands".into(), Value::Array(slow_json));
        o.insert("console".into(), host.console.borrow().report());
        host.dialogs.report(o);
    }
}

/// The text of a failed run's error.
fn failure_text(report: &Value) -> String {
    let s = |k: &str| report[k].as_str().unwrap_or_default();
    let (line, column) = (report["line"].as_u64().unwrap_or(0), report["column"].as_u64().unwrap_or(0));
    let at = match report["file"].as_str() {
        Some(file) if line > 0 => format!(": line {line}, column {column} of {file}"),
        _ if line > 0 => format!(": line {line}, column {column}"),
        _ => String::new(),
    };
    let written = report["written"].as_array().map_or(0, Vec::len);
    let kept = if written > 0 {
        let paths: Vec<&str> = report["written"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        format!(" (these files stay written: {})", paths.join(", "))
    } else {
        String::new()
    };
    format!("{}{at}: {} ({}){kept}", s("name"), s("message"), s("kind"))
}

/// A run's slice: the working copy in, the real session lent to the host functions, one measured
/// slice run in the file-access scope the run began in, and everything back. A panic in the
/// interpreter is the outer `Err`.
fn run_slice(s: &mut Session, job: &mut Job) -> std::result::Result<std::result::Result<Progress, RunError>, String> {
    let depth = std::mem::replace(&mut s.depth, 0);
    job.world.swap(s);
    // The run's front document draws and converts through its own profiles, and the interface's
    // again after the slice.
    s.sync_front_profiles();
    s.script_in_run = true;
    let cell = job.host.session.clone();
    std::mem::swap(s, &mut *cell.borrow_mut());
    let scope = job.scope.clone();
    let realm = &mut job.realm;
    let started = clock();
    let r = scope.run(|| {
        guard::catch_panic(|| {
            #[cfg(test)]
            if PANIC_IN_SLICE.with(Cell::get) {
                panic!("a slice failed");
            }
            realm.advance(SLICE_FUEL)
        })
    });
    if let Some(started) = started {
        job.host.run_nanos.set(job.host.run_nanos.get().saturating_add(started.elapsed().as_nanos()));
    }
    std::mem::swap(s, &mut *cell.borrow_mut());
    s.script_in_run = false;
    job.world.swap(s);
    s.depth = depth;
    s.sync_front_profiles();
    r
}

/// Tests: removes the session's running job while the session keeps its hold on it. → whether a
/// job was removed.
#[cfg(test)]
pub(crate) fn forget_job(s: &Session) -> bool {
    s.script_key.as_ref().is_some_and(|k| JOBS.with(|jobs| jobs.borrow_mut().remove(&k.id)).is_some())
}

/// The objects a run may add before it is asked to continue or stop.
fn growth_objects() -> u64 {
    #[cfg(test)]
    if let Some(n) = TEST_GROWTH.with(Cell::get) {
        return n;
    }
    GROWTH_OBJECTS
}

/// The limits of a command's result on its way to the script.
fn result_limits() -> DataLimits {
    #[cfg(test)]
    if let Some(bytes) = TEST_RESULT_BYTES.with(Cell::get) {
        return DataLimits { bytes, ..DataLimits::default() };
    }
    DataLimits::default()
}

#[cfg(test)]
thread_local! {
    /// Tests: the bytes a command's result may hold on its way to the script.
    pub(crate) static TEST_RESULT_BYTES: Cell<Option<usize>> = const { Cell::new(None) };
    /// Tests: makes the next slice panic.
    pub(crate) static PANIC_IN_SLICE: Cell<bool> = const { Cell::new(false) };
    /// Tests: the objects a run may add in place of [`GROWTH_OBJECTS`].
    pub(crate) static TEST_GROWTH: Cell<Option<u64>> = const { Cell::new(None) };
    /// Tests: the heap of a named engine in place of [`ENGINE_HEAP_BYTES`].
    pub(crate) static TEST_ENGINE_HEAP: Cell<Option<usize>> = const { Cell::new(None) };
    /// Tests: the bytes of source maps a named engine keeps, in place of [`KEPT_MAP_BYTES`].
    pub(crate) static TEST_KEPT_MAP_BYTES: Cell<Option<usize>> = const { Cell::new(None) };
    /// Tests: the `JSINCLUDE` folders in place of the environment's.
    pub(crate) static TEST_JSINCLUDE: RefCell<Option<Vec<std::path::PathBuf>>> = const { RefCell::new(None) };
    /// Tests: the time since a run started, in place of its clock's.
    pub(crate) static TEST_ELAPSED: Cell<Option<std::time::Duration>> = const { Cell::new(None) };
    /// Tests: runs have no clock, as in a wasm build.
    pub(crate) static TEST_NO_CLOCK: Cell<bool> = const { Cell::new(false) };
}
