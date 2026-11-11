//! File › Scripts: the scripts of the Scripts folder next to the settings
//! ([`vectorcraft_engine::Session::attach_scripts_dir`]), each `.js` or `.jsx` file by its name
//! without the extension and each subfolder as a submenu, above Check Script… and Other Script….
//!
//! The folder is read on another thread ([`vectorcraft_engine::script::folder::list`]: names only,
//! within its limits), one read at a time: when the app starts, each time its window becomes the
//! active one, and each time the File menu of the window's menu bar opens ([`refresh`]). The menus show the last finished
//! listing ([`items`]), so a script added to the folder is listed without a restart.
//!
//! `ui.runInstalledScript {path}` runs a listed script (`script.run`) without the warning Other
//! Script… shows: the file was put in the folder to be run from the menu. The path is one a
//! listing gave; a file that is no longer a plain script under the folder is not run.

use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};
use vectorcraft_engine::script::folder::{Entry, Listing};

use crate::VectorcraftApp;
use crate::menus::Item;

/// The command a listed script's menu item runs.
pub const COMMAND: &str = "ui.runInstalledScript";
/// The menu's note under a listing that may lack scripts.
pub const TRUNCATED: &str = "Some scripts were not listed";
/// The menu's note when the folder is there and cannot be read.
pub const UNREADABLE: &str = "The Scripts folder cannot be read";
/// The notes, for the catalogs ([`crate::menus::menu_strings`]).
pub const LABELS: &[&str] = &[TRUNCATED, UNREADABLE];

#[derive(Default)]
struct State {
    /// The last finished listing.
    listing: Arc<Listing>,
    /// How many finished listings differed from the one before.
    revision: u64,
    /// The listing being read on another thread.
    reading: Option<Receiver<Listing>>,
    /// Another read was asked for while one was under way; [`poll`] starts it when that one ends.
    pending: bool,
    /// A first read was asked for.
    started: bool,
    /// The window was the active one at the last [`poll`].
    focused: bool,
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

/// Changes whenever the menus' listing changes (the macOS menu bar follows it).
pub fn revision() -> u64 {
    state().revision
}

/// Read the Scripts folder again on another thread, unless the session has no folder (the web, and
/// a run without preferences). While a read is under way, another read starts when it ends
/// ([`poll`]). `ctx` is asked to draw when the read ends.
pub fn refresh(app: &VectorcraftApp, ctx: Option<&egui::Context>) {
    let Some(root) = app.session.scripts_dir().map(Path::to_path_buf) else { return };
    let mut state = state();
    if state.reading.is_some() {
        state.pending = true;
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.cloned();
    let spawned = std::thread::Builder::new().name("vectorcraft-scripts".into()).spawn(move || {
        // The app's own folder, read for the person at the keyboard.
        let listing = vectorcraft_engine::file_access::unconfined(|| vectorcraft_engine::script::folder::list(&root));
        if tx.send(listing).is_ok()
            && let Some(ctx) = ctx
        {
            ctx.request_repaint();
        }
    });
    if spawned.is_ok() {
        state.reading = Some(rx);
    }
}

/// Once per iteration: start a read on the first one and when the window becomes the active one,
/// take in a read that ended, and then start a read asked for while it was under way.
pub fn poll(app: &VectorcraftApp, ctx: &egui::Context) {
    let focused = ctx.input(|i| i.focused);
    let start = {
        let mut state = state();
        let start = !state.started || (focused && !state.focused);
        (state.started, state.focused) = (true, focused);
        start
    };
    if start {
        refresh(app, Some(ctx));
    }
    take(false);
    let again = {
        let mut state = state();
        let again = state.pending && state.reading.is_none();
        if again {
            state.pending = false;
        }
        again
    };
    if again {
        refresh(app, Some(ctx));
    }
}

/// Take in the read under way: when it has ended, or with `wait` once it ends.
fn take(wait: bool) {
    let mut state = state();
    let Some(reading) = &state.reading else { return };
    let listing = if wait { reading.recv().map_err(|_| TryRecvError::Disconnected) } else { reading.try_recv() };
    match listing {
        Ok(listing) => {
            if *state.listing != listing {
                state.listing = Arc::new(listing);
                state.revision += 1;
            }
            state.reading = None;
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => state.reading = None,
    }
}

/// Wait for the read under way and take it in.
#[cfg(test)]
pub(crate) fn wait() {
    take(true);
}

/// Wait for the read under way to end and leave its listing to be taken in, so the read is under
/// way until the next [`poll`].
#[cfg(test)]
pub(crate) fn end_untaken() {
    let mut state = state();
    let Some(reading) = &state.reading else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    if let Ok(listing) = reading.recv() {
        let _ = tx.send(listing);
    }
    state.reading = Some(rx);
}

/// File › Scripts' items: the listed scripts and subfolders, a note when some may be missing or
/// the folder cannot be read, then Check Script… and Other Script….
pub fn items() -> Vec<Item> {
    // A row whose name is past [`MAX_NAMES`] is left out (`dropped`).
    fn level(entries: &[Entry], dropped: &mut bool) -> Vec<Item> {
        let mut items = Vec::with_capacity(entries.len());
        for e in entries {
            let item = match e {
                Entry::Script { label, path } => intern(label).map(|l| Item::Cmd(l, COMMAND, json!({ "path": path.to_string_lossy() }))),
                Entry::Folder { label, entries } => {
                    let inner = level(entries, dropped);
                    intern(label).filter(|_| !inner.is_empty()).map(|l| Item::Sub(l, inner))
                }
            };
            match item {
                Some(item) => items.push(item),
                None => *dropped = true,
            }
        }
        items
    }
    let listing = state().listing.clone();
    let mut dropped = false;
    let mut items = level(&listing.entries, &mut dropped);
    if listing.truncated || dropped {
        items.push(Item::Header(TRUNCATED));
    }
    if listing.unreadable {
        items.push(Item::Header(UNREADABLE));
    }
    if !items.is_empty() {
        items.push(Item::Sep);
    }
    items.extend([Item::Cmd("Check Script…", "ui.checkScript", Value::Null), Item::Cmd("Other Script…", "ui.runScript", Value::Null)]);
    items
}

/// Is `children` a subfolder's submenu, named after the folder? Its first item is a listed script
/// or another subfolder, and it has no Other Script…, which only File › Scripts itself has.
pub fn is_folder(children: &[Item]) -> bool {
    let listed = match children.first() {
        Some(Item::Cmd(_, id, _)) => *id == COMMAND,
        Some(Item::Sub(_, inner)) => is_folder(inner),
        _ => false,
    };
    listed && !children.iter().any(|c| matches!(c, Item::Cmd(_, "ui.runScript", _)))
}

/// May the request being served read the listed script `p` names? Under an automation read root
/// (a control client's request), a script outside the root is left out of `ui.menu.list`, as
/// `script.listInstalled` refuses to list the folder there.
pub fn readable(p: &Value) -> bool {
    p.get("path").and_then(Value::as_str).is_some_and(|path| vectorcraft_engine::file_access::check_read(path).is_ok())
}

/// `ui.runInstalledScript {path}`: run the listed script at `path`. A script that is no longer in
/// the folder is not run, and the folder is read again. The script runs from its file, so its
/// `#include` lines and `$.evalFile` read beside it.
pub fn run(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let path = p.get("path").and_then(Value::as_str).ok_or("missing `path`")?;
    app.session.installed_script(path).map_err(|e| {
        let ctx = app.ui_ctx.clone();
        refresh(app, ctx.as_ref());
        e.to_string()
    })?;
    app.run("script.run", json!({ "path": path }))
}
