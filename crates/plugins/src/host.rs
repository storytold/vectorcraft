//! What a plug-in may import from the host ([`IMPORTS`]), from plug-in API level 2 on:
//! `vc_progress` reports how far a run has got, `vc_cancelled` tells it the user or a host limit
//! asked it to stop, and `vc_log` writes a log line (the `progress` permission); `vc_settings_*`
//! read and write its settings (`settings`) and `vc_cache_*` its cache (`cache`, [`crate::storage`]).
//! A module may import one only when its manifest gives `api` 2 and lists the import's permission,
//! and the host checks each import when the module is loaded ([`check_imports`], [`check_granted`]):
//! one it doesn't offer, one with another signature and one the manifest doesn't allow are refused
//! by name.
//!
//! Each call's arguments are checked: progress is clamped and passed on only when it changes (with
//! a known total, to another tenth of a percent; past [`MAX_PROGRESS_REPORTS`], once per
//! [`PROGRESS_STRIDE`] changes), log text must lie inside the module's memory and be UTF-8, and
//! each line ([`MAX_LOG_LINE`]) and each run ([`MAX_LOG_LINES`], [`MAX_LOG_BYTES`],
//! [`MAX_LOG_READ`]) are capped. Keys and values must lie inside the module's memory, keys must be
//! [`crate::storage::valid_name`]s, values are capped, and so are each run's storage calls
//! ([`MAX_STORAGE_CALLS`]) and the bytes they move ([`MAX_STORAGE_BYTES`]). Nothing a module passes
//! can panic the host.

use std::sync::Arc;

use wasmi::{Caller, Engine, Extern, ExternType, Linker, Module, StoreLimits, ValType};

use crate::manifest::Manifest;
use crate::permissions::{self, PROGRESS_API, Permission};
use crate::storage::{self, Meter, Storage, StorageError};
use crate::{Error, Result};

/// The module a plug-in imports host functions from (the name a Rust or C toolchain gives
/// `extern` functions by default).
pub const MODULE: &str = "env";

/// What the host passes as the step's description with a plug-in's progress
/// ([`vectorcraft_doc::cancel::progress`]), so a progress window shows a share rather than a count
/// of artboards or files.
pub const PROGRESS_STEP: &str = "plug-in";

/// Longest log line: its first 1,024 bytes of text, shown as at most 1,024 characters with hidden
/// characters escaped. A longer one is cut and ends with `…`.
pub const MAX_LOG_LINE: usize = 1024;

/// Most log lines one run writes; later ones are dropped, after one "log truncated" line.
pub const MAX_LOG_LINES: usize = 200;

/// Most bytes of log text, as written with its escapes, one run writes; lines past it are dropped
/// as past [`MAX_LOG_LINES`].
pub const MAX_LOG_BYTES: usize = 32 * 1024;

/// Most bytes of text the host reads from a run's `vc_log` calls, checked or not: a call that
/// would read past it is dropped as past [`MAX_LOG_LINES`].
pub const MAX_LOG_READ: usize = 1024 * 1024;

/// Most progress reports one run passes on at the full rate; after them, one changed report in
/// [`PROGRESS_STRIDE`] is, and so is the first after a total becomes known or unknown. The run's
/// last report is always kept ([`Report::progress`]).
pub const MAX_PROGRESS_REPORTS: usize = 2000;

/// Past [`MAX_PROGRESS_REPORTS`], how many changed reports go by for each one passed on.
pub const PROGRESS_STRIDE: usize = 1000;

/// What `vc_log` returns: the line was written.
pub const LOG_WRITTEN: i32 = 0;
/// What `vc_log` returns: the run's log budget is spent, so the line was dropped.
pub const LOG_DROPPED: i32 = 1;
/// What `vc_log` returns: the text doesn't lie inside the module's memory.
pub const LOG_OUTSIDE_MEMORY: i32 = -1;
/// What `vc_log` returns: the text is not UTF-8.
pub const LOG_NOT_UTF8: i32 = -2;
/// What `vc_log` returns: the level is not 0 to 3.
pub const LOG_BAD_LEVEL: i32 = -3;

/// What a storage import that writes or removes returns when it did.
pub const STORAGE_DONE: i32 = 0;
/// What a storage import returns: the key or entry has no value.
pub const STORAGE_NOT_FOUND: i32 = -1;
/// What a storage import returns: a key, value or buffer doesn't lie inside the module's memory.
pub const STORAGE_OUTSIDE_MEMORY: i32 = -2;
/// What a storage import returns: the key is not a [`crate::storage::valid_name`].
pub const STORAGE_BAD_NAME: i32 = -3;
/// What a storage import returns: the write would pass a cap ([`crate::storage`]); nothing changed.
pub const STORAGE_FULL: i32 = -4;
/// What a storage import returns: there is nowhere to keep anything (no plug-in store, as on the
/// web, or a module that isn't installed).
pub const STORAGE_UNAVAILABLE: i32 = -5;
/// What a storage import returns: reading or writing the file failed.
pub const STORAGE_FAILED: i32 = -6;
/// What a storage import returns: the run used up its storage calls or bytes
/// ([`MAX_STORAGE_CALLS`], [`MAX_STORAGE_BYTES`]).
pub const STORAGE_BUDGET: i32 = -7;

/// Most storage calls one run makes; later ones return [`STORAGE_BUDGET`].
pub const MAX_STORAGE_CALLS: usize = 10_000;

/// Most bytes one run's storage calls read and write, all together, counting the whole settings
/// file each time it is read or written and each cache entry read or written; a call that would
/// pass it returns [`STORAGE_BUDGET`].
pub const MAX_STORAGE_BYTES: usize = 1 << 30;

/// One host import: its name, its type and the permission it needs.
pub struct Import {
    pub name: &'static str,
    params: &'static [ValType],
    results: &'static [ValType],
    /// Its type as WebAssembly text writes it.
    pub signature: &'static str,
    pub permission: Permission,
}

const I32: ValType = ValType::I32;

/// An import that takes `params` i32 arguments and returns an i32.
const fn storage_import(name: &'static str, params: &'static [ValType], signature: &'static str, permission: Permission) -> Import {
    Import { name, params, results: &[I32], signature, permission }
}

/// The host imports of API level 2, in module [`MODULE`]. With `progress`:
///
/// - `vc_progress(done: i64, total: i64)`: `done` of `total` steps are finished (`total` 0 or less:
///   not known). Negative values count as 0 and `done` is clamped to `total`;
/// - `vc_cancelled() -> i32`: 1 once the user cancelled the run or its time budget ran out, else 0;
/// - `vc_log(level: i32, ptr: i32, len: i32) -> i32`: writes the UTF-8 text at `ptr` as a log line
///   of `level` (0 error, 1 warning, 2 information, 3 debug) → [`LOG_WRITTEN`], [`LOG_DROPPED`],
///   [`LOG_OUTSIDE_MEMORY`], [`LOG_NOT_UTF8`] or [`LOG_BAD_LEVEL`].
///
/// With `settings`, for the key of `key_len` bytes at `key`:
///
/// - `vc_settings_get(key, key_len, out, out_cap) -> i32`: the value's length, its bytes copied to
///   `out` when they fit in `out_cap` (nothing is copied otherwise);
/// - `vc_settings_set(key, key_len, value, value_len) -> i32`: sets it → [`STORAGE_DONE`];
/// - `vc_settings_delete(key, key_len) -> i32`: removes it → [`STORAGE_DONE`].
///
/// With `cache`, the same for the entry named at `name`: `vc_cache_read`, `vc_cache_write` and
/// `vc_cache_clear(name, name_len)`, which removes the entry, or every entry when `name_len` is 0.
///
/// A storage call that fails returns [`STORAGE_NOT_FOUND`], [`STORAGE_OUTSIDE_MEMORY`],
/// [`STORAGE_BAD_NAME`], [`STORAGE_FULL`], [`STORAGE_UNAVAILABLE`], [`STORAGE_FAILED`] or
/// [`STORAGE_BUDGET`].
pub const IMPORTS: [Import; 9] = [
    Import {
        name: "vc_progress",
        params: &[ValType::I64, ValType::I64],
        results: &[],
        signature: "(param i64 i64)",
        permission: Permission::Progress,
    },
    Import { name: "vc_cancelled", params: &[], results: &[I32], signature: "(result i32)", permission: Permission::Progress },
    Import {
        name: "vc_log",
        params: &[I32, I32, I32],
        results: &[I32],
        signature: "(param i32 i32 i32) (result i32)",
        permission: Permission::Progress,
    },
    storage_import("vc_settings_get", &[I32, I32, I32, I32], "(param i32 i32 i32 i32) (result i32)", Permission::Settings),
    storage_import("vc_settings_set", &[I32, I32, I32, I32], "(param i32 i32 i32 i32) (result i32)", Permission::Settings),
    storage_import("vc_settings_delete", &[I32, I32], "(param i32 i32) (result i32)", Permission::Settings),
    storage_import("vc_cache_read", &[I32, I32, I32, I32], "(param i32 i32 i32 i32) (result i32)", Permission::Cache),
    storage_import("vc_cache_write", &[I32, I32, I32, I32], "(param i32 i32 i32 i32) (result i32)", Permission::Cache),
    storage_import("vc_cache_clear", &[I32, I32], "(param i32 i32) (result i32)", Permission::Cache),
];

/// Which of a plug-in's stores a storage import reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Area {
    Settings,
    Cache,
}

impl Area {
    /// The largest value one key or entry holds.
    fn max_value(self) -> usize {
        match self {
            Area::Settings => storage::MAX_SETTING_BYTES,
            Area::Cache => storage::MAX_CACHE_ENTRY,
        }
    }
}

/// What a storage import returns for `e`.
fn storage_code(e: &StorageError) -> i32 {
    match e {
        StorageError::Full => STORAGE_FULL,
        StorageError::Unavailable => STORAGE_UNAVAILABLE,
        StorageError::Io(_) => STORAGE_FAILED,
        StorageError::Budget => STORAGE_BUDGET,
    }
}

/// A log line's level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    /// The level `vc_log`'s `level` names.
    fn from_code(code: i32) -> Option<LogLevel> {
        match code {
            0 => Some(LogLevel::Error),
            1 => Some(LogLevel::Warn),
            2 => Some(LogLevel::Info),
            3 => Some(LogLevel::Debug),
            _ => None,
        }
    }

    fn log_level(self) -> log::Level {
        match self {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
        }
    }
}

/// A line a plug-in wrote with `vc_log`: its control and bidirectional formatting characters
/// escaped ([`permissions::display_text`]), and cut to [`MAX_LOG_LINE`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub level: LogLevel,
    pub text: String,
}

/// The text of the line that stands for the lines a run's log budget dropped.
pub const LOG_TRUNCATED: &str = "log truncated: later lines were dropped";

/// What a run reported through the host imports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Its last progress report, clamped: `(done, total)`, `total` 0 when not known.
    pub progress: Option<(u64, u64)>,
    /// The lines it wrote, then [`LOG_TRUNCATED`] once its budget dropped one.
    pub lines: Vec<LogLine>,
    /// Did its log budget drop a line?
    pub truncated: bool,
}

/// When a run must stop (native: a wall-clock deadline).
#[derive(Clone, Copy)]
pub(crate) struct Deadline {
    #[cfg(not(target_arch = "wasm32"))]
    at: std::time::Instant,
}

impl Deadline {
    pub(crate) fn new(wall_time_ms: u64) -> Self {
        #[cfg(target_arch = "wasm32")]
        let _ = wall_time_ms;
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            at: std::time::Instant::now() + std::time::Duration::from_millis(wall_time_ms),
        }
    }

    pub(crate) fn passed(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if std::time::Instant::now() >= self.at {
            return true;
        }
        false
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.passed() { Err(Error::Limit("time budget".into())) } else { Ok(()) }
    }
}

/// One run's state that the host imports read and write.
pub(crate) struct Run {
    /// The plug-in's id, which its log lines carry.
    id: String,
    /// Are its progress reports passed on ([`vectorcraft_doc::cancel::progress`])? A live
    /// effect's are not: it is drawn as part of something else, such as an export.
    forward: bool,
    /// Are its log lines written to the app's log? Not while the module is being loaded.
    logged: bool,
    deadline: Deadline,
    /// The last report passed on, as per mille of a known total (with 1) or the count of an
    /// unknown one (with 0), how many were, and how many changed ones went by since the last.
    last: Option<(u64, u64)>,
    reports: usize,
    skipped: usize,
    /// Bytes of log text written (with their escapes) and read.
    log_bytes: usize,
    log_read: usize,
    /// Where the plug-in keeps its settings and cache (none while the module is being loaded), and
    /// how many storage calls the run made and bytes they moved.
    storage: Option<Arc<dyn Storage>>,
    storage_calls: usize,
    storage_bytes: usize,
    pub(crate) report: Report,
}

impl Run {
    pub(crate) fn new(id: &str, forward: bool, logged: bool, deadline: Deadline) -> Self {
        Run {
            id: id.to_string(),
            forward,
            logged,
            deadline,
            last: None,
            reports: 0,
            skipped: 0,
            log_bytes: 0,
            log_read: 0,
            storage: None,
            storage_calls: 0,
            storage_bytes: 0,
            report: Report::default(),
        }
    }

    /// The run, with the plug-in's settings and cache kept in `storage`.
    pub(crate) fn with_storage(mut self, storage: Option<Arc<dyn Storage>>) -> Self {
        self.storage = storage;
        self
    }

    /// Makes one storage call with `call`, on the plug-in's storage and a [`Meter`] holding the
    /// run's storage bytes left, and counts the call and the bytes it read and wrote → the code for
    /// a run that has used up its calls or has no storage, else what `call` returns.
    fn metered<T>(&mut self, call: impl FnOnce(&dyn Storage, &mut Meter) -> std::result::Result<T, StorageError>) -> std::result::Result<T, i32> {
        if self.storage_calls >= MAX_STORAGE_CALLS {
            return Err(STORAGE_BUDGET);
        }
        self.storage_calls += 1;
        let Some(store) = self.storage.clone() else { return Err(STORAGE_UNAVAILABLE) };
        let mut meter = Meter::new(MAX_STORAGE_BYTES.saturating_sub(self.storage_bytes) as u64);
        let done = call(store.as_ref(), &mut meter);
        // What the call read before it failed counts too.
        self.storage_bytes = self.storage_bytes.saturating_add(usize::try_from(meter.used()).unwrap_or(usize::MAX));
        done.map_err(|e| storage_code(&e))
    }

    /// The key or entry name of `len` bytes at `ptr` in `memory`, checked, or the code a storage
    /// import returns for it.
    fn storage_name(memory: &[u8], ptr: usize, len: usize) -> std::result::Result<String, i32> {
        let bytes = ptr.checked_add(len).and_then(|end| memory.get(ptr..end)).ok_or(STORAGE_OUTSIDE_MEMORY)?;
        let name = std::str::from_utf8(bytes).ok().filter(|n| storage::valid_name(n)).ok_or(STORAGE_BAD_NAME)?;
        Ok(name.to_string())
    }

    /// `vc_settings_get` and `vc_cache_read`: copies the value of the key at `name` into the
    /// `out_cap` bytes at `out` when it fits → its length.
    fn storage_read(&mut self, area: Area, memory: &mut [u8], name: (usize, usize), out: (usize, usize)) -> i32 {
        let key = match Self::storage_name(memory, name.0, name.1) {
            Ok(key) => key,
            Err(code) => return code,
        };
        if out.1 > 0 && out.0.checked_add(out.1).is_none_or(|end| end > memory.len()) {
            return STORAGE_OUTSIDE_MEMORY;
        }
        // The storage counts what it read whether or not the value fits the buffer.
        let got = self.metered(|store, meter| match area {
            Area::Settings => store.setting(&key, meter),
            Area::Cache => store.cache_entry(&key, meter),
        });
        let value = match got {
            Ok(Some(value)) => value,
            Ok(None) => return STORAGE_NOT_FOUND,
            Err(code) => return code,
        };
        let Ok(len) = i32::try_from(value.len()) else { return STORAGE_FAILED };
        if value.len() <= out.1
            && let Some(dest) = memory.get_mut(out.0..out.0 + value.len())
        {
            dest.copy_from_slice(&value);
        }
        len
    }

    /// `vc_settings_set` and `vc_cache_write`: sets the key at `name` to the `value` bytes.
    fn storage_write(&mut self, area: Area, memory: &[u8], name: (usize, usize), value: (usize, usize)) -> i32 {
        let key = match Self::storage_name(memory, name.0, name.1) {
            Ok(key) => key,
            Err(code) => return code,
        };
        let Some(bytes) = value.0.checked_add(value.1).and_then(|end| memory.get(value.0..end)) else { return STORAGE_OUTSIDE_MEMORY };
        if bytes.len() > area.max_value() {
            return STORAGE_FULL;
        }
        let written = self.metered(|store, meter| match area {
            Area::Settings => store.set_setting(&key, bytes, meter),
            Area::Cache => store.set_cache_entry(&key, bytes, meter),
        });
        written.map_or_else(|code| code, |()| STORAGE_DONE)
    }

    /// `vc_settings_delete` and `vc_cache_clear`: removes the key at `name`, or with `vc_cache_clear`
    /// and a `name` of no bytes, every cache entry.
    fn storage_remove(&mut self, area: Area, memory: &[u8], name: (usize, usize)) -> i32 {
        let key = match (area, name.1) {
            (Area::Cache, 0) => None,
            _ => match Self::storage_name(memory, name.0, name.1) {
                Ok(key) => Some(key),
                Err(code) => return code,
            },
        };
        let removed = self.metered(|store, meter| match (area, key) {
            (Area::Settings, Some(key)) => store.delete_setting(&key, meter),
            (Area::Cache, Some(key)) => store.remove_cache_entry(&key),
            (_, None) => store.clear_cache().map(|_| true),
        });
        match removed {
            Ok(true) => STORAGE_DONE,
            Ok(false) => STORAGE_NOT_FOUND,
            Err(code) => code,
        }
    }

    fn progress(&mut self, done: i64, total: i64) {
        let total = u64::try_from(total).unwrap_or(0);
        let done = u64::try_from(done).unwrap_or(0);
        let done = if total > 0 { done.min(total) } else { done };
        self.report.progress = Some((done, total));
        let key = if total > 0 { ((u128::from(done) * 1000 / u128::from(total)) as u64, 1) } else { (done, 0) };
        if !self.forward || self.last == Some(key) {
            return;
        }
        let switched = self.last.is_some_and(|(_, t)| (t > 0) != (total > 0));
        if self.reports >= MAX_PROGRESS_REPORTS && !switched {
            self.skipped += 1;
            if self.skipped < PROGRESS_STRIDE {
                return;
            }
        }
        self.skipped = 0;
        self.last = Some(key);
        self.reports += 1;
        vectorcraft_doc::cancel::progress(done, total, PROGRESS_STEP);
    }

    fn cancelled(&self) -> i32 {
        i32::from(vectorcraft_doc::cancel::is_cancelled() || self.deadline.passed())
    }

    /// Writes the `len` bytes at `ptr` in `memory` as a line of `level` → what `vc_log` returns.
    fn log(&mut self, level: i32, memory: &[u8], ptr: usize, len: usize) -> i32 {
        let Some(level) = LogLevel::from_code(level) else { return LOG_BAD_LEVEL };
        let Some(bytes) = ptr.checked_add(len).and_then(|end| memory.get(ptr..end)) else { return LOG_OUTSIDE_MEMORY };
        if self.report.truncated {
            return LOG_DROPPED;
        }
        if self.log_read.saturating_add(len) > MAX_LOG_READ {
            return self.truncate();
        }
        self.log_read += len;
        let Ok(text) = std::str::from_utf8(bytes) else { return LOG_NOT_UTF8 };
        // A character the cut splits is left out.
        let head = text.get(..text.floor_char_boundary(MAX_LOG_LINE)).unwrap_or(text);
        let shown = if head.len() < text.len() {
            let mut shown = permissions::display_text(head, MAX_LOG_LINE - 1);
            if !shown.ends_with('…') {
                shown.push('…');
            }
            shown
        } else {
            permissions::display_text(head, MAX_LOG_LINE)
        };
        if self.report.lines.len() >= MAX_LOG_LINES || self.log_bytes + shown.len() > MAX_LOG_BYTES {
            return self.truncate();
        }
        self.log_bytes += shown.len();
        self.write(level, shown);
        LOG_WRITTEN
    }

    /// Drops the line and the run's later ones, after one line that says so → [`LOG_DROPPED`].
    fn truncate(&mut self) -> i32 {
        self.report.truncated = true;
        self.write(LogLevel::Warn, LOG_TRUNCATED.to_string());
        LOG_DROPPED
    }

    fn write(&mut self, level: LogLevel, text: String) {
        if self.logged {
            log::log!(target: "vectorcraft_plugins", level.log_level(), "{}: {text}", self.id);
        }
        self.report.lines.push(LogLine { level, text });
    }
}

/// What a store holds: its memory limits and the run's state.
pub(crate) struct Host {
    pub(crate) limits: StoreLimits,
    pub(crate) run: Run,
}

/// The module's memory and the store's state, for an import that reads or writes the memory.
fn memory_and_host<'a>(caller: &'a mut Caller<'_, Host>) -> Option<(&'a mut [u8], &'a mut Host)> {
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else { return None };
    Some(memory.data_and_store_mut(caller))
}

/// An i32 argument as the unsigned offset or length it stands for.
fn offset(v: i32) -> usize {
    v as u32 as usize
}

/// A linker that offers the host imports whose permission is in `granted`, and no other import.
pub(crate) fn linker(engine: &Engine, granted: &[Permission]) -> Linker<Host> {
    let mut linker = Linker::<Host>::new(engine);
    // Each name is defined once, so none of these fails.
    if granted.contains(&Permission::Progress) {
        let _ = linker.func_wrap(MODULE, "vc_progress", |mut caller: Caller<'_, Host>, done: i64, total: i64| {
            caller.data_mut().run.progress(done, total);
        });
        let _ = linker.func_wrap(MODULE, "vc_cancelled", |caller: Caller<'_, Host>| -> i32 { caller.data().run.cancelled() });
        let _ = linker.func_wrap(MODULE, "vc_log", |mut caller: Caller<'_, Host>, level: i32, ptr: i32, len: i32| -> i32 {
            let Some((data, host)) = memory_and_host(&mut caller) else { return LOG_OUTSIDE_MEMORY };
            host.run.log(level, data, offset(ptr), offset(len))
        });
    }
    for (area, permission, get, set, remove) in [
        (Area::Settings, Permission::Settings, "vc_settings_get", "vc_settings_set", "vc_settings_delete"),
        (Area::Cache, Permission::Cache, "vc_cache_read", "vc_cache_write", "vc_cache_clear"),
    ] {
        if !granted.contains(&permission) {
            continue;
        }
        let _ = linker.func_wrap(MODULE, get, move |mut caller: Caller<'_, Host>, key: i32, key_len: i32, out: i32, out_cap: i32| -> i32 {
            let Some((data, host)) = memory_and_host(&mut caller) else { return STORAGE_OUTSIDE_MEMORY };
            host.run.storage_read(area, data, (offset(key), offset(key_len)), (offset(out), offset(out_cap)))
        });
        let _ = linker.func_wrap(MODULE, set, move |mut caller: Caller<'_, Host>, key: i32, key_len: i32, value: i32, value_len: i32| -> i32 {
            let Some((data, host)) = memory_and_host(&mut caller) else { return STORAGE_OUTSIDE_MEMORY };
            host.run.storage_write(area, data, (offset(key), offset(key_len)), (offset(value), offset(value_len)))
        });
        let _ = linker.func_wrap(MODULE, remove, move |mut caller: Caller<'_, Host>, key: i32, key_len: i32| -> i32 {
            let Some((data, host)) = memory_and_host(&mut caller) else { return STORAGE_OUTSIDE_MEMORY };
            host.run.storage_remove(area, data, (offset(key), offset(key_len)))
        });
    }
    linker
}

/// Checks that every import of `module` is a host import with the host's signature → them. Any
/// other import is refused, and the error names it.
pub(crate) fn check_imports(module: &Module) -> Result<Vec<&'static Import>> {
    let mut found = Vec::new();
    for imp in module.imports() {
        let offered = if imp.module() == MODULE { IMPORTS.iter().find(|i| i.name == imp.name()) } else { None };
        let (from, name) = (permissions::display_text(imp.module(), 64), permissions::display_text(imp.name(), 64));
        let Some(host) = offered else {
            return Err(Error::Abi(format!(
                "the module imports `{from}.{name}`, which the host doesn't offer: a plug-in may import only the `env.vc_*` functions that docs/plugins.md lists (plug-in API level 2), and no WASI, files or network"
            )));
        };
        match imp.ty() {
            ExternType::Func(f) if f.params() == host.params && f.results() == host.results => found.push(host),
            _ => {
                let signature = host.signature;
                return Err(Error::Abi(format!("the module imports `env.{name}` with another type than the host's: it is {signature}")));
            }
        }
    }
    Ok(found)
}

/// Checks that `manifest` allows the host imports `imports`: `api` 2 and each import's permission.
/// The error names the first import it doesn't allow.
pub(crate) fn check_granted(manifest: &Manifest, imports: &[&Import]) -> Result<()> {
    let granted = manifest.permissions();
    match imports.iter().find(|i| manifest.api < PROGRESS_API || !granted.contains(&i.permission)) {
        Some(import) => {
            let (name, permission) = (import.name, import.permission.name());
            Err(Error::Abi(format!(
                "the module imports `env.{name}`, which a plug-in may use only at plug-in API level {PROGRESS_API} (`api` {PROGRESS_API}) and with the permission `{permission}`"
            )))
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> Run {
        Run::new("test", true, false, Deadline::new(60_000))
    }

    /// Progress is clamped: negative counts are 0 and `done` never passes a known total.
    #[test]
    fn progress_is_clamped() {
        let mut r = run();
        for ((done, total), want) in
            [((5, 3), (3, 3)), ((-4, 10), (0, 10)), ((7, -1), (7, 0)), ((i64::MAX, i64::MAX), (i64::MAX as u64, i64::MAX as u64))]
        {
            r.progress(done, total);
            assert_eq!(r.report.progress, Some(want), "{done} of {total}");
        }
    }

    /// Reports are passed on only when they change (with a known total, to another tenth of a
    /// percent, however the total changes; with an unknown total, each change of `done`), and past
    /// [`MAX_PROGRESS_REPORTS`] once per [`PROGRESS_STRIDE`] changes, except the first after the
    /// total becomes known; the last one is always kept.
    #[test]
    fn progress_is_passed_on_at_a_bounded_rate() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink: vectorcraft_doc::cancel::ProgressSink = {
            let seen = seen.clone();
            Rc::new(move |d, t, w| seen.borrow_mut().push((d, t, w.to_string())))
        };
        let token = vectorcraft_doc::cancel::CancelToken::new();
        let mut r = run();
        vectorcraft_doc::cancel::scope(&token, Some(sink), || {
            for done in 0..=1_000_000 {
                r.progress(done, 1_000_000);
            }
        });
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1001, "one per tenth of a percent");
        assert_eq!(seen.last(), Some(&(1_000_000, 1_000_000, PROGRESS_STEP.to_string())));
        let mut unknown = run();
        let later = Rc::new(RefCell::new(Vec::new()));
        let sink: vectorcraft_doc::cancel::ProgressSink = {
            let later = later.clone();
            Rc::new(move |d, t, _| later.borrow_mut().push((d, t)))
        };
        vectorcraft_doc::cancel::scope(&token, Some(sink), || {
            for done in 0..10_000 {
                unknown.progress(done, 0);
            }
            assert_eq!(unknown.reports, MAX_PROGRESS_REPORTS + (10_000 - MAX_PROGRESS_REPORTS) / PROGRESS_STRIDE);
            assert_eq!(unknown.report.progress, Some((9_999, 0)), "the last report is kept");
            // A total that becomes known is passed on at once, and the reports after it keep
            // coming at the slower rate.
            unknown.progress(10, 100);
            assert_eq!(later.borrow().last(), Some(&(10, 100)));
            for done in 11..=100 {
                unknown.progress(done, 100);
            }
        });
        assert_eq!(later.borrow().last(), Some(&(10, 100)), "90 changes are fewer than the stride");
        assert_eq!(unknown.report.progress, Some((100, 100)));
        // A new total at the same tenth of a percent is not a new step.
        let mut retotalled = run();
        let steps = Rc::new(RefCell::new(Vec::new()));
        let sink: vectorcraft_doc::cancel::ProgressSink = {
            let steps = steps.clone();
            Rc::new(move |d, t, _| steps.borrow_mut().push((d, t)))
        };
        vectorcraft_doc::cancel::scope(&token, Some(sink), || {
            for total in 10..2_010 {
                retotalled.progress(total / 10, total - total % 10);
            }
        });
        assert_eq!(*steps.borrow(), [(1, 10)]);
        assert_eq!(retotalled.report.progress, Some((200, 2_000)));
    }

    /// Text outside memory, text that isn't UTF-8 and an unknown level are errors; a long line is
    /// cut, also inside a character, and hidden characters are escaped.
    #[test]
    fn log_text_is_checked_and_cut() {
        let mut r = run();
        let memory = b"hello\xff\xfe\nworld";
        assert_eq!(r.log(2, memory, 0, 5), LOG_WRITTEN);
        assert_eq!(r.log(2, memory, 0, memory.len() + 1), LOG_OUTSIDE_MEMORY);
        assert_eq!(r.log(2, memory, usize::MAX, 2), LOG_OUTSIDE_MEMORY);
        assert_eq!(r.log(2, memory, 4, 3), LOG_NOT_UTF8);
        assert_eq!(r.log(4, memory, 0, 5), LOG_BAD_LEVEL);
        assert_eq!(r.log(-1, memory, 0, 5), LOG_BAD_LEVEL);
        assert_eq!(r.log(0, memory, 7, 6), LOG_WRITTEN);
        // The cut falls inside the 512th "é", which is left out.
        let long = format!("a{}", "é".repeat(MAX_LOG_LINE));
        assert_eq!(r.log(1, long.as_bytes(), 0, long.len()), LOG_WRITTEN);
        let lines = &r.report.lines;
        assert_eq!(lines[0], LogLine { level: LogLevel::Info, text: "hello".into() });
        assert_eq!(lines[1], LogLine { level: LogLevel::Error, text: "\\u{000A}world".into() });
        assert_eq!(lines[2], LogLine { level: LogLevel::Warn, text: format!("a{}…", "é".repeat(MAX_LOG_LINE / 2 - 1)) });
        assert!(!r.report.truncated);
        // Text past the cut is checked too.
        let bad_tail = [vec![b'a'; MAX_LOG_LINE * 2], vec![0xff]].concat();
        assert_eq!(r.log(2, &bad_tail, 0, bad_tail.len()), LOG_NOT_UTF8);
        // A cut line of escapes is at most MAX_LOG_LINE characters, its ellipsis included.
        let controls = vec![1u8; MAX_LOG_LINE * 2];
        assert_eq!(r.log(2, &controls, 0, controls.len()), LOG_WRITTEN);
        let text = &r.report.lines[3].text;
        assert!(text.starts_with("\\u{0001}") && text.ends_with('…') && text.chars().count() <= MAX_LOG_LINE, "{text}");
    }

    /// The byte budget counts a line as written, with its escapes, so short lines of hidden
    /// characters can't write more than [`MAX_LOG_BYTES`].
    #[test]
    fn log_budget_counts_escaped_text() {
        let mut r = run();
        let controls = [1u8; 100];
        let shown = "\\u{0001}".len() * controls.len();
        let fit = MAX_LOG_BYTES / shown;
        for i in 0..fit {
            assert_eq!(r.log(2, &controls, 0, controls.len()), LOG_WRITTEN, "line {i}");
        }
        assert_eq!(r.log(2, &controls, 0, controls.len()), LOG_DROPPED);
        let written: usize = r.report.lines.iter().filter(|l| l.text != LOG_TRUNCATED).map(|l| l.text.len()).sum();
        assert!(written <= MAX_LOG_BYTES, "{written}");
    }

    /// The host reads at most [`MAX_LOG_READ`] bytes of log text in a run, so long texts can't
    /// make it check more; once past it, later calls are dropped without reading.
    #[test]
    fn log_text_read_is_bounded() {
        let mut r = run();
        let big = vec![b'a'; MAX_LOG_READ / 4];
        for _ in 0..4 {
            assert_eq!(r.log(2, &big, 0, big.len()), LOG_WRITTEN);
        }
        assert_eq!(r.log(2, b"x", 0, 1), LOG_DROPPED);
        assert!(r.report.truncated);
        let mut huge = run();
        let over = vec![0xffu8; MAX_LOG_READ + 1];
        assert_eq!(huge.log(2, &over, 0, over.len()), LOG_DROPPED, "dropped before it is checked");
        assert_eq!(huge.log(2, b"x", 0, 1), LOG_DROPPED);
    }

    /// Lines past the run's budget are dropped, with one line that says so.
    #[test]
    fn log_lines_past_the_budget_are_dropped_with_one_truncation_line() {
        let mut r = run();
        for i in 0..MAX_LOG_LINES + 50 {
            let want = if i < MAX_LOG_LINES { LOG_WRITTEN } else { LOG_DROPPED };
            assert_eq!(r.log(2, b"line", 0, 4), want, "line {i}");
        }
        assert_eq!(r.report.lines.len(), MAX_LOG_LINES + 1);
        assert_eq!(r.report.lines.last(), Some(&LogLine { level: LogLevel::Warn, text: LOG_TRUNCATED.into() }));
        assert!(r.report.truncated);
        // So is text past the run's bytes.
        let mut bytes = run();
        let big = vec![b'x'; MAX_LOG_LINE];
        let fit = MAX_LOG_BYTES / MAX_LOG_LINE;
        for _ in 0..fit {
            assert_eq!(bytes.log(2, &big, 0, big.len()), LOG_WRITTEN);
        }
        assert_eq!(bytes.log(2, b"x", 0, 1), LOG_DROPPED);
        assert_eq!(bytes.report.lines.len(), fit + 1);
    }
}
