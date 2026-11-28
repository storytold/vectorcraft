//! File › Scripts: the scripts of the Scripts folder next to the settings
//! ([`vectorcraft_engine::Session::attach_scripts_dir`]), each `.js` or `.jsx` file by its name
//! without the extension and each subfolder as a submenu, above Check Script… and Other Script….
//!
//! The folder is read by a job of the host's queue ([`vectorcraft_engine::script::folder::list`]:
//! names only, within its limits), one read at a time: when the app starts, each time its window
//! becomes the active one, and each time a menu of the window's menu bar opens ([`refresh`]). The
//! menus show the last finished listing ([`items`]), so a script added to the folder is listed
//! without a restart.
//!
//! Until the first read ends the menu shows [`LOADING`], and [`SLOW`] once that read has taken
//! [`SLOW_SECS`].
//!
//! `ui.runInstalledScript {path}` runs a listed script (`script.run`) without the warning Other
//! Script… shows: the file was put in the folder to be run from the menu. The path is one a
//! listing gave; a file that is no longer a plain script under the folder is not run. With General ›
//! Run Startup Scripts on, the startup scripts run first (`script.startup.run`).

// PLAN: needs craft-menu, craft-host and craft-platform in crates/ui-egui/Cargo.toml (shell owner).
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use craft::host::queue::{JobQueue, Priority, QueueOptions};
use craft::host::{Host, Wake};
use craft::menu::{CommandRef, Item, Node};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::scripts::NOT_LISTED;
use vectorcraft_engine::script::folder::{Entry, Listing};

use crate::VectorcraftApp;

/// The command a listed script's menu item runs.
pub const COMMAND: &str = "ui.runInstalledScript";
/// The menu's note under a listing that may lack scripts.
pub const TRUNCATED: &str = "Some scripts were not listed";
/// The menu's note when the folder is there and cannot be read.
pub const UNREADABLE: &str = "The Scripts folder cannot be read";
/// The menu's note while the first read is under way.
pub const LOADING: &str = "Loading scripts…";
/// The menu's note once the first read has taken [`SLOW_SECS`].
pub const SLOW: &str = "The Scripts folder is still being read";
/// The notes, for the catalogs ([`crate::menus::menu_strings`]).
pub const LABELS: &[&str] = &[TRUNCATED, UNREADABLE, LOADING, SLOW];
/// How long the first read runs before the menu shows [`SLOW`].
pub const SLOW_SECS: f64 = 2.0;

/// The command Other Script… runs, which only File › Scripts itself lists.
const OTHER_SCRIPT: &str = "ui.runScript";

#[derive(Default)]
struct State {
    /// The reads of the folder, keyed by the folder; built at the first read.
    queue: Option<JobQueue<PathBuf, Listing>>,
    /// The last finished listing.
    listing: Arc<Listing>,
    /// How many finished listings differed from the one before, and how often the note before the
    /// first listing changed.
    revision: u64,
    /// A listing has been taken in.
    loaded: bool,
    /// When the first read started, in seconds of [`craft::platform::clock::monotonic_ms`], until a
    /// listing comes.
    first_read: Option<f64>,
}

impl State {
    /// The queue, built on the host's jobs; false without a host (no listing is read then).
    fn ensure_queue(&mut self, ctx: &egui::Context) -> bool {
        if self.queue.is_none() {
            let Some(host) = Host::try_of_ctx(ctx) else { return false };
            let wake = ctx.clone();
            let wake = Wake::new(move || wake.request_repaint());
            self.queue = Some(JobQueue::new(host.jobs_shared().clone(), wake, QueueOptions::default()));
        }
        true
    }

    /// The note in place of a listing before the first one, at `now` seconds.
    fn note_at(&self, now: f64) -> Option<&'static str> {
        if self.loaded {
            return None;
        }
        let started = self.first_read?;
        Some(if now - started >= SLOW_SECS { SLOW } else { LOADING })
    }

    /// Show a finished listing; the first one also takes the place of the loading note.
    fn take_in(&mut self, listing: Listing) {
        let changed = *self.listing != listing;
        if changed {
            self.listing = Arc::new(listing);
        }
        if changed || !self.loaded {
            self.revision += 1;
        }
        (self.loaded, self.first_read) = (true, None);
    }
}

/// The listing the menus show. One per process in the app; one per thread in unit tests, which run
/// in parallel threads, each with its own folder.
fn state() -> MutexGuard<'static, State> {
    #[cfg(not(test))]
    let state: &'static Mutex<State> = {
        static S: std::sync::OnceLock<Mutex<State>> = std::sync::OnceLock::new();
        S.get_or_init(Default::default)
    };
    #[cfg(test)]
    let state: &'static Mutex<State> = {
        thread_local!(static S: &'static Mutex<State> = Box::leak(Box::default()));
        S.with(|s| *s)
    };
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// The clock the notes follow, in seconds.
fn now() -> f64 {
    craft::platform::clock::monotonic_ms() / 1000.0
}

/// The most different names the menu shows in one session. Each is kept for the rest of the
/// session (menu items hold `&'static str` labels), at most 128 characters each.
pub const MAX_NAMES: usize = 16_384;

/// Leak-once interning of the names the menu shows; `None` for a new name once [`MAX_NAMES`] are
/// kept, so renaming scripts over and over keeps the memory bounded.
pub(crate) fn intern(s: &str) -> Option<&'static str> {
    #[cfg(not(test))]
    let set: &'static Mutex<std::collections::HashSet<&'static str>> = {
        static SET: std::sync::OnceLock<Mutex<std::collections::HashSet<&'static str>>> = std::sync::OnceLock::new();
        SET.get_or_init(Default::default)
    };
    #[cfg(test)]
    let set: &'static Mutex<std::collections::HashSet<&'static str>> = {
        thread_local!(static SET: &'static Mutex<std::collections::HashSet<&'static str>> = Box::leak(Box::default()));
        SET.with(|s| *s)
    };
    let mut set = set.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = set.get(s) {
        return Some(v);
    }
    if set.len() >= MAX_NAMES {
        return None;
    }
    let v: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(v);
    Some(v)
}

/// Changes whenever the menus' listing or the note before the first listing changes, so the menus
/// are laid out again. The low bit is the [`SLOW`] note, which the clock brings in.
pub fn revision() -> u64 {
    let state = state();
    state.revision * 2 + u64::from(state.note_at(now()) == Some(SLOW))
}

/// Read the Scripts folder again, unless the session has no folder (the web, and a run without
/// preferences) or the context has no host. A read asked for while one runs waits for it (the
/// queue runs one job per folder). `ctx` is asked to draw when the read ends.
pub fn refresh(app: &VectorcraftApp, ctx: &egui::Context) {
    let Some(root) = app.session.scripts_dir().map(Path::to_path_buf) else { return };
    let mut state = state();
    if !state.ensure_queue(ctx) {
        return;
    }
    if let Some(queue) = &state.queue {
        let read = root.clone();
        let clock = app.session.script_listing_clock();
        // The app's own folder, read for the person at the keyboard.
        queue.submit(root, Priority::VISIBLE, move |_| {
            vectorcraft_engine::file_access::unconfined(|| vectorcraft_engine::script::folder::list(&read, clock))
        });
    }
    if !state.loaded && state.first_read.is_none() {
        // The menus show the loading note, and a frame comes when it is due to change to SLOW.
        state.first_read = Some(now());
        state.revision += 1;
        ctx.request_repaint_after(Duration::from_secs_f64(SLOW_SECS));
    }
}

/// Once per pass: read the folder at the first pass and in a pass whose input says the window
/// became the active one, then take in the reads that ended. Only the pass that receives the
/// window event reads; a second pass of the frame just takes in.
pub fn poll(app: &VectorcraftApp, ctx: &egui::Context) {
    let first = state().queue.is_none();
    if first || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
        refresh(app, ctx);
    }
    take();
}

/// Take in the reads that ended.
fn take() {
    let mut state = state();
    let Some(queue) = &state.queue else { return };
    let done: Vec<Listing> = std::iter::from_fn(|| queue.try_recv()).map(|done| done.result).collect();
    for listing in done {
        state.take_in(listing);
    }
}

/// Wait for the reads under way and take them in.
#[cfg(test)]
pub(crate) fn finish() {
    {
        let state = state();
        if let Some(queue) = &state.queue {
            queue.finish(Duration::from_secs(10));
        }
    }
    take();
}

/// File › Scripts' items: the listed scripts and subfolders, a note when some may be missing or
/// the folder cannot be read, or one before the first listing, then Check Script… and Other
/// Script….
pub fn items() -> Vec<Node> {
    items_at(now())
}

/// [`items`] with the notes as they are at `now` seconds.
pub(crate) fn items_at(now: f64) -> Vec<Node> {
    fn level(entries: &[Entry]) -> Vec<Node> {
        entries
            .iter()
            .filter_map(|e| match e {
                Entry::Script { label, path } => {
                    let command = CommandRef::with_params(COMMAND, json!({ "path": path.to_string_lossy() }));
                    Some(Node::Item(Item::new(command, label.to_string())))
                }
                Entry::Folder { label, entries } => {
                    let inner = level(entries);
                    (!inner.is_empty()).then(|| Node::submenu(label.to_string(), inner))
                }
            })
            .collect()
    }
    let (listing, note) = {
        let state = state();
        (state.listing.clone(), state.note_at(now))
    };
    let mut nodes = level(&listing.entries);
    let notes = [listing.truncated.then_some(TRUNCATED), listing.unreadable.then_some(UNREADABLE), note];
    nodes.extend(notes.into_iter().flatten().map(crate::menus::placeholder));
    if !nodes.is_empty() {
        nodes.push(Node::Separator);
    }
    nodes.extend([
        Node::Item(Item::new(CommandRef::new("ui.checkScript"), "Check Script…")),
        Node::Item(Item::new(CommandRef::new(OTHER_SCRIPT), "Other Script…")),
    ]);
    nodes
}

/// Is `children` a subfolder's submenu, named after the folder? Its first item is a listed script
/// or another subfolder, and it has no Other Script…, which only File › Scripts itself has.
pub fn is_folder(children: &[Node]) -> bool {
    let listed = match children.first() {
        Some(Node::Item(item)) => item.command.id == COMMAND,
        Some(Node::Submenu { children, .. }) => is_folder(children),
        _ => false,
    };
    listed && !children.iter().any(|c| matches!(c, Node::Item(item) if item.command.id == OTHER_SCRIPT))
}

/// May the request being served read the listed script `p` names? Under an automation read root
/// (a control client's request), a script outside the root is left out of `ui.menu.list`, as
/// `script.listInstalled` refuses to list the folder there.
pub fn readable(p: &Value) -> bool {
    p.get("path").and_then(Value::as_str).is_some_and(|path| vectorcraft_engine::file_access::check_read(path).is_ok())
}

/// `ui.runInstalledScript {path}`: run the listed script at `path`. `script.run` reads the file
/// once, checks it as a listed script and runs that text, so its `#include` lines and `$.evalFile`
/// read in the folder that read found. A script that is no longer in the folder is not run, and the
/// folder is read again.
pub fn run(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let path = p.get("path").and_then(Value::as_str).ok_or("missing `path`")?;
    // With startup scripts to run first, a script no longer in the folder is found before they run.
    let listed = match app.session.startup_skip() {
        Some(_) => Ok(()),
        None => app.session.installed_script(path).map(drop).map_err(|e| e.to_string()),
    };
    listed.and_then(|()| crate::script_runs::run_chosen(app, json!({ "path": path, "installed": true }))).inspect_err(|e| {
        if e == NOT_LISTED
            && let Some(ctx) = app.ui_ctx.clone()
        {
            refresh(app, &ctx);
        }
    })
}
