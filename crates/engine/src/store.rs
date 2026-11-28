//! The user's own PDF presets, ICC profiles and plug-ins, kept in folders next to the preferences
//! so that they outlive the process (#2168, #2174, #2541). The apps decide where
//! ([`Session::attach_pdf_preset_store`], [`Session::attach_profile_store`],
//! [`Session::attach_plugin_store`]); a session nobody attached a folder to keeps nothing, which
//! is how the engine's own tests and embedders run.
//!
//! * **PDF presets**: one `.vcpdfpresets` file per preset (the format `pdf.preset.export` writes,
//!   so a file can also be imported by hand), named after the preset. A session that attaches the
//!   folder replaces its saved presets with the ones in it; a folder that doesn't exist yet takes
//!   the presets the session has (the ones an older version kept in the preferences file).
//! * **ICC profiles**: the bytes of each profile `color.loadProfile` registered, as `.icc` files
//!   named after the profile, never the path they came from (a profile in a system folder may
//!   move or go away). Attaching the folder registers them again.
//! * **Color Settings**: the working spaces, intent and black point compensation
//!   `edit.colorSettings` sets, as one JSON file (#6335). Attaching it, after the profiles, makes
//!   them the active settings again.
//! * **Plug-ins**: the module of each plug-in `plugin.install` installs, as a `.wasm` file named
//!   after its id (as Illustrator keeps plug-ins in its Plug-ins folder); `plugin.remove` deletes it.
//!   Attaching the folder installs them again, except in Safe Mode and except the ones the user
//!   disabled, which `disabled.json` in the same folder lists (`plugin.setEnabled`). The Additional
//!   Plug-ins Folder preference is not copied: it is loaded from where it is. `grants.json` holds what the user granted the plug-ins
//!   stored there that list permissions (id, module hash, permissions); a stored plug-in whose
//!   permissions it doesn't grant is skipped with a log record, and stays in the folder. The file
//!   carries a generation and a MAC of its generation, grants and folder, made with a key the user's
//!   key store keeps ([`grant_keys`]); a file without that MAC, or at another generation than the
//!   one the key store keeps for the folder, grants nothing.
//! * **Plug-in settings and caches**: `storage/id-<id>/` in the plug-in store, the id written as
//!   [`case_safe`] writes it, holds what a plug-in
//!   keeps with the `settings` and `cache` permissions ([`PluginStorage`]): `settings.json` and a
//!   `cache` folder with one file per entry. The host names every file; a plug-in only names keys.
//!   Replacing a plug-in keeps them, and `plugin.remove` deletes them.
//! * **Turns on the plug-in store**: a window or process locks `disabled.lock` in the plug-in folder
//!   ([`StoreTurn`]) while it reads and changes the stored plug-ins, `grants.json`, `disabled.json`
//!   and the plug-ins' settings and caches, so that two of them using one folder change it one after
//!   the other: an install's module and grant are written in one turn, and a grant written for one
//!   plug-in doesn't undo another one's.
//!
//! The files are untrusted input like any other: sizes and counts are capped, a file that does not
//! parse is skipped with a log record, and every write is atomic.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use serde_json::Value;
use vectorcraft_pdf::{APP, PdfPreset, builtin_preset};
use vectorcraft_plugins::loader::{Job, Loads};
use vectorcraft_plugins::permissions::GrantToken;
use vectorcraft_plugins::safe_mode;
use vectorcraft_plugins::storage::{self, Meter, Storage, StorageError};

use crate::Session;
use crate::cmd::pdfcmds::{PRESET_FORMAT, PresetFile};

pub mod grant_keys;

/// Files read from one store folder, at most (the rest are ignored).
pub const MAX_STORE_FILES: usize = 1000;
/// The largest PDF presets file read (a preset is a few kilobytes).
pub const MAX_PRESET_FILE: u64 = 1 << 20;
/// The largest ICC profile file read or loaded (CMYK profiles with large tables are a few MB).
pub const MAX_PROFILE_FILE: u64 = 32 << 20;
/// The most stored ICC profile bytes read at startup, all files together (the rest are skipped).
pub const MAX_PROFILE_STORE: u64 = 256 << 20;
/// The extension of a stored ICC profile.
const PROFILE_EXT: &str = "icc";
/// The most stored plug-in bytes read at startup, all files together (the rest are skipped).
pub const MAX_PLUGIN_STORE: u64 = 512 << 20;
/// The extension of a stored plug-in.
const PLUGIN_EXT: &str = "wasm";
/// The file in the plug-in folder that lists the plug-ins the user disabled.
pub const DISABLED_FILE: &str = "disabled.json";
/// The file in the plug-in folder that a window locks while it reads and changes the plug-in store
/// ([`StoreTurn`]), [`DISABLED_FILE`] included.
pub const DISABLED_LOCK: &str = "disabled.lock";
/// The file in the plug-in store that holds the permissions the user granted.
const GRANTS_FILE: &str = "grants.json";
/// The error of a `plugin.remove` that can't remove the plug-in's grant from [`GRANTS_FILE`].
pub(crate) const GRANT_NOT_REMOVED: &str = "the plug-in's grant can't be removed from the plug-in store";
/// The error of a change to the plug-in store that can't take its turn ([`StoreTurn`]).
pub(crate) const STORE_NOT_LOCKED: &str = "the plug-in store can't be locked";
/// The largest grants file read.
const MAX_GRANTS_FILE: u64 = 1 << 20;
/// The folder in the plug-in store that holds each plug-in's settings and cache.
const PLUGIN_DATA_DIR: &str = "storage";
/// A plug-in's settings file, in its folder.
const SETTINGS_FILE: &str = "settings.json";
/// Where a settings file that doesn't read is moved, replacing the one moved there before.
const BAD_SETTINGS_FILE: &str = "settings.unreadable.json";
/// The folder of a plug-in's cache entries, in its folder.
const CACHE_DIR: &str = "cache";
/// The extension of a cache entry's file.
const CACHE_EXT: &str = "bin";
/// Items in a cache folder a write looks at, at most: the entries, and temporary files a write left.
/// A write to a folder that holds more is refused as full.
pub(crate) const MAX_CACHE_SCAN: usize = storage::MAX_CACHE_ENTRIES * 2;
/// Held while a plug-in's settings or cache change, so two runs can't pass a cap together or lose
/// each other's settings. Taken after the plug-in store's turn ([`StoreTurn`]), never before it.
static PLUGIN_DATA_LOCK: Mutex<()> = Mutex::new(());
/// The state of each plug-in folder's storage, shared by every storage made for the folder until
/// the plug-in is removed, which closes it and drops it.
static PLUGIN_DATA_OPEN: Mutex<BTreeMap<PathBuf, Arc<PluginData>>> = Mutex::new(BTreeMap::new());
/// The grants of plug-in stores that the sessions of the process don't count, and when each was
/// last written ([`IgnoredGrants`]). Another process reads `grants.json` as it is.
static IGNORED_PLUGIN_GRANTS: Mutex<IgnoredGrants> = Mutex::new(IgnoredGrants { ignored: BTreeSet::new(), written: BTreeMap::new(), writes: 0 });
/// How many of the latest grant writes [`IgnoredGrants`] keeps.
const MAX_GRANT_WRITES_KEPT: usize = 256;

/// Grants of plug-in stores, by the store's folder ([`plugin_store_key`]) and the plug-in id.
struct IgnoredGrants {
    /// The grants that should be gone but could not be removed: no session in the process counts
    /// them ([`Session::ignore_stored_grant`]) until one writes the store's grants for the id
    /// ([`Session::store_grant_in`]).
    ignored: BTreeSet<(PathBuf, String)>,
    /// The latest grant writes of the process, at most [`MAX_GRANT_WRITES_KEPT`], each with the
    /// count of writes it made ([`IgnoredGrants::writes`]).
    written: BTreeMap<(PathBuf, String), u64>,
    /// How many grant writes the sessions of the process have made ([`Session::grant_writes`]).
    writes: u64,
}

/// The file name (without extension) a stored item called `name` gets: its name made safe for any
/// file system, then a hash of the name, so that names that differ only in characters a file name
/// can't hold, or in case, can't share a file. Names equal in any case are one item.
pub(crate) fn file_stem(name: &str) -> String {
    let safe: String = name.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).take(48).collect();
    // FNV-1a over the lower-cased name: stable across versions and platforms, unlike `DefaultHasher`.
    let hash = name.trim().to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{}-{hash:016x}", safe.trim())
}

/// The folder `dir` spelled one way for every path to it, so that sessions attached to it by
/// different paths share [`IGNORED_PLUGIN_GRANTS`]: the canonical path when the folder exists, else
/// the canonical path of its nearest folder that exists followed by the rest of the path, with `.`
/// and `..` taken out by name. The path as given when it has no absolute form (the current folder
/// can't be read).
pub(crate) fn plugin_store_key(dir: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(dir) {
        return path;
    }
    let Ok(absolute) = std::path::absolute(dir) else { return dir.to_path_buf() };
    let mut parts: Vec<std::path::Component> = Vec::new();
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(parts.last(), Some(std::path::Component::Normal(_))) {
                    parts.pop();
                }
            }
            _ => parts.push(part),
        }
    }
    for kept in (1..=parts.len()).rev() {
        let base: PathBuf = parts[..kept].iter().collect();
        if let Ok(found) = std::fs::canonicalize(&base) {
            return parts[kept..].iter().fold(found, |path, part| path.join(part));
        }
    }
    parts.iter().collect()
}

/// The regular files of `dir` with the extension `ext` (any case), by file name, at most
/// [`MAX_STORE_FILES`]; none when the folder can't be read.
fn list_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) && p.is_file())
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files.truncate(MAX_STORE_FILES);
    files
}

/// [`list_files`], each file with its stamp ([`file_stamp`]) from the same look at it.
fn list_stamped_files(dir: &Path, ext: &str) -> Vec<(PathBuf, (u64, SystemTime))> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).filter_map(|p| stamped_file(p, ext)).collect();
    files.sort_by_key(|(p, _)| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files.truncate(MAX_STORE_FILES);
    files
}

/// The file `p` with its size and modified time, as [`list_stamped_files`] lists it: `None` unless
/// it is a regular file with the extension `ext`.
fn stamped_file(p: PathBuf, ext: &str) -> Option<(PathBuf, (u64, SystemTime))> {
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
        return None;
    }
    let m = std::fs::metadata(&p).ok().filter(std::fs::Metadata::is_file)?;
    match m.modified() {
        Ok(t) => Some((p, (m.len(), t))),
        Err(e) => {
            skip_stored(&p, e);
            None
        }
    }
}

/// The content of `path` when it is at most `max` bytes; the file is never read past `max`.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let bytes = read_past_cap(path, max).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(bytes)
}

/// The first `max + 1` bytes of `path` at most, so that a result longer than `max` shows the file is
/// over its cap.
fn read_past_cap(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = vec![];
    std::fs::File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Plug-ins loading on worker threads, which [`Session::poll_plugin_loads`] and
/// [`Session::finish_plugin_loads`] install.
pub(crate) enum PluginLoad {
    /// The plug-in store's ([`Session::load_stored_plugins`]).
    Stored(StoredLoad),
    /// The Additional Plug-ins Folder's ([`crate::cmd::plugin::sync_prefs`]).
    Folder(crate::cmd::plugin::FolderPending),
}

impl PluginLoad {
    /// Starts the modules loading, unless they started already.
    fn start(&mut self) {
        match self {
            PluginLoad::Stored(load) => load.loads.begin(),
            PluginLoad::Folder(load) => load.start(),
        }
    }

    /// The installs and removals it checks its modules against (`None`: a folder that couldn't be
    /// listed, or one ended already).
    fn changes(&self) -> Option<&vectorcraft_plugins::registry::LoadChanges> {
        match self {
            PluginLoad::Stored(load) => Some(&load.changes),
            PluginLoad::Folder(load) => load.changes(),
        }
    }

    /// [`PluginLoad::changes`], to adopt an earlier load's.
    fn changes_mut(&mut self) -> Option<&mut vectorcraft_plugins::registry::LoadChanges> {
        match self {
            PluginLoad::Stored(load) => Some(&mut load.changes),
            PluginLoad::Folder(load) => load.changes_mut(),
        }
    }

    /// Whether every module has been installed or skipped.
    fn done(&self) -> bool {
        match self {
            PluginLoad::Stored(load) => load.next == load.files.len() && load.rest.is_empty(),
            PluginLoad::Folder(load) => load.done(),
        }
    }

    /// Whether something can be installed or ended now, without waiting.
    pub(crate) fn ready(&mut self) -> bool {
        match self {
            PluginLoad::Stored(load) => load.next == load.files.len() || !load.loaded.is_empty() || load.loads.next_ready(),
            PluginLoad::Folder(load) => load.ready(),
        }
    }

    /// Waits until [`PluginLoad::ready`], starting the modules loading first ([`PluginLoad::start`]).
    fn wait(&mut self) {
        self.start();
        match self {
            PluginLoad::Stored(load) if load.loaded.is_empty() && load.next < load.files.len() => load.loads.wait_next(),
            PluginLoad::Stored(_) => {}
            PluginLoad::Folder(load) => load.wait(),
        }
    }

    /// Drops the modules not installed yet ([`Session::cancel_plugin_loads`]), leaving it
    /// [`PluginLoad::done`] with what it installed.
    fn cancel(&mut self) {
        match self {
            PluginLoad::Stored(load) => {
                load.loads = Loads::deferred(Vec::new());
                load.loaded.clear();
                load.next = load.files.len();
                load.rest.clear();
            }
            PluginLoad::Folder(load) => load.cancel(),
        }
    }

    /// Whether installing it takes the plug-in store's own turn, making the store folder (a
    /// folder's plug-ins keep their settings and cache there), rather than one that goes ahead
    /// without a store folder.
    fn creates_store(&self) -> bool {
        matches!(self, PluginLoad::Folder(_))
    }
}

/// A stored plug-in file, with its stamp when it was listed ([`file_stamp`]; `None`: it couldn't be
/// read).
type StoredFile = (PathBuf, Option<FileStamp>);

/// The stored plug-ins loading from the plug-in store `dir`.
pub(crate) struct StoredLoad {
    dir: PathBuf,
    /// The files listed, in file-name order.
    files: Vec<StoredFile>,
    loads: Loads<StoredRead>,
    /// Outcomes taken from `loads` and not installed yet, in order.
    loaded: VecDeque<vectorcraft_plugins::Result<StoredRead>>,
    /// The ids this load installed, in file-name order.
    installed: Vec<String>,
    /// The next file to install.
    next: usize,
    /// The store's grants this load removed, by id, because the module stored for the id lists no
    /// permissions: a later file holding a module with that id that the grant covers still loads,
    /// and the grant goes back to the store.
    retired: BTreeMap<String, vectorcraft_plugins::permissions::Grant>,
    /// Installs and removals elsewhere since the store was listed, which a module loaded before
    /// them doesn't undo.
    changes: vectorcraft_plugins::registry::LoadChanges,
    /// The files listed after `files`, in file-name order, with their stamps from when the store was
    /// listed, that wait for `files` to load because what is left of [`MAX_PLUGIN_STORE`] may not
    /// cover them until the reads that fail give back what they didn't read
    /// ([`list_stored_plugins`]).
    rest: VecDeque<StoredFile>,
    /// What is left of [`MAX_PLUGIN_STORE`]: the listed length of each file in `files` is taken
    /// from it, and what a failed read didn't read of it is given back.
    left: u64,
}

/// What a stored plug-in's load read of its file, at most its listed length, and the module.
pub(crate) struct StoredRead {
    read: u64,
    plugin: vectorcraft_plugins::Result<vectorcraft_plugins::Plugin>,
}

/// Tests: what runs before a stored module is read, given its path.
#[cfg(test)]
type StoredHook = Arc<dyn Fn(&Path) + Send + Sync>;

/// Tests: runs on the worker thread before the stored module at the path it is given is read.
#[cfg(test)]
pub(crate) static BEFORE_STORED_LOAD: Mutex<Option<StoredHook>> = Mutex::new(None);

#[cfg(test)]
fn before_stored_load(path: &Path) {
    let hook = BEFORE_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(hook) = hook {
        hook(path);
    }
}

/// Tests: runs on the worker thread once the stored module at the path it is given has loaded, and
/// before its file is read again to check that it still holds the module.
#[cfg(test)]
pub(crate) static AFTER_STORED_LOAD: Mutex<Option<StoredHook>> = Mutex::new(None);

#[cfg(test)]
fn after_stored_load(path: &Path) {
    let hook = AFTER_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(hook) = hook {
        hook(path);
    }
}

/// Why a stored plug-in whose file changed between its listing and its install is skipped.
const CHANGED_WHILE_LOADING: &str = "it changed or went away while it loaded";

/// The stored plug-in files that didn't install in this process, by path, with why
/// ([`skip_stored`]). A file stays listed until it installs or its store is loaded again.
static STORED_FAILED: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());

/// The stored plug-ins skipped ([`STORED_FAILED`]).
fn stored_failures() -> MutexGuard<'static, BTreeMap<PathBuf, String>> {
    STORED_FAILED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Skips the stored plug-in `path` because of `why`: logs it and lists the file as failed.
fn skip_stored(path: &Path, why: impl std::fmt::Display) {
    log::warn!("skipping the stored plug-in {}: {why}", path.display());
    stored_failures().insert(path.to_path_buf(), why.to_string());
}

/// The stored plug-in files in the plug-in store `dir` that didn't install in this process, in
/// file-name order, each with why.
pub(crate) fn failed_stored_plugins(dir: &Path) -> Vec<(PathBuf, String)> {
    stored_failures().iter().filter(|(p, _)| p.parent() == Some(dir)).map(|(p, why)| (p.clone(), why.clone())).collect()
}

/// Stops listing the stored plug-in file `path` as failed: a plug-in installed from it.
pub(crate) fn clear_stored_failure(path: &Path) {
    stored_failures().remove(path);
}

/// A plug-in installed from the stored file `path`: it is no longer listed as failed, by the store
/// (a load in another session may have failed on it meanwhile) or by the folder report of a
/// `plugin.reload` of the store's folder.
fn installed_from(path: &Path) {
    clear_stored_failure(path);
    crate::cmd::plugin::clear_folder_failure(path);
}

/// Why the stored plug-in `path` didn't install in this process, if it is listed as failed.
pub(crate) fn stored_failure(path: &Path) -> Option<String> {
    stored_failures().get(path).cloned()
}

#[cfg(test)]
thread_local! {
    /// Tests: the stored plug-in bytes a store listed on this thread may read ([`MAX_PLUGIN_STORE`]).
    pub(crate) static PLUGIN_STORE_BUDGET: std::cell::Cell<u64> = const { std::cell::Cell::new(MAX_PLUGIN_STORE) };
}

/// The stored plug-in bytes one store's load reads, all files together ([`MAX_PLUGIN_STORE`]).
#[cfg(not(test))]
fn plugin_store_budget() -> u64 {
    MAX_PLUGIN_STORE
}

#[cfg(test)]
fn plugin_store_budget() -> u64 {
    PLUGIN_STORE_BUDGET.with(std::cell::Cell::get)
}

/// What a file over its cap is told (a detail after the file's name, in the log or an error).
fn too_large(max: u64) -> String {
    format!("larger than {max} bytes")
}

/// Takes the stored plug-in files at the front of `rest`, in order, each one's listed length taken
/// from `left` → those files, and the jobs that load them, each reading no more than that length.
/// It stops at the first file that `left` doesn't cover but would if files taken before it failed to
/// read and gave back what they took; that file and the ones after it stay in `rest`. A file that
/// `left` wouldn't cover even then is skipped with a log record, and once nothing can be given back
/// to a used-up `left` the rest are, as when the files were read one after the other.
fn list_stored_plugins(rest: &mut VecDeque<StoredFile>, left: &mut u64) -> (Vec<StoredFile>, Vec<Job<StoredRead>>) {
    let limits = vectorcraft_plugins::Limits::default();
    let mut files = Vec::new();
    let mut jobs = Vec::new();
    // What the files taken so far took of `left`.
    let mut taken = 0;
    while let Some((path, stamp)) = rest.pop_front() {
        let len = stamp.flatten().map_or(0, |(len, _)| len);
        let most = *left + taken;
        if most == 0 {
            log::warn!("skipping the other stored plug-ins: they hold more than {MAX_PLUGIN_STORE} bytes together");
            let why = format!("the stored plug-ins hold more than {MAX_PLUGIN_STORE} bytes together");
            let mut failed = stored_failures();
            for (path, _) in std::iter::once((path, stamp)).chain(rest.drain(..)) {
                failed.insert(path, why.clone());
            }
            break;
        }
        if len > most {
            skip_stored(&path, too_large(most));
            continue;
        }
        // An empty file too waits once `left` is used up.
        if len > *left || *left == 0 {
            rest.push_front((path, stamp));
            break;
        }
        *left -= len;
        taken += len;
        let item = safe_mode::Item::file(path.display().to_string());
        let (read, limits) = (path.clone(), limits.clone());
        jobs.push(Job::new(Some(item), move || {
            #[cfg(test)]
            before_stored_load(&read);
            let mut bytes = vec![];
            let got = std::fs::File::open(&read).and_then(|f| f.take(len + 1).read_to_end(&mut bytes));
            let plugin = match got {
                Err(e) => Err(vectorcraft_plugins::Error::Io(e.to_string())),
                Ok(_) if bytes.len() as u64 > len => Err(vectorcraft_plugins::Error::Io(too_large(len))),
                Ok(_) => vectorcraft_plugins::Plugin::load(&bytes, limits),
            };
            #[cfg(test)]
            after_stored_load(&read);
            // A file replaced while its module loaded, by one that kept its length and modified time,
            // is told apart by its content: it is read again here, within the load's deadline. The
            // install then skips a file whose stamp changed since.
            let plugin = plugin.and_then(|p| {
                if std::fs::File::open(&read).is_ok_and(|f| holds(f, &bytes)) {
                    Ok(p)
                } else {
                    Err(vectorcraft_plugins::Error::Io(CHANGED_WHILE_LOADING.into()))
                }
            });
            // A read that failed part way read what `bytes` holds.
            Ok(StoredRead { read: (bytes.len() as u64).min(len), plugin })
        }));
        files.push((path, stamp));
    }
    (files, jobs)
}

/// Whether `old` holds exactly `bytes`. It is read no further than one byte past their length, so a
/// large file is not read into memory.
pub(crate) fn holds(old: impl Read, bytes: &[u8]) -> bool {
    let mut read = vec![];
    old.take(bytes.len() as u64 + 1).read_to_end(&mut read).is_ok() && read == bytes
}

/// Write `bytes` to `dir/file`, making the folder; nothing is written when the file has them
/// already. A failure is logged: remembering is best effort and never fails the command.
fn put(dir: &Path, file: &str, bytes: &[u8]) -> bool {
    let path = dir.join(file);
    if std::fs::File::open(&path).is_ok_and(|old| holds(old, bytes)) {
        return true;
    }
    #[cfg(test)]
    if file == GRANTS_FILE && GRANTS_WRITE_FAILS.get() {
        log::warn!("can't save {}: the test fails it", path.display());
        return false;
    }
    let written = std::fs::create_dir_all(dir).and_then(|()| write_atomic(&path, bytes));
    if let Err(e) = &written {
        log::warn!("can't save {}: {e}", path.display());
    }
    written.is_ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    vectorcraft_format::write_atomic(path, bytes)
}

/// The browser has no folders to keep anything in (the apps there never attach a store).
#[cfg(target_arch = "wasm32")]
fn write_atomic(_: &Path, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file system"))
}

/// [`DISABLED_LOCK`] in the plug-in store `dir`, locked until the file is dropped, waiting while
/// another window or process holds it, or another handle in this process. The file is opened for
/// reading and writing (a handle that only appends can't be locked on Windows) and is never
/// replaced or removed, so that every window locks the same file.
#[cfg(not(target_arch = "wasm32"))]
fn lock_store(dir: &Path) -> std::io::Result<std::fs::File> {
    let file = open_store_lock(dir)?;
    match file.try_lock() {
        Ok(()) => return Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {}
        Err(std::fs::TryLockError::Error(e)) => return Err(e),
    }
    #[cfg(test)]
    if let Some(waiting) = WHILE_STORE_LOCKED.take() {
        waiting();
    }
    file.lock()?;
    Ok(file)
}

/// [`DISABLED_LOCK`] in `dir`, opened (and made, with the folder) but not locked.
#[cfg(not(target_arch = "wasm32"))]
fn open_store_lock(dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    std::fs::File::options().create(true).truncate(false).read(true).write(true).open(dir.join(DISABLED_LOCK))
}

/// [`lock_store`] without waiting: `None` while another window or process holds the lock.
#[cfg(not(target_arch = "wasm32"))]
fn try_lock_store(dir: &Path) -> std::io::Result<Option<std::fs::File>> {
    let file = open_store_lock(dir)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// The browser has no folders to keep anything in.
#[cfg(target_arch = "wasm32")]
fn try_lock_store(_: &Path) -> std::io::Result<Option<std::fs::File>> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file system"))
}

/// The browser has no folders to keep anything in.
#[cfg(target_arch = "wasm32")]
fn lock_store(_: &Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file system"))
}

/// A turn on a plug-in store ([`Session::plugin_store_turn`]): while it is held, no other window or
/// process that takes one reads and changes the store's plug-ins, grants, disabled list, settings
/// or caches. Dropping it ends the turn. A turn is taken once per operation and handed to the
/// functions it calls, since a second lock of the same file by the same thread waits for ever.
pub(crate) struct StoreTurn {
    /// [`DISABLED_LOCK`], locked; `None` without a store, and for a load that goes ahead without
    /// a turn ([`Session::plugin_store_turn_for_load`]), which then writes nothing to the store.
    lock: Option<std::fs::File>,
}

impl StoreTurn {
    /// A turn that isn't held, for a load that goes ahead without one: it writes nothing.
    pub(crate) fn unheld() -> Self {
        StoreTurn { lock: None }
    }

    /// Whether the store may be written in this turn.
    fn held(&self) -> bool {
        self.lock.is_some()
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: [`Session::put_back_plugin`] fails on this thread, as on a full volume.
    pub(crate) static PUT_BACK_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: removing [`GRANTS_FILE`] fails on this thread while the file still reads, as when
    /// another program holds it open.
    pub(crate) static GRANTS_DELETE_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: writing [`GRANTS_FILE`] fails on this thread and leaves the file as it was, as on a
    /// full volume.
    pub(crate) static GRANTS_WRITE_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: [`Session::put_back_plugin`] writes the old module back on this thread and then
    /// reports a failure, as when the last flush of a copy fails.
    pub(crate) static PUT_BACK_FAILS_AFTER_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: moving a plug-in's settings and cache back from `<folder>.removed` ([`move_back`])
    /// fails on this thread, as when another program holds a file in the folder.
    pub(crate) static MOVE_BACK_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: writing the `.delete` file beside a folder about to be deleted ([`delete_aside`])
    /// fails on this thread, as on a full volume.
    pub(crate) static MARKER_WRITE_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: runs once on this thread when locking the plug-in store ([`lock_store`]) finds it
    /// locked and waits.
    pub(crate) static WHILE_STORE_LOCKED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once on this thread when a grant write has read `grants.json` and before it
    /// writes the file.
    pub(crate) static AFTER_GRANTS_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once on this thread when a change of a plug-in's settings has read them and
    /// before it writes them.
    pub(crate) static AFTER_SETTINGS_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// Remove `dir/file` → whether it is gone; a file that isn't there is what was wanted.
fn take(dir: &Path, file: &str) -> bool {
    let path = dir.join(file);
    #[cfg(test)]
    let removed = if file == GRANTS_FILE && GRANTS_DELETE_FAILS.get() {
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    } else {
        std::fs::remove_file(&path)
    };
    #[cfg(not(test))]
    let removed = std::fs::remove_file(&path);
    match removed {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            log::warn!("can't remove {}: {e}", path.display());
            false
        }
        _ => true,
    }
}

impl Session {
    /// Keep the PDF presets this session saves in `dir`, and take the ones already there as the
    /// saved presets (see the module docs). Call it once, after the preferences were restored.
    pub fn attach_pdf_preset_store(&mut self, dir: PathBuf) {
        if dir.is_dir() {
            self.prefs.pdf_presets = load_presets(&dir);
            self.pdf_preset_dir = Some(dir);
        } else {
            self.pdf_preset_dir = Some(dir);
            for preset in self.prefs.pdf_presets.clone() {
                self.store_pdf_preset(None, &preset);
            }
        }
    }

    /// Write the saved preset `preset` to the store, which held it as `old_name` when that is set
    /// (a rename). A rename writes the new file first and removes the old one only when that write
    /// succeeded: after a failed write the store still holds the preset under its old name. When
    /// the two names differ only in case, the old file is written and then renamed to the new name;
    /// when that rename fails, the old file gets its content back. Without a store, nothing happens.
    pub(crate) fn store_pdf_preset(&self, old_name: Option<&str>, preset: &PdfPreset) {
        let Some(dir) = &self.pdf_preset_dir else { return };
        let mut clean = preset.clone();
        clean.settings.clear_passwords();
        let file = format!("{}.{PRESET_FORMAT}", file_stem(&clean.name));
        let bytes = match serde_json::to_vec_pretty(&PresetFile::new(vec![clean])) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("can't save the PDF preset `{}`: {e}", preset.name);
                return;
            }
        };
        match old_name.map(|o| format!("{}.{PRESET_FORMAT}", file_stem(o))).filter(|old| *old != file) {
            // Names equal in any case get file names that differ only in case. A file system that
            // ignores case holds them as one file, which removing the old name would delete.
            Some(old) if old.to_lowercase() == file.to_lowercase() => match read_capped(&dir.join(&old), MAX_PRESET_FILE) {
                Ok(before) => {
                    if put(dir, &old, &bytes)
                        && let Err(e) = std::fs::rename(dir.join(&old), dir.join(&file))
                    {
                        log::warn!("can't rename {} to {file}: {e}", dir.join(&old).display());
                        put(dir, &old, &before);
                    }
                }
                Err(_) => {
                    put(dir, &file, &bytes);
                }
            },
            Some(old) => {
                if put(dir, &file, &bytes) {
                    take(dir, &old);
                }
            }
            None => {
                put(dir, &file, &bytes);
            }
        }
    }

    /// Remove the preset `name` from the store.
    pub(crate) fn unstore_pdf_preset(&self, name: &str) {
        if let Some(dir) = &self.pdf_preset_dir {
            take(dir, &format!("{}.{PRESET_FORMAT}", file_stem(name)));
        }
    }

    /// Keep the ICC profiles this session loads in `dir`, and register the ones already there.
    /// Call it before anything that names a profile (restoring the preferences, the first command).
    pub fn attach_profile_store(&mut self, dir: PathBuf) {
        let mut left = MAX_PROFILE_STORE;
        for path in list_files(&dir, PROFILE_EXT) {
            if left == 0 {
                log::warn!("skipping the other stored colour profiles: they hold more than {MAX_PROFILE_STORE} bytes together");
                break;
            }
            let loaded = read_capped(&path, MAX_PROFILE_FILE.min(left)).and_then(|bytes| {
                left -= bytes.len() as u64;
                vectorcraft_color::cms::register_icc(&bytes, None).map_err(|e| e.to_string())
            });
            if let Err(e) = loaded {
                log::warn!("skipping the stored colour profile {}: {e}", path.display());
            }
        }
        self.profile_dir = Some(dir);
    }

    /// Write the profile `name` (its file's `bytes`) to the store; `false` when it didn't go there.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn store_profile(&self, name: &str, bytes: &[u8]) -> bool {
        self.profile_dir.as_deref().is_some_and(|dir| put(dir, &format!("{}.{PROFILE_EXT}", file_stem(name)), bytes))
    }

    /// Remove the profile `name` from the store.
    pub(crate) fn unstore_profile(&self, name: &str) {
        if let Some(dir) = &self.profile_dir {
            take(dir, &format!("{}.{PROFILE_EXT}", file_stem(name)));
        }
    }

    /// Keep Edit › Color Settings (the working spaces, intent and black point compensation) in the
    /// file `path`, and make the settings saved there the active ones (#6335). After
    /// [`Session::attach_profile_store`]: they may name a profile it registers. Settings that can't be
    /// read, or that name a profile that is gone, leave the defaults with a log record, and the file
    /// as it is until the settings change.
    pub fn attach_color_settings(&mut self, path: PathBuf) {
        if path.exists() {
            let restored = read_capped(&path, MAX_PRESET_FILE)
                .and_then(|bytes| serde_json::from_slice::<vectorcraft_color::cms::ColorSettings>(&bytes).map_err(|e| e.to_string()))
                .and_then(|settings| vectorcraft_color::cms::set_active(&settings).map_err(|e| e.to_string()));
            if let Err(e) = restored {
                log::warn!("not using the saved color settings {}: {e}", path.display());
            }
        }
        self.color_settings_file = Some(path);
    }

    /// Write `settings` to the file [`Session::attach_color_settings`] named; `false` when they
    /// didn't go there.
    pub(crate) fn store_color_settings(&self, settings: &vectorcraft_color::cms::ColorSettings) -> bool {
        let Some(path) = &self.color_settings_file else { return false };
        let (Some(dir), Some(file)) = (path.parent(), path.file_name().and_then(|f| f.to_str())) else { return false };
        serde_json::to_vec_pretty(settings).is_ok_and(|bytes| put(dir, file, &bytes))
    }

    /// Keep the plug-ins this session installs in `dir`, take the disabled plug-ins saved there
    /// ([`DISABLED_FILE`]), and install the stored ones ([`Session::load_stored_plugins`]). Loads
    /// still running from an earlier call are finished first.
    pub fn attach_plugin_store(&mut self, dir: PathBuf) {
        self.finish_plugin_loads();
        self.plugin_dir = Some(dir);
        self.start_stored_plugins(true, None);
    }

    /// Install the plug-ins kept in the plug-in folder: none in Safe Mode, and none the user
    /// disabled (matched by file name, which is made from the id). The modules load on worker
    /// threads ([`vectorcraft_plugins::loader`]) and install in file-name order, whatever order
    /// their loads finish in; the launch marker names each file while it loads. A module that
    /// doesn't load (its load panicked or didn't finish in time included), one that changed or
    /// went away while it loaded, and one that lists permissions the grants there don't grant that
    /// very module ([`grant_covers`]), is skipped with a log record, and stays in the folder with
    /// its grant. A grant for an id whose stored module lists no permissions is forgotten; one that
    /// can't be removed from the grants file doesn't count for any session in the process attached
    /// to the folder until the file can be written for the id ([`Session::ignore_stored_grant`]).
    /// The folder is listed, and what loaded is installed and the folder changed, in turns on it
    /// ([`Session::plugin_store_turn_for_load`]); the modules load between them. With
    /// [`Session::load_plugins_in_background`] on, this returns while the modules load.
    pub fn load_stored_plugins(&mut self) {
        self.start_stored_plugins(false, None);
    }

    /// Loads the stored plug-in `path` again, as [`Session::load_stored_plugins`] loads each file,
    /// and waits until it is installed or skipped (`plugin.retry`) → the id it installed, if any.
    pub(crate) fn retry_stored_plugin(&mut self, path: &Path) -> Option<String> {
        self.finish_plugin_loads();
        self.start_stored_plugins(false, Some(path))
    }

    /// [`Session::load_stored_plugins`]; `disabled`: first take the disabled plug-ins saved in the
    /// folder ([`DISABLED_FILE`]); `only`: load that one file of the folder, if it is there. The
    /// files it loads stop being listed as failed ([`failed_stored_plugins`]) until they fail again.
    /// With `only`, waits for the load → the id it installed, if any.
    fn start_stored_plugins(&mut self, disabled: bool, only: Option<&Path>) -> Option<String> {
        let dir = self.plugin_dir.clone()?;
        // A `plugin.install` in another session or process writes the store before this lists it.
        let held = crate::cmd::plugin::hold_plugin_changes();
        let turn = self.plugin_store_turn_for_load();
        match only {
            Some(path) => {
                stored_failures().remove(path);
            }
            None => stored_failures().retain(|p, _| p.parent() != Some(dir.as_path())),
        }
        if disabled {
            let path = dir.join(DISABLED_FILE);
            let saved = read_capped(&path, safe_mode::MAX_DISABLED_FILE_BYTES).map(|b| safe_mode::parse_disabled_file(&b)).unwrap_or_default();
            safe_mode::set_disabled(saved);
        }
        if safe_mode::active() {
            return None;
        }
        let module = vectorcraft_plugins::Limits::default().max_module_bytes as u64;
        let mut rest = VecDeque::new();
        let disabled: BTreeSet<String> = safe_mode::skipped_ids().iter().map(|id| file_stem(id)).collect();
        // A retry reads its one file without listing the folder again.
        let listed: Vec<_> = match only {
            Some(path) => stamped_file(path.to_path_buf(), PLUGIN_EXT).filter(|(p, _)| p.parent() == Some(dir.as_path())).into_iter().collect(),
            None => list_stamped_files(&dir, PLUGIN_EXT),
        };
        // A retried file that isn't there (yet) stays listed, so a later retry can load it.
        if let Some(path) = only.filter(|p| listed.is_empty() && stored_failure(p).is_none()) {
            skip_stored(path, "it is no longer in the plug-in store");
        }
        for (path, stamp) in listed {
            let item = safe_mode::Item::file(path.display().to_string());
            let by_id = path.file_stem().is_some_and(|s| disabled.contains(s.to_string_lossy().as_ref()));
            if safe_mode::skipped(&item) || by_id {
                log::info!("skipping the stored plug-in {}: disabled", path.display());
            } else if stamp.0 > module {
                skip_stored(&path, too_large(module));
            } else {
                rest.push_back((path, Some(Some(stamp))));
            }
        }
        let mut left = plugin_store_budget();
        let (files, jobs) = list_stored_plugins(&mut rest, &mut left);
        // A change made after the listing, while `held` no longer stops one, is made elsewhere.
        let mut changes = vectorcraft_plugins::registry::LoadChanges::default();
        changes.start();
        // The modules load after the turn, once the loads queued before them are done.
        drop((turn, held));
        let load = StoredLoad {
            dir,
            files,
            loads: Loads::deferred(jobs),
            loaded: VecDeque::new(),
            installed: Vec::new(),
            next: 0,
            retired: BTreeMap::new(),
            changes,
            rest,
            left,
        };
        if only.is_some() {
            let report = self.finish_plugin_load(PluginLoad::Stored(load));
            return report["loaded"].as_array()?.first()?.as_str().map(str::to_string);
        }
        self.queue_plugin_load(PluginLoad::Stored(load));
        None
    }

    /// Installs what the stored plug-ins' `load` has loaded so far, in file-name order up to the
    /// first module still loading, without waiting ([`Session::load_stored_plugins`]), while
    /// [`crate::cmd::plugin::hold_plugin_changes`] and `turn` are held.
    fn install_stored_plugins(&mut self, load: &mut StoredLoad, turn: &StoreTurn) {
        if load.next == load.files.len() && !load.rest.is_empty() {
            // Every file listed so far is installed or skipped, and the reads that failed gave
            // back their length: the files waiting for them are listed now.
            let (files, jobs) = list_stored_plugins(&mut load.rest, &mut load.left);
            load.files = files;
            load.next = 0;
            load.loads = Loads::start(jobs);
            return;
        }
        load.loaded.extend(load.loads.ready());
        if load.loaded.is_empty() {
            return;
        }
        let dir = load.dir.clone();
        // The grants file is read once for this turn. `written` holds whether it names each id
        // whose grant this turn removed or put back. Only a signed file grants anything
        // ([`signed_grants`]).
        let file = read_grants(&dir);
        let mut grants = self.counted(&dir, signed_grants(&dir, &file));
        let mut written: BTreeMap<String, bool> = BTreeMap::new();
        let named = |written: &BTreeMap<String, bool>, id: &str| {
            written.get(id).copied().unwrap_or_else(|| match &file {
                Ok(file) => file.names(id),
                Err(e) => e.unread,
            })
        };
        while let (Some(loaded), Some((path, stamp))) = (load.loaded.pop_front(), load.files.get(load.next)) {
            load.next += 1;
            load.loads.release(1);
            // What a read that failed didn't read takes nothing from the total. A load that panicked
            // or passed its deadline gives nothing back.
            let loaded = loaded.and_then(|r| {
                load.left += stamp.flatten().map_or(0, |(len, _)| len).saturating_sub(r.read);
                r.plugin
            });
            // The worker read the file again once its module loaded ([`list_stored_plugins`]); one
            // whose stamp changed since then is skipped too.
            if file_stamp(path).ok() != *stamp {
                skip_stored(path, CHANGED_WHILE_LOADING);
                continue;
            }
            match loaded {
                Ok(plugin) if load.changes.superseded(plugin.id()) => {
                    log::warn!("skipping the stored plug-in {}: {}", path.display(), vectorcraft_plugins::registry::SUPERSEDED);
                }
                // Only this store's grants count: one given elsewhere in the process doesn't let a
                // module stored here load.
                Ok(plugin) if plugin.manifest().asks_permissions() => {
                    let m = plugin.manifest();
                    let stored = grants.get(&m.id).filter(|g| grant_covers(g, &plugin)).cloned();
                    // A grant given for the id since this load removed one, in another window or
                    // process, is kept.
                    let back = (!grants.contains_key(&m.id) && !named(&written, &m.id))
                        .then(|| load.retired.get(&m.id).filter(|g| grant_covers(g, &plugin)).cloned())
                        .flatten();
                    match stored.or(back.clone()) {
                        Some(g) => {
                            // A grant this load removed for an earlier file with the id goes back. When
                            // it can't be written back, the module isn't installed: the store doesn't
                            // grant it, so it wouldn't load at the next launch either.
                            if back.is_some() {
                                if !self.store_grant_in(turn, &m.id, Some(g.clone())) {
                                    skip_stored(path, "couldn't put back its grant");
                                    continue;
                                }
                                load.retired.remove(&m.id);
                                written.insert(m.id.clone(), true);
                                grants.insert(m.id.clone(), g.clone());
                            }
                            let id = m.id.clone();
                            let token = GrantToken::from_engine_grants_file(g.clone());
                            // The storage is made before the plug-ins are held: making it reads the store.
                            let storage = plugin_storage(Some(&dir), &id, turn);
                            let attach = || vectorcraft_plugins::registry::set_storage(&id, storage);
                            // The id is checked for a change elsewhere again while the plug-ins are held.
                            let installed = match vectorcraft_plugins::registry::install_for_load(
                                plugin.with_source(path.display().to_string()),
                                &load.changes,
                                Some(token),
                                attach,
                            ) {
                                Ok(Some(p)) => {
                                    load.changes.installed(&p);
                                    load.installed.push(p.id().to_string());
                                    installed_from(path);
                                    true
                                }
                                Ok(None) => {
                                    log::warn!("skipping the stored plug-in {}: {}", path.display(), vectorcraft_plugins::registry::SUPERSEDED);
                                    false
                                }
                                Err(e) => {
                                    skip_stored(path, e);
                                    false
                                }
                            };
                            // A grant put back for a module that didn't install is removed again, so the
                            // store doesn't grant those bytes at the next launch.
                            if back.is_some() && !installed {
                                if self.store_grant_in(turn, &id, None) {
                                    written.insert(id.clone(), false);
                                    grants.remove(&id);
                                    load.retired.insert(id, g);
                                } else {
                                    log::warn!("couldn't remove the grant put back for the stored plug-in {}", path.display());
                                }
                            }
                        }
                        None => skip_stored(path, "the user hasn't granted this module the permissions it lists"),
                    }
                }
                Ok(plugin) => {
                    let id = plugin.id().to_string();
                    // The storage is made before the plug-ins are held: making it reads the store.
                    let storage = plugin_storage(Some(&dir), &id, turn);
                    let attach = || vectorcraft_plugins::registry::set_storage(&id, storage);
                    // The id is checked for a change elsewhere again while the plug-ins are held.
                    let p = match vectorcraft_plugins::registry::install_for_load(
                        plugin.with_source(path.display().to_string()),
                        &load.changes,
                        None,
                        attach,
                    ) {
                        Ok(Some(p)) => p,
                        Ok(None) => {
                            log::warn!("skipping the stored plug-in {}: {}", path.display(), vectorcraft_plugins::registry::SUPERSEDED);
                            continue;
                        }
                        Err(e) => {
                            skip_stored(path, e);
                            continue;
                        }
                    };
                    load.changes.installed(&p);
                    load.installed.push(p.id().to_string());
                    installed_from(path);
                    // In the same turn: the grants for the id stop counting, the process's and the
                    // store's, so no session sees the earlier module as approved. A module installed
                    // in its place meanwhile keeps the process's grant recorded for it.
                    vectorcraft_plugins::registry::revoke_if_installed(&id, p.serial());
                    if let Some(g) = grants.remove(&id) {
                        load.retired.insert(id.clone(), g);
                    }
                    if named(&written, &id) {
                        let writes = self.grant_writes();
                        if self.store_grant_in(turn, &id, None) {
                            written.insert(id.clone(), false);
                        } else if self.holds_plugin_grant(&id) {
                            self.ignore_stored_grant(&id, writes);
                        }
                    }
                }
                Err(e) => skip_stored(path, e),
            }
        }
    }

    /// Whether the loads of the stored plug-ins and of the Additional Plug-ins Folder go on after
    /// the call that starts them ([`Session::attach_plugin_store`], [`Session::load_stored_plugins`],
    /// preferences that set the folder): the app turns it on for its launch so that its window
    /// shows while they load, installs what loaded with [`Session::poll_plugin_loads`] as its
    /// frames go by, and turns it off once the launch's loads have started. Off (the default),
    /// those calls return once what loaded is installed, the loads still running from before
    /// included.
    pub fn load_plugins_in_background(&mut self, on: bool) {
        self.plugin_loads_in_background = on;
    }

    /// Installs what the plug-in loads have loaded so far, in the order the loads started and each
    /// load's file-name order, up to the first module still loading, without waiting → whether
    /// some are still loading. A load starts once the loads before it are done. When another
    /// session or process is changing the plug-ins or the store, nothing installs until a later
    /// call.
    pub fn poll_plugin_loads(&mut self) -> bool {
        while let Some(mut next) = self.plugin_loads.pop_front() {
            next.start();
            if !next.ready() {
                self.plugin_loads.push_front(next);
                return true;
            }
            let Some((held, turn)) = self.plugin_install_turn_now(next.creates_store()) else {
                self.plugin_loads.push_front(next);
                return true;
            };
            self.install_plugin_load(&mut next, &turn);
            if !next.done() {
                self.plugin_loads.push_front(next);
                return true;
            }
            self.end_plugin_load(next, &turn);
            drop((turn, held));
        }
        false
    }

    /// Waits for every plug-in load still running and installs what they loaded, in the order the
    /// loads started. Plug-in commands, and commands that may edit, call it first
    /// ([`Session::execute`]).
    pub fn finish_plugin_loads(&mut self) {
        while let Some(next) = self.plugin_loads.pop_front() {
            self.finish_plugin_load(next);
        }
    }

    /// Ends the plug-in loads still running without installing more of what they load: the app
    /// calls it as it quits. Modules not started are dropped, and the ones running are given until
    /// their deadline, so the launch marker is not left naming a plug-in that was loading. Each load
    /// then ends as one that finished does for what it installed; a folder's end saves the disabled
    /// list only when the store's turn can be taken without waiting.
    pub fn cancel_plugin_loads(&mut self) {
        while let Some(mut load) = self.plugin_loads.pop_front() {
            load.cancel();
            // A stored load's grants were settled as each module installed: its end writes nothing.
            if !load.creates_store() {
                self.end_plugin_load(load, &StoreTurn::unheld());
                continue;
            }
            // Another session changing the plug-ins isn't waited for either: the folder's load then
            // ends without the store's turn, so the grants of what it installed are still forgotten
            // and the store isn't written.
            let Some(held) = crate::cmd::plugin::try_hold_plugin_changes() else {
                self.end_plugin_load(load, &StoreTurn::unheld());
                continue;
            };
            // A store another window or process is using isn't waited for: the turn then isn't
            // held, and the folder's end writes nothing to the store.
            let turn = match self.plugin_dir.as_deref().map(try_lock_store) {
                Some(Ok(Some(file))) => StoreTurn { lock: Some(file) },
                _ => StoreTurn::unheld(),
            };
            self.end_plugin_load(load, &turn);
            drop((turn, held));
        }
    }

    /// Waits for `load`'s modules and installs them as they load, in turns on the store, then
    /// ends it → its report (the installed ids for the stored plug-ins).
    pub(crate) fn finish_plugin_load(&mut self, mut load: PluginLoad) -> Value {
        load.start();
        loop {
            load.wait();
            let held = crate::cmd::plugin::hold_plugin_changes();
            let turn = self.plugin_install_turn(load.creates_store());
            self.install_plugin_load(&mut load, &turn);
            if load.done() {
                let report = self.end_plugin_load(load, &turn);
                drop((turn, held));
                return report;
            }
        }
    }

    /// A turn on the store for installing what a load loaded, taken after
    /// [`crate::cmd::plugin::hold_plugin_changes`]: the folder's own turn
    /// ([`Session::plugin_store_turn`]) when `create`, else one that goes ahead without a store
    /// folder ([`Session::plugin_store_turn_for_load`]). One that can't be locked goes ahead
    /// unheld.
    fn plugin_install_turn(&self, create: bool) -> StoreTurn {
        if create { self.plugin_store_turn().unwrap_or_else(|_| StoreTurn::unheld()) } else { self.plugin_store_turn_for_load() }
    }

    /// [`crate::cmd::plugin::hold_plugin_changes`] and [`Session::plugin_install_turn`] without
    /// waiting: `None` while another session holds the one or another window or process the other.
    fn plugin_install_turn_now(&self, create: bool) -> Option<(MutexGuard<'static, ()>, StoreTurn)> {
        let held = crate::cmd::plugin::try_hold_plugin_changes()?;
        let turn = match self.plugin_dir.as_deref() {
            Some(dir) if create || dir.is_dir() => match try_lock_store(dir) {
                Ok(Some(file)) => StoreTurn { lock: Some(file) },
                Ok(None) => return None,
                Err(e) => {
                    log::warn!("can't lock {}: {e}", dir.join(DISABLED_LOCK).display());
                    StoreTurn::unheld()
                }
            },
            _ => StoreTurn::unheld(),
        };
        Some((held, turn))
    }

    /// Whether plug-ins are still loading on worker threads ([`Session::load_plugins_in_background`]).
    pub fn plugins_loading(&self) -> bool {
        !self.plugin_loads.is_empty()
    }

    /// Keeps `load` to install after the loads started before it; without
    /// [`Session::load_plugins_in_background`], waits for them all and installs what they loaded.
    pub(crate) fn queue_plugin_load(&mut self, load: PluginLoad) {
        self.plugin_loads.push_back(load);
        // The first load starts now; one queued after it starts once those before it are done.
        if let [only] = self.plugin_loads.make_contiguous() {
            only.start();
        }
        if !self.plugin_loads_in_background {
            self.finish_plugin_loads();
        }
    }

    /// Installs what `load` has loaded so far, without waiting, in `turn`.
    fn install_plugin_load(&mut self, load: &mut PluginLoad, turn: &StoreTurn) {
        match load {
            PluginLoad::Stored(load) => self.install_stored_plugins(load, turn),
            PluginLoad::Folder(load) => crate::cmd::plugin::install_folder(self, load, turn),
        }
    }

    /// Ends `load` once [`PluginLoad::done`], in `turn` → its report.
    fn end_plugin_load(&mut self, load: PluginLoad, turn: &StoreTurn) -> Value {
        // The loads queued after it count what it installed as their own: one of theirs replaces
        // it, as it would have with the loads run one after the other.
        if let Some(ended) = load.changes() {
            for later in &mut self.plugin_loads {
                if let Some(changes) = later.changes_mut() {
                    changes.adopt(ended);
                }
            }
        }
        match load {
            // A stored load's grants were settled as each module installed.
            PluginLoad::Stored(load) => serde_json::json!({"loaded": load.installed}),
            PluginLoad::Folder(load) => crate::cmd::plugin::end_folder(self, load, turn),
        }
    }

    /// Take a turn on the attached plug-in store ([`StoreTurn`]), waiting while another window or
    /// process holds one; without a store, a turn that writes nothing. An error, with a log record,
    /// when the lock file can't be made, opened or locked. Taken after
    /// [`crate::cmd::plugin::hold_plugin_changes`] when both are needed, and never by a thread that
    /// holds a turn already.
    pub(crate) fn plugin_store_turn(&self) -> std::io::Result<StoreTurn> {
        let Some(dir) = self.plugin_dir.as_deref() else { return Ok(StoreTurn::unheld()) };
        lock_store(dir)
            .map(|file| StoreTurn { lock: Some(file) })
            .inspect_err(|e| log::warn!("can't lock {}: {e}", dir.join(DISABLED_LOCK).display()))
    }

    /// [`Session::plugin_store_turn`] for a load, which goes ahead without the turn when the store
    /// folder isn't there (nothing is stored) or can't be locked, and then writes nothing to it.
    pub(crate) fn plugin_store_turn_for_load(&self) -> StoreTurn {
        match self.plugin_dir.as_deref() {
            Some(dir) if dir.is_dir() => self.plugin_store_turn().unwrap_or_else(|_| StoreTurn::unheld()),
            _ => StoreTurn::unheld(),
        }
    }

    /// Write the disabled plug-ins saved for the next launch to the plug-in folder → whether they
    /// went there, every change included (`false` without a folder, and when a plug-in turned off
    /// found the file's list full: [`safe_mode::Merged::complete`]). The changes this launch made are applied to the list
    /// the file holds now ([`safe_mode::merge_saved`]), so a change another window saved to the same
    /// folder meanwhile stays. Windows that save at the same time take turns on [`DISABLED_LOCK`];
    /// when it can't be locked, nothing is written, and the next change saves this one too. So is
    /// a file that can't be read (too large, say) or that a later version wrote
    /// ([`safe_mode::read_disabled_file`]): it is kept as it is.
    pub fn store_disabled_plugins(&self) -> bool {
        self.plugin_store_turn().is_ok_and(|turn| self.store_disabled_plugins_in(&turn))
    }

    /// [`Session::store_disabled_plugins`] in `turn`; `false` when it isn't held.
    pub(crate) fn store_disabled_plugins_in(&self, turn: &StoreTurn) -> bool {
        let Some(dir) = self.plugin_dir.as_deref() else { return false };
        if !turn.held() {
            return false;
        }
        let path = dir.join(DISABLED_FILE);
        let theirs = match read_capped(&path, safe_mode::MAX_DISABLED_FILE_BYTES) {
            Ok(bytes) => match safe_mode::read_disabled_file(&bytes) {
                Some(items) => items,
                None => {
                    log::warn!("can't save {}: it holds another format or version of the list", path.display());
                    return false;
                }
            },
            Err(_) if !path.exists() => Vec::new(),
            Err(e) => {
                log::warn!("can't save {}: {e}", path.display());
                return false;
            }
        };
        let merged = safe_mode::merge_saved(Some(theirs));
        let written = put(dir, DISABLED_FILE, &merged.bytes);
        if written {
            safe_mode::saved_written(&merged);
        }
        written && merged.complete()
    }

    /// Whether `plugin` installs without asking the user: it lists no permissions, or this very
    /// module (its id and SHA-256) was granted every one it lists, in the grants of the attached
    /// plug-in store when there is one ([`grant_covers`]), not counting a grant that should be gone
    /// and could not be removed ([`Session::ignore_stored_grant`]), else in this process's
    /// ([`vectorcraft_plugins::registry::approved`]).
    pub(crate) fn plugin_approved(&self, plugin: &vectorcraft_plugins::Plugin) -> bool {
        self.plugin_approver()(plugin)
    }

    /// [`Session::plugin_approved`] for many plug-ins: reads the store's grants once, when it is
    /// made, and checks each plug-in against them.
    pub(crate) fn plugin_approver(&self) -> impl Fn(&vectorcraft_plugins::Plugin) -> bool + use<> {
        let grants = self.plugin_dir.as_deref().map(|dir| self.counted_grants(dir));
        move |plugin| {
            let m = plugin.manifest();
            match &grants {
                Some(grants) if m.asks_permissions() => grants.get(&m.id).is_some_and(|g| grant_covers(g, plugin)),
                _ => vectorcraft_plugins::registry::approved(plugin),
            }
        }
    }

    /// [`Session::plugin_approver`] for a folder load: a module the store's grants approve gets
    /// that grant recorded for the process too, since the registry installs only a module that a
    /// recorded grant covers ([`vectorcraft_plugins::registry::install`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn plugin_folder_approver(&self) -> impl Fn(&vectorcraft_plugins::Plugin) -> bool + use<> {
        let grants = self.plugin_dir.as_deref().map(|dir| self.counted_grants(dir));
        move |plugin| {
            let m = plugin.manifest();
            match &grants {
                Some(grants) if m.asks_permissions() => match grants.get(&m.id).filter(|g| grant_covers(g, plugin)) {
                    Some(g) => {
                        vectorcraft_plugins::registry::grant(GrantToken::from_engine_grants_file(g.clone()));
                        true
                    }
                    None => false,
                },
                _ => vectorcraft_plugins::registry::approved(plugin),
            }
        }
    }

    /// The grants kept in the plug-in store `dir` ([`stored_grants`]) without those the process
    /// doesn't count for that folder ([`Session::ignore_stored_grant`]). The folder's key is worked
    /// out on each call, so a path whose link now leads elsewhere is matched with where it leads.
    fn counted_grants(&self, dir: &Path) -> BTreeMap<String, vectorcraft_plugins::permissions::Grant> {
        self.counted(dir, stored_grants(dir))
    }

    /// `grants`, read from the plug-in store `dir`, without those the process doesn't count for
    /// that folder ([`Session::counted_grants`]).
    fn counted(
        &self,
        dir: &Path,
        mut grants: BTreeMap<String, vectorcraft_plugins::permissions::Grant>,
    ) -> BTreeMap<String, vectorcraft_plugins::permissions::Grant> {
        let key = plugin_store_key(dir);
        let shared = IGNORED_PLUGIN_GRANTS.lock().unwrap_or_else(|e| e.into_inner());
        for (_, id) in shared.ignored.range((key.clone(), String::new())..).take_while(|(folder, _)| *folder == key) {
            grants.remove(id);
        }
        grants
    }

    /// Write the module `bytes` of the plug-in `id` to the store in `turn` → whether they went there
    /// (`false` without a folder, and when `turn` isn't held). The file written is no longer listed
    /// as failed ([`failed_stored_plugins`]).
    pub(crate) fn store_plugin(&self, turn: &StoreTurn, id: &str, bytes: &[u8]) -> bool {
        let name = format!("{}.{PLUGIN_EXT}", file_stem(id));
        turn.held()
            && self.plugin_dir.as_deref().is_some_and(|dir| {
                let written = put(dir, &name, bytes);
                if written {
                    stored_failures().remove(&dir.join(&name));
                    crate::cmd::plugin::clear_folder_failure(&dir.join(&name));
                }
                written
            })
    }

    /// The module the store holds for the plug-in `id`: `None` without a folder or such a file, and
    /// an error, with a log record, when the file can't be read or is larger than the module limit.
    pub(crate) fn stored_plugin_bytes(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(dir) = self.plugin_dir.as_deref() else { return Ok(None) };
        let path = dir.join(format!("{}.{PLUGIN_EXT}", file_stem(id)));
        if !path.exists() {
            return Ok(None);
        }
        let max = vectorcraft_plugins::Limits::default().max_module_bytes as u64;
        read_capped(&path, max).map(Some).inspect_err(|e| log::warn!("can't read the stored plug-in {}: {e}", path.display()))
    }

    /// Return the store's module for the plug-in `id` to `previous`, the bytes it held before a
    /// write ([`Session::stored_plugin_bytes`]): with `None`, the file is removed. → whether the
    /// write reported success; a write that reports a failure may still have put `previous` back.
    /// In `turn`, the one the write was made in.
    pub(crate) fn put_back_plugin(&self, turn: &StoreTurn, id: &str, previous: Option<&[u8]>) -> bool {
        #[cfg(test)]
        if PUT_BACK_FAILS.get() {
            return false;
        }
        let back = match previous {
            Some(bytes) => self.store_plugin(turn, id, bytes),
            None => self.unstore_plugin(turn, id),
        };
        #[cfg(test)]
        if PUT_BACK_FAILS_AFTER_WRITE.get() {
            return false;
        }
        back
    }

    /// The file the plug-in `id` is stored in, spelled as [`Session::load_stored_plugins`] names it
    /// (`None` without a plug-in folder).
    pub(crate) fn stored_plugin_file(&self, id: &str) -> Option<String> {
        self.plugin_dir.as_ref().map(|dir| dir.join(format!("{}.{PLUGIN_EXT}", file_stem(id))).display().to_string())
    }

    /// Tests: [`Session::store_grant_in`] in a turn of its own ([`Session::plugin_store_turn`]);
    /// `false` when it can't be taken.
    #[cfg(test)]
    pub(crate) fn store_grant(&self, id: &str, grant: Option<vectorcraft_plugins::permissions::Grant>) -> bool {
        self.plugin_store_turn().is_ok_and(|turn| self.store_grant_in(&turn, id, grant))
    }

    /// Record in the store what the user granted plug-in `id` (`None`: nothing), leaving the store's
    /// other grants as they are; with none left, remove the file. Entries this version can't use
    /// are written back as they were, except one for `id`. The generation is raised, and the file is
    /// written at the generation after the raised one with the MAC of its grants for this folder
    /// ([`grant_keys`]), making the key when the key store holds none; the key store then keeps that
    /// generation ([`put_grants`]). A signed file left empty is removed after raising the generation
    /// ([`take_signed_grants`]). The generation is never lowered: when the file can't be written or
    /// removed after the raise, the file in place grants nothing and its plug-ins ask again. A file
    /// VectorCraft didn't write for this folder ([`Trust::Unsigned`]) grants nothing: a grant replaces it with a file that holds that grant alone, and removing one
    /// raises the generation ([`raise_generation`]), then takes out the entry for `id` and leaves the
    /// rest unsigned. Removing a grant when there is no file, or when the file grants nothing
    /// (unsigned, too large or not a list) and has no entry to take out, also raises the generation
    /// when the key store holds a key ([`retire_signed_copies`]), so that a signed copy put back
    /// doesn't grant it again. The generation is never read from the file; when it can't be raised,
    /// nothing is recorded. When the key store can't be reached ([`Trust::Unknown`], or
    /// a key can't be made or kept), the file is left as it is and nothing is recorded, and an entry
    /// for `id` stays; a removal then reports a failure, also when the file doesn't name `id`. A
    /// grants file that is too large, can't be read or doesn't hold a list is left as it is, with a
    /// log record, and nothing is recorded; so is one whose new content would be larger than
    /// `MAX_GRANTS_FILE`. A removal still holds for a file that is too large or doesn't hold a list
    /// once the generation is raised ([`retire_signed_copies`]). The file is read and
    /// written in `turn`. → whether the store now holds what was asked (`false` without a folder,
    /// and when `turn` isn't held). When it does, the grant kept for `id` counts again, for every
    /// session in the process attached to the store ([`Session::ignore_stored_grant`]).
    pub(crate) fn store_grant_in(&self, turn: &StoreTurn, id: &str, grant: Option<vectorcraft_plugins::permissions::Grant>) -> bool {
        let done = turn.held() && self.write_grant(id, grant);
        if done && let Some(key) = self.plugin_dir.as_deref().map(plugin_store_key) {
            let mut shared = IGNORED_PLUGIN_GRANTS.lock().unwrap_or_else(|e| e.into_inner());
            let entry = (key, id.to_string());
            shared.ignored.remove(&entry);
            shared.writes = shared.writes.saturating_add(1);
            let writes = shared.writes;
            shared.written.insert(entry, writes);
            if shared.written.len() > MAX_GRANT_WRITES_KEPT
                && let Some(oldest) = shared.written.iter().min_by_key(|(_, at)| **at).map(|(entry, _)| entry.clone())
            {
                shared.written.remove(&oldest);
            }
        }
        done
    }

    /// How many grant writes the sessions of the process have made ([`Session::store_grant_in`]).
    /// Taken before a write that may fail, for [`Session::ignore_stored_grant`].
    pub(crate) fn grant_writes(&self) -> u64 {
        IGNORED_PLUGIN_GRANTS.lock().unwrap_or_else(|e| e.into_inner()).writes
    }

    /// How many grant writes the process keeps ([`IgnoredGrants::written`]).
    #[cfg(test)]
    pub(crate) fn grant_writes_kept() -> usize {
        IGNORED_PLUGIN_GRANTS.lock().unwrap_or_else(|e| e.into_inner()).written.len()
    }

    /// Stop counting the attached store's grant for the plug-in `id`, in every session in the
    /// process attached to the same folder, until [`Session::store_grant_in`] next succeeds for it in
    /// one of them: for a grant that should be gone and could not be removed, which would otherwise
    /// approve the bytes it covers. `writes` is [`Session::grant_writes`] from before the failed
    /// write: when a session has written the store's grants for `id` since, the file holds what that
    /// write asked for, and the grant keeps counting. A write no longer among the latest
    /// [`MAX_GRANT_WRITES_KEPT`] is not seen, and the grant stops counting. Another process on the
    /// folder reads `grants.json` as it is.
    pub(crate) fn ignore_stored_grant(&self, id: &str, writes: u64) {
        if let Some(key) = self.plugin_dir.as_deref().map(plugin_store_key) {
            let mut shared = IGNORED_PLUGIN_GRANTS.lock().unwrap_or_else(|e| e.into_inner());
            let entry = (key, id.to_string());
            if shared.written.get(&entry).is_none_or(|at| *at <= writes) {
                shared.ignored.insert(entry);
            }
        }
    }

    fn write_grant(&self, id: &str, grant: Option<vectorcraft_plugins::permissions::Grant>) -> bool {
        let Some(dir) = &self.plugin_dir else { return false };
        let mut file = match read_grants(dir) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("not updating the stored plug-in grants {}: {}", dir.join(GRANTS_FILE).display(), e.why);
                // A file that is too large or doesn't hold a list grants nothing and is left as it
                // is, but a signed copy made before it must not grant `id` again once it is put back:
                // the removal holds once no such copy can.
                return grant.is_none() && !e.unread && retire_signed_copies(dir);
            }
        };
        #[cfg(test)]
        if let Some(between) = AFTER_GRANTS_READ.take() {
            between();
        }
        match (file.trust, &grant) {
            // Nothing in the file can be checked, so nothing in it is rewritten: a grant holds for
            // the session only, and one the file may still hold for `id` can't be removed. A file
            // that doesn't name `id` may not be the one the kept generation was signed for, so the
            // removal isn't known to hold either.
            (Trust::Unknown, _) => return false,
            // The file grants nothing. A new grant replaces it with a signed file that holds that
            // grant alone; removing one takes its entry out and leaves the rest unsigned.
            (Trust::Unsigned, Some(_)) => {
                log::warn!("replacing the stored plug-in grants {}: VectorCraft didn't write them for this folder", dir.join(GRANTS_FILE).display());
                file.grants.clear();
                file.other.clear();
            }
            (Trust::Unsigned | Trust::Signed, _) => {}
        }
        let signed = file.trust == Trust::Signed || grant.is_some();
        let removing = grant.is_none();
        let others = file.other.len();
        file.other.retain(|entry| entry.get("id").and_then(serde_json::Value::as_str) != Some(id));
        let changed = match grant {
            Some(g) => file.grants.insert(id.to_string(), g.clone()).as_ref() != Some(&g),
            None => file.grants.remove(id).is_some(),
        } || file.other.len() != others;
        if !changed {
            // Without a file, or with one that grants nothing, the last signed file may still be
            // around as a copy at the kept generation: a removal raises the generation so that the
            // copy doesn't grant `id` again once it is put back. A signed file that doesn't name `id`
            // is the latest, so every copy that does is at an earlier generation already. The new
            // generation is past every one a file was signed at, so the file left in place grants
            // nothing either.
            return !removing || (file.present && file.trust == Trust::Signed) || retire_signed_copies(dir);
        }
        let mut entries: Vec<serde_json::Value> = file.grants.into_values().filter_map(|g| serde_json::to_value(g).ok()).collect();
        entries.extend(file.other);
        if signed {
            return if entries.is_empty() { take_signed_grants(dir) } else { put_grants(dir, entries, true) };
        }
        // An unsigned file loses an entry: the generation is raised first, so that no signed copy
        // made before brings the entry back, nor the file itself when it can't be rewritten.
        if raise_generation(dir).is_none() {
            return false;
        }
        if entries.is_empty() { take(dir, GRANTS_FILE) } else { put_grants(dir, entries, false) }
    }

    /// Whether the store holds, or may hold, an entry for the plug-in `id`, including one this
    /// version can't use ([`GrantsFile::names`]). `true` when the grants file can't be opened or
    /// read, because it may name `id` once it reads again; `false` without a folder, and when the
    /// file is too large or doesn't hold a list.
    pub(crate) fn holds_plugin_grant(&self, id: &str) -> bool {
        self.plugin_dir.as_deref().is_some_and(|dir| match read_grants(dir) {
            Ok(file) => file.names(id),
            Err(e) => e.unread,
        })
    }

    /// The grant the attached plug-in store's signed grants file holds for the plug-in `id` (`None`
    /// without a folder, and when the file grants nothing: unsigned, unreadable or without an entry
    /// for `id`).
    pub(crate) fn stored_plugin_grant(&self, id: &str) -> Option<vectorcraft_plugins::permissions::Grant> {
        self.plugin_dir.as_deref().and_then(|dir| stored_grants(dir).remove(id))
    }

    /// Does the attached plug-in store hold a module file for the plug-in `id`, loaded or not (one
    /// that no grant covers is skipped at startup and stays in the folder)?
    pub(crate) fn holds_stored_plugin(&self, id: &str) -> bool {
        self.plugin_dir.as_deref().is_some_and(|dir| std::fs::symlink_metadata(dir.join(format!("{}.{PLUGIN_EXT}", file_stem(id)))).is_ok())
    }

    /// Remove the plug-in `id` from the store in `turn` → whether its file is gone (`false` without a
    /// folder, and when `turn` isn't held). A file removed is no longer listed as failed
    /// ([`failed_stored_plugins`]).
    pub(crate) fn unstore_plugin(&self, turn: &StoreTurn, id: &str) -> bool {
        let name = format!("{}.{PLUGIN_EXT}", file_stem(id));
        turn.held()
            && self.plugin_dir.as_deref().is_some_and(|dir| {
                let gone = take(dir, &name);
                if gone {
                    stored_failures().remove(&dir.join(&name));
                    crate::cmd::plugin::clear_folder_failure(&dir.join(&name));
                }
                gone
            })
    }

    /// Keep the settings and cache of plug-in `id` in the attached plug-in store from now on; without
    /// one, nowhere (its storage calls then fail as unavailable). Settings and cache that a removal
    /// kept are moved back first, in `turn` ([`plugin_storage`]).
    pub(crate) fn attach_plugin_storage(&self, turn: &StoreTurn, id: &str) {
        vectorcraft_plugins::registry::set_storage(id, plugin_storage(self.plugin_dir.as_deref(), id, turn));
    }

    /// Does the attached plug-in store have a folder of settings and cache for plug-in `id`, or one
    /// that a removal kept ([`aside_paths`], [`is_kept`])? A folder behind a `storage` folder
    /// that is a link is not counted: the plug-in's storage calls can't reach it and removing the
    /// plug-in leaves it.
    pub(crate) fn has_plugin_storage(&self, id: &str) -> bool {
        self.plugin_dir.as_deref().is_some_and(|dir| {
            let path = plugin_data_dir(dir, id);
            !is_link(&dir.join(PLUGIN_DATA_DIR)) && (std::fs::symlink_metadata(&path).is_ok() || aside_paths(&path).iter().any(|a| is_kept(a)))
        })
    }

    /// Delete the settings and cache of plug-in `id` from the store, and stop keeping any for it.
    /// Settings and cache that an earlier removal kept ([`is_kept`]) are first moved back when the
    /// folder is missing ([`restore_removed`]); what earlier deletions left is deleted. The folder is
    /// then renamed out of the way ([`aside_paths`]: `<folder>.removed`, or `<folder>.removing` while
    /// `<folder>.removed` holds kept settings), and that fails, leaving it as it was, when the rename
    /// does. `forget` runs once the folder is out of the way, before anything is deleted: when it
    /// returns `false`, the folder is renamed back and this fails, and the kept settings stay. When
    /// that rename fails too, the error also names both paths and the reason, and the next attach or
    /// removal for the id moves the folder back. Once `forget` returns `true`, the folder and the
    /// kept settings are deleted ([`delete_aside`]); what that deletion leaves stays, with a log
    /// record, and a later call for the id tries to delete it again. All of it happens in `turn`;
    /// with a store and a turn that isn't held, nothing changes and this fails.
    pub(crate) fn remove_plugin_storage(&self, turn: &StoreTurn, id: &str, forget: impl FnOnce() -> bool) -> Result<(), StorageError> {
        let not_forgotten = || StorageError::Io(GRANT_NOT_REMOVED.to_string());
        if self.plugin_dir.is_some() && !turn.held() {
            return Err(StorageError::Io(STORE_NOT_LOCKED.to_string()));
        }
        vectorcraft_plugins::registry::set_storage(id, None);
        let Some(dir) = &self.plugin_dir else {
            return if forget() { Ok(()) } else { Err(not_forgotten()) };
        };
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = plugin_data_dir(dir, id);
        // Every storage handed out for the folder, including those runs still hold, is closed once
        // the folder is out of the way; until then they keep working.
        let close = || {
            if let Some(data) = PLUGIN_DATA_OPEN.lock().unwrap_or_else(|e| e.into_inner()).remove(&path) {
                data.closed.store(true, Ordering::Release);
            }
        };
        // A `storage` folder that is a link leads out of the store: nothing there is deleted. A link
        // in place of the plug-in's own folder is removed itself (`remove_dir_all` doesn't follow it).
        let data = dir.join(PLUGIN_DATA_DIR);
        if is_link(&data) {
            log::warn!("{} is a link: the plug-in settings and cache it leads to are left", data.display());
            if !forget() {
                return Err(not_forgotten());
            }
            close();
            return Ok(());
        }
        let asides = aside_paths(&path);
        if std::fs::symlink_metadata(&path).is_err()
            && let Err(e) = restore_removed(&path)
        {
            log::warn!("can't move the settings and cache of {} back: {e}", path.display());
        }
        for aside in asides.iter().filter(|a| !is_kept(a)) {
            delete_aside(aside);
        }
        let target = if std::fs::symlink_metadata(&path).is_err() {
            asides.iter().rev().find(|a| is_kept(a)).unwrap_or(&asides[0])
        } else if !is_kept(&asides[0]) {
            &asides[0]
        } else if std::fs::symlink_metadata(&asides[1]).is_err() {
            &asides[1]
        } else {
            return Err(StorageError::Io(format!("{}: {} and {} are in the way", path.display(), asides[0].display(), asides[1].display())));
        };
        match std::fs::rename(&path, target) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StorageError::Io(format!("{}: {e}", path.display()))),
            Ok(()) => {}
        }
        if !forget() {
            if std::fs::symlink_metadata(target).is_ok()
                && let Err(e) = move_back(target, &path)
            {
                log::warn!("can't move {} back: {e}", target.display());
                return Err(StorageError::Io(format!(
                    "{GRANT_NOT_REMOVED}, and its settings and cache can't be moved back from {} to {}: {e}",
                    target.display(),
                    path.display()
                )));
            }
            return Err(not_forgotten());
        }
        close();
        for aside in &asides {
            delete_aside(aside);
        }
        Ok(())
    }
}

/// `name` (a plug-in id or a cache entry name, ASCII) written for a file name that no other name
/// gets on a file system that ignores case: `_` is written `__`, each capital letter `_` and the
/// letter in lower case, and `.` `_0`.
fn case_safe(name: &str) -> String {
    let mut file = String::with_capacity(name.len() * 2);
    for c in name.chars() {
        match c {
            '_' => file.push_str("__"),
            '.' => file.push_str("_0"),
            c if c.is_ascii_uppercase() => {
                file.push('_');
                file.push(c.to_ascii_lowercase());
            }
            c => file.push(c),
        }
    }
    file
}

/// The folder of plug-in `id`'s settings and cache in the plug-in store `dir`. The `id-` in front
/// keeps an id such as `con` from naming a device on Windows.
pub(crate) fn plugin_data_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join(PLUGIN_DATA_DIR).join(format!("id-{}", case_safe(id)))
}

/// `path` with `suffix` added to its last part.
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut named = path.as_os_str().to_os_string();
    named.push(suffix);
    PathBuf::from(named)
}

/// Where [`Session::remove_plugin_storage`] moves the plug-in folder `path` out of the way:
/// `<path>.removed`, and `<path>.removing` while `<path>.removed` holds settings and cache an
/// earlier removal kept ([`is_kept`]). The second one, when it is there, is the newer.
fn aside_paths(path: &Path) -> [PathBuf; 2] {
    [suffixed(path, ".removed"), suffixed(path, ".removing")]
}

/// `<aside>.delete`: an empty file written next to the folder `aside` before it is deleted, so
/// that what a failed deletion leaves there is not taken for settings a removal kept.
fn delete_marker(aside: &Path) -> PathBuf {
    suffixed(aside, ".delete")
}

/// Does `aside` hold settings and cache that a removal kept: a folder, or a link in place of one,
/// without a [`delete_marker`] beside it?
fn is_kept(aside: &Path) -> bool {
    std::fs::symlink_metadata(aside).is_ok_and(|m| !m.is_file()) && std::fs::symlink_metadata(delete_marker(aside)).is_err()
}

/// Write the empty file `marker` ([`delete_marker`]).
fn write_delete_marker(marker: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if MARKER_WRITE_FAILS.get() {
        return Err(std::io::Error::other("the volume is full"));
    }
    std::fs::write(marker, b"")
}

/// Delete the folder `aside` (a file there is left), writing its [`delete_marker`] first and
/// removing it once the folder is gone; what the deletion can't remove stays, with a log record.
/// When the marker can't be written, nothing is deleted, so that the folder stays whole.
fn delete_aside(aside: &Path) {
    let marker = delete_marker(aside);
    match std::fs::symlink_metadata(aside) {
        Err(_) => {
            let _ = std::fs::remove_file(&marker);
            return;
        }
        Ok(m) if m.is_file() => return,
        Ok(_) => {}
    }
    if let Err(e) = write_delete_marker(&marker) {
        log::warn!("can't save {}: {e}; {} is left as it is", marker.display(), aside.display());
        return;
    }
    match std::fs::remove_dir_all(aside) {
        Err(e) => log::warn!("can't delete {}: {e}", aside.display()),
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(&marker) {
                log::warn!("can't remove {}: {e}", marker.display());
            }
        }
    }
}

/// Rename the plug-in folder at `aside` back to `path`.
fn move_back(aside: &Path, path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if MOVE_BACK_FAILS.get() {
        return Err(std::io::Error::other("the folder is in use"));
    }
    std::fs::rename(aside, path)
}

/// Move the newest settings and cache that a removal kept ([`aside_paths`], [`is_kept`]) back to
/// the plug-in folder `path` when there is none. When `path` is there, it is used and the kept
/// folders are left as they are, with a log record, until the plug-in's removal deletes them.
/// Called under [`PLUGIN_DATA_LOCK`].
fn restore_removed(path: &Path) -> std::io::Result<()> {
    let asides = aside_paths(path);
    let mut kept = asides.iter().rev().filter(|a| is_kept(a));
    if std::fs::symlink_metadata(path).is_ok() {
        for aside in kept {
            log::warn!("{} is used: the settings and cache in {} are left as they are", path.display(), aside.display());
        }
        return Ok(());
    }
    kept.next().map_or(Ok(()), |aside| move_back(aside, path))
}

/// Where plug-in `id` keeps its settings and cache in the plug-in store `dir`. Settings and cache
/// that a removal kept are moved back first ([`restore_removed`]); when that fails, the failure is
/// logged and there is no storage (its calls fail as unavailable), so that nothing the plug-in
/// writes takes their place. A later attach or load tries again. They are moved in `turn`; when it
/// isn't held, they stay where they are, as when moving them fails.
fn plugin_storage(dir: Option<&Path>, id: &str, turn: &StoreTurn) -> Option<Arc<dyn Storage>> {
    let dir = dir?;
    let data = plugin_data_dir(dir, id);
    if !is_link(&dir.join(PLUGIN_DATA_DIR)) {
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let kept_aside = || std::fs::symlink_metadata(&data).is_err() && aside_paths(&data).iter().any(|a| is_kept(a));
        let restored = if turn.held() || !kept_aside() { restore_removed(&data) } else { Err(std::io::Error::other(STORE_NOT_LOCKED)) };
        if let Err(e) = restored {
            log::warn!("can't move the settings and cache of {} back: {e}", data.display());
            return None;
        }
    }
    let state = PLUGIN_DATA_OPEN.lock().unwrap_or_else(|e| e.into_inner()).entry(data.clone()).or_default().clone();
    Some(Arc::new(PluginStorage { root: dir.to_path_buf(), dir: data, state }))
}

/// A file's size and modified time (`None`: there is no file).
type FileStamp = Option<(u64, SystemTime)>;

/// The stamp of the file at `path`.
fn file_stamp(path: &Path) -> Result<FileStamp, StorageError> {
    match std::fs::metadata(path) {
        Ok(m) => Ok(Some((m.len(), m.modified().map_err(|e| io(&e))?))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(&e)),
    }
}

/// The modified time of the folder at `dir` (`None`: it can't be read, or there is no folder).
fn folder_stamp(dir: &Path) -> Option<SystemTime> {
    std::fs::symlink_metadata(dir).ok()?.modified().ok()
}

/// What every storage made for one plug-in folder shares ([`PLUGIN_DATA_OPEN`]).
#[derive(Default)]
struct PluginData {
    /// Set, under [`PLUGIN_DATA_LOCK`], once the plug-in is removed.
    closed: AtomicBool,
    kept: Mutex<Kept>,
}

/// What the folder's files last held in this process.
#[derive(Default)]
struct Kept {
    settings: Option<KeptSettings>,
    cache: Option<KeptCache>,
}

/// The settings by key, and the settings file's stamp when they were read or written.
struct KeptSettings {
    stamp: FileStamp,
    values: BTreeMap<String, Vec<u8>>,
}

/// The bytes of the cache entries, and the cache folder's stamp when they were added up.
#[derive(Clone, Copy)]
struct KeptCache {
    stamp: Option<SystemTime>,
    bytes: u64,
}

/// The settings and cache of one plug-in, in its folder of the plug-in store (see the module docs).
/// Keys and entry names are [`storage::valid_name`]s; an entry's file name is its name as
/// [`case_safe`] writes it, so that names differing only in case get different files. A link
/// anywhere in the folder's path below the store is refused, and so is a cache file that is not a
/// plain file. The settings stay in memory ([`Kept`]) while the settings file keeps its size and
/// modified time, and every change is written to the file before the call returns. The total
/// `cache_bytes` reports stays in memory while the cache folder keeps its modified time.
pub(crate) struct PluginStorage {
    /// The plug-in store, and the plug-in's folder in it.
    root: PathBuf,
    dir: PathBuf,
    state: Arc<PluginData>,
}

/// The sizes of `files` added up, at most `u64::MAX`: the sizes come from the file system.
fn sum_sizes(files: &[(PathBuf, u64)]) -> u64 {
    files.iter().fold(0, |sum, (_, n)| sum.saturating_add(*n))
}

/// Is `name` the file name of a cache entry? Entry files end in `.bin` and never start with a `.`
/// ([`case_safe`] writes no `.`), while the temporary files of `write_atomic` do.
fn is_entry_file(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| !n.starts_with('.') && n.ends_with(&format!(".{CACHE_EXT}")))
}

/// What a storage call is told when the file system fails.
fn io(e: &std::io::Error) -> StorageError {
    StorageError::Io(e.to_string())
}

/// Does grant `g` cover `plugin`: was it given to this module (the same id and SHA-256) for every
/// permission the module lists? A module with other bytes under the id needs the user's approval
/// again, and so doesn't get the settings and cache kept for the id without it. A grant whose hash
/// is missing or not a valid SHA-256 covers no module.
fn grant_covers(g: &vectorcraft_plugins::permissions::Grant, plugin: &vectorcraft_plugins::Plugin) -> bool {
    let m = plugin.manifest();
    g.covers(&m.id, plugin.hash(), &m.permissions())
}

/// Is `path` a link (a symbolic link or, on Windows, a junction)?
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || reparse_point(&m))
}

#[cfg(windows)]
fn reparse_point(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

impl PluginStorage {
    fn settings_path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    fn cache_dir(&self) -> PathBuf {
        self.dir.join(CACHE_DIR)
    }

    /// The file of the cache entry `name`.
    fn entry_path(&self, name: &str) -> PathBuf {
        self.cache_dir().join(format!("{}.{CACHE_EXT}", case_safe(name)))
    }

    /// Checks that no folder between the store and `path`, nor `path` itself, is a link.
    fn no_links(&self, path: &Path) -> Result<(), StorageError> {
        let mut at = path;
        while at != self.root {
            if is_link(at) {
                return Err(StorageError::Unavailable);
            }
            match at.parent() {
                Some(up) => at = up,
                None => break,
            }
        }
        Ok(())
    }

    /// A turn on the plug-in store ([`lock_store`]), taken before [`PLUGIN_DATA_LOCK`], so that a
    /// window or process changing the same settings or cache waits for this change.
    fn turn(&self) -> Result<std::fs::File, StorageError> {
        lock_store(&self.root).map_err(|e| StorageError::Io(format!("{STORE_NOT_LOCKED}: {e}")))
    }

    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.state.kept.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The settings: those in `kept` when the settings file still has their stamp, else the file's,
    /// which then replace them.
    fn read_settings<'a>(&self, kept: &'a mut Option<KeptSettings>, meter: &mut Meter) -> Result<&'a mut KeptSettings, StorageError> {
        let path = self.settings_path();
        self.no_links(&path)?;
        let stamp = file_stamp(&path)?;
        let held = match kept.take() {
            Some(held) if held.stamp == stamp => held,
            _ => self.load_settings(stamp, meter)?,
        };
        Ok(kept.insert(held))
    }

    /// Reads the settings file, whose stamp is `stamp`. A file that is too large or doesn't parse is
    /// moved aside ([`BAD_SETTINGS_FILE`]) with a log record, and the settings are empty. `meter` is
    /// charged the file's size before it is read.
    fn load_settings(&self, stamp: FileStamp, meter: &mut Meter) -> Result<KeptSettings, StorageError> {
        let path = self.settings_path();
        let Some((size, _)) = stamp else { return Ok(KeptSettings { stamp, values: BTreeMap::new() }) };
        let readable = size <= storage::MAX_SETTINGS_BYTES as u64;
        if readable {
            meter.take(size)?;
        }
        let parsed = readable
            .then(|| read_capped(&path, storage::MAX_SETTINGS_BYTES as u64).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes).ok())
            .and_then(|stored| {
                let decode =
                    |(k, v): (String, String)| if storage::valid_name(&k) { vectorcraft_format::base64_decode(&v).map(|v| (k, v)) } else { None };
                let settings: Option<BTreeMap<String, Vec<u8>>> = stored.into_iter().map(decode).collect();
                settings.filter(|s| s.len() <= storage::MAX_SETTINGS && s.values().all(|v| v.len() <= storage::MAX_SETTING_BYTES))
            });
        match parsed {
            Some(values) => Ok(KeptSettings { stamp, values }),
            None => {
                let aside = self.dir.join(BAD_SETTINGS_FILE);
                log::warn!("the plug-in settings {} don't read: moved to {} and started empty", path.display(), aside.display());
                std::fs::rename(&path, &aside).map_err(|e| io(&e))?;
                Ok(KeptSettings { stamp: None, values: BTreeMap::new() })
            }
        }
    }

    /// Writes the settings in `kept` to the file and records the file's new stamp. When that fails,
    /// or settings were written and the file is gone, `kept` is emptied, so the next call reads the
    /// file again.
    fn save_settings(&self, kept: &mut Option<KeptSettings>, meter: &mut Meter) -> Result<(), StorageError> {
        let Some(held) = kept.as_mut() else { return Ok(()) };
        let written = self.write_settings(&held.values, meter);
        match (written, file_stamp(&self.settings_path())) {
            (Ok(()), Ok(stamp)) if stamp.is_some() || held.values.is_empty() => {
                held.stamp = stamp;
                Ok(())
            }
            (written, _) => {
                *kept = None;
                written
            }
        }
    }

    /// Writes `settings` as the settings file, or removes it when there are none. `meter` is charged
    /// the new file's size before it is written.
    fn write_settings(&self, settings: &BTreeMap<String, Vec<u8>>, meter: &mut Meter) -> Result<(), StorageError> {
        let path = self.settings_path();
        if settings.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(&e)),
                _ => Ok(()),
            };
        }
        let stored: BTreeMap<&str, String> = settings.iter().map(|(k, v)| (k.as_str(), vectorcraft_format::base64_encode(v))).collect();
        let bytes = serde_json::to_vec(&stored).map_err(|e| StorageError::Io(e.to_string()))?;
        if bytes.len() > storage::MAX_SETTINGS_BYTES {
            return Err(StorageError::Full);
        }
        meter.take(bytes.len() as u64)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| io(&e))?;
        self.no_links(&path)?;
        write_atomic(&path, &bytes).map_err(|e| io(&e))
    }

    /// Fails with [`StorageError::Unavailable`] once the storage is closed.
    fn open(&self) -> Result<(), StorageError> {
        if self.state.closed.load(Ordering::Acquire) { Err(StorageError::Unavailable) } else { Ok(()) }
    }

    /// The entries of the cache folder (plain files named as [`Self::entry_path`] names them, which
    /// leaves out the temporary files of writes) and their sizes, and whether the folder holds more
    /// than [`MAX_CACHE_SCAN`] items, in which case only the entries among the first that many are
    /// listed.
    fn cache_files(&self) -> Result<(Vec<(PathBuf, u64)>, bool), StorageError> {
        let dir = self.cache_dir();
        self.no_links(&dir)?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((vec![], false)),
            Err(e) => return Err(io(&e)),
        };
        let mut files = Vec::new();
        for (seen, e) in entries.enumerate() {
            if seen == MAX_CACHE_SCAN {
                return Ok((files, true));
            }
            let e = e.map_err(|e| io(&e))?;
            if is_entry_file(&e.file_name())
                && let Ok(m) = std::fs::symlink_metadata(e.path())
                && m.is_file()
                && !reparse_point(&m)
            {
                files.push((e.path(), m.len()));
            }
        }
        Ok((files, false))
    }
}

impl Storage for PluginStorage {
    fn setting(&self, key: &str, meter: &mut Meter) -> Result<Option<Vec<u8>>, StorageError> {
        // Reading may move a settings file that doesn't read aside.
        let _turn = self.turn()?;
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let mut kept = self.kept();
        Ok(self.read_settings(&mut kept.settings, meter)?.values.get(key).cloned())
    }

    fn set_setting(&self, key: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError> {
        let _turn = self.turn()?;
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let mut kept = self.kept();
        let settings = &mut self.read_settings(&mut kept.settings, meter)?.values;
        #[cfg(test)]
        if let Some(between) = AFTER_SETTINGS_READ.take() {
            between();
        }
        if value.len() > storage::MAX_SETTING_BYTES || (!settings.contains_key(key) && settings.len() >= storage::MAX_SETTINGS) {
            return Err(StorageError::Full);
        }
        if settings.get(key).is_some_and(|old| old == value) {
            return Ok(());
        }
        settings.insert(key.to_string(), value.to_vec());
        self.save_settings(&mut kept.settings, meter)
    }

    fn delete_setting(&self, key: &str, meter: &mut Meter) -> Result<bool, StorageError> {
        let _turn = self.turn()?;
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let mut kept = self.kept();
        if self.read_settings(&mut kept.settings, meter)?.values.remove(key).is_none() {
            return Ok(false);
        }
        self.save_settings(&mut kept.settings, meter).map(|()| true)
    }

    fn cache_entry(&self, name: &str, meter: &mut Meter) -> Result<Option<Vec<u8>>, StorageError> {
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let path = self.entry_path(name);
        self.no_links(&path)?;
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() && m.len() <= storage::MAX_CACHE_ENTRY as u64 => meter.take(m.len())?,
            Ok(_) => return Err(StorageError::Unavailable),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(&e)),
        }
        read_capped(&path, storage::MAX_CACHE_ENTRY as u64).map(Some).map_err(StorageError::Io)
    }

    fn set_cache_entry(&self, name: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError> {
        // The cache's size and count are added up and the entry written in one turn.
        let _turn = self.turn()?;
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let path = self.entry_path(name);
        let (files, overfull) = self.cache_files()?;
        if overfull {
            return Err(StorageError::Full);
        }
        let old = files.iter().find(|(p, _)| *p == path).map(|(_, n)| *n);
        let total = sum_sizes(&files).saturating_sub(old.unwrap_or(0)).saturating_add(value.len() as u64);
        let too_many = old.is_none() && files.len() >= storage::MAX_CACHE_ENTRIES;
        if value.len() > storage::MAX_CACHE_ENTRY || total > storage::MAX_CACHE_BYTES || too_many {
            return Err(StorageError::Full);
        }
        meter.take(value.len() as u64)?;
        std::fs::create_dir_all(self.cache_dir()).map_err(|e| io(&e))?;
        self.no_links(&path)?;
        write_atomic(&path, value).map_err(|e| io(&e))?;
        let stamp = folder_stamp(&self.cache_dir());
        self.kept().cache = stamp.map(|_| KeptCache { stamp, bytes: total });
        Ok(())
    }

    fn remove_cache_entry(&self, name: &str) -> Result<bool, StorageError> {
        let _turn = self.turn()?;
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let path = self.entry_path(name);
        self.no_links(&path)?;
        match std::fs::remove_file(&path) {
            Ok(()) => {
                self.kept().cache = None;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(&e)),
        }
    }

    fn clear_cache(&self) -> Result<u64, StorageError> {
        let _turn = self.turn()?;
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let dir = self.cache_dir();
        self.no_links(&dir)?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(io(&e)),
        };
        // Every plain file, one at a time as the folder is read, past the ones a write looks at too.
        let mut freed = 0_u64;
        for e in entries {
            let path = e.map_err(|e| io(&e))?.path();
            if let Ok(m) = std::fs::symlink_metadata(&path)
                && m.is_file()
                && !reparse_point(&m)
            {
                std::fs::remove_file(&path).map_err(|e| io(&e))?;
                freed = freed.saturating_add(m.len());
            }
        }
        self.kept().cache = Some(KeptCache { stamp: folder_stamp(&dir), bytes: 0 });
        Ok(freed)
    }

    fn cache_bytes(&self) -> u64 {
        if self.open().is_err() {
            return 0;
        }
        let stamp = folder_stamp(&self.cache_dir());
        if let Some(kept) = self.kept().cache.filter(|k| k.stamp == stamp) {
            return kept.bytes;
        }
        let Ok((files, _)) = self.cache_files() else { return 0 };
        let bytes = sum_sizes(&files);
        self.kept().cache = Some(KeptCache { stamp, bytes });
        bytes
    }
}

/// What the grants file of a plug-in store holds.
#[derive(Default)]
struct GrantsFile {
    /// The grants this version can use, by plug-in id. They grant something only when the file is
    /// [`Trust::Signed`].
    grants: BTreeMap<String, vectorcraft_plugins::permissions::Grant>,
    /// The entries it can't use (a permission it doesn't know, an invalid id), as they were.
    other: Vec<serde_json::Value>,
    /// Whether VectorCraft wrote the grants for this folder.
    trust: Trust,
    /// Whether there is a grants file: `false` when there is none, and the file is then empty and
    /// [`Trust::Signed`].
    present: bool,
}

impl GrantsFile {
    /// Whether the file has an entry for the plug-in `id`, usable by this version or not.
    fn names(&self, id: &str) -> bool {
        self.grants.contains_key(id) || self.other.iter().any(|e| e.get("id").and_then(serde_json::Value::as_str) == Some(id))
    }
}

/// Whether a grants file was written by VectorCraft for its folder ([`grant_keys`]).
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Trust {
    /// There is no file, or it is at the generation kept for its folder and carries the MAC of that
    /// generation, its grants and its folder made with the user's key.
    #[default]
    Signed,
    /// It doesn't: an older version wrote it, another program wrote or changed it, it was copied
    /// from another folder or machine, it is an earlier copy or a write whose generation wasn't kept
    /// (its generation is not the one the key store keeps), or the key store holds no key. It grants
    /// nothing.
    Unsigned,
    /// The key store can't be reached, so it is not known which. It grants nothing, and is left as
    /// it is.
    Unknown,
}

/// The version of the grants file this version writes ([`SignedGrants::version`]).
const GRANTS_VERSION: u64 = 1;

/// The grants file as this version writes it: its generation ([`grant_keys::current_generation`]),
/// the grants, and their MAC ([`grant_keys::sign`]) over the generation and the list written as
/// compact JSON.
#[derive(serde::Serialize, serde::Deserialize)]
struct SignedGrants {
    version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
    grants: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mac: Option<String>,
}

/// Tests: a grants file for the plug-in store `dir` that holds `grants` (a JSON list, written as it
/// is) at the generation kept for `dir`, with their MAC made with the key in use, as VectorCraft
/// writes one.
#[cfg(test)]
pub(crate) fn signed_grants_file(dir: &Path, grants: &str) -> Vec<u8> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(grants).unwrap_or_default();
    let key = grant_keys::signing_key().unwrap_or_else(|e| panic!("no key: {e}"));
    let folder = plugin_store_key(dir);
    let generation = grant_keys::current_generation(&folder).unwrap_or_else(|e| panic!("no generation: {e}"));
    let canonical = serde_json::to_vec(&entries).unwrap_or_default();
    let mac = grant_keys::sign(&key, &folder, generation, &canonical).unwrap_or_default();
    format!(r#"{{"version":{GRANTS_VERSION},"generation":{generation},"grants":{grants},"mac":"{mac}"}}"#).into_bytes()
}

/// Tests: the entries of the grants file `bytes`, signed or not.
#[cfg(test)]
pub(crate) fn grant_entries(bytes: &[u8]) -> Vec<serde_json::Value> {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Array(entries)) => entries,
        Ok(serde_json::Value::Object(mut file)) => match file.remove("grants") {
            Some(serde_json::Value::Array(entries)) => entries,
            _ => vec![],
        },
        _ => vec![],
    }
}

/// Whether `signed`, read from the grants file of the plug-in store `dir`, is at the generation kept
/// for `dir` and carries the MAC of that generation, its grants and `dir` made with the user's key.
/// A file at another generation, one behind or one ahead of the kept one included, is
/// [`Trust::Unsigned`].
fn trust_of(dir: &Path, signed: &SignedGrants) -> Trust {
    let (Some(tag), Some(generation), true) = (&signed.mac, signed.generation, signed.version == GRANTS_VERSION) else { return Trust::Unsigned };
    let Ok(grants) = serde_json::to_vec(&signed.grants) else { return Trust::Unsigned };
    let folder = plugin_store_key(dir);
    let kept = grant_keys::current_key().and_then(|key| key.map(|key| Ok((key, grant_keys::current_generation(&folder)?))).transpose());
    match kept {
        Ok(Some((key, kept))) if generation == kept && grant_keys::verify(&key, &folder, generation, &grants, tag) => Trust::Signed,
        Ok(_) => Trust::Unsigned,
        Err(e) => {
            log::warn!("can't check the stored plug-in grants {}: {e}", dir.join(GRANTS_FILE).display());
            Trust::Unknown
        }
    }
}

/// Write `entries` as the grants file of the plug-in store `dir`, pretty-printed, or compact when
/// that is larger than [`MAX_GRANTS_FILE`] → whether it went there; the file is not written when the
/// key or the generation can't be had or it is too large either way. When `signed`, the generation
/// is raised first ([`raise_generation`]), the file is written at the generation after the raised one
/// with its MAC, and the key store keeps that generation once the file is written. Every signed write
/// is then at a generation no file was signed at before, a write whose generation wasn't kept
/// included, since that one is at most one past the generation the key store kept. A file too large
/// either way is found before the raise, and then nothing changes. The kept generation only goes up:
/// when the file isn't written after the raise, the generation stays raised, so the file in place
/// grants nothing and its plug-ins ask again. When the file's generation can't be kept, the file is
/// at a generation the key store doesn't hold, so it grants nothing either. Both report a failure.
/// Call it in the store's turn ([`StoreTurn`]).
fn put_grants(dir: &Path, entries: Vec<serde_json::Value>, signed: bool) -> bool {
    let path = dir.join(GRANTS_FILE);
    let folder = plugin_store_key(dir);
    if !signed {
        return match grants_bytes(serde_json::to_value(entries).ok()) {
            Some(bytes) => put(dir, GRANTS_FILE, &bytes),
            None => {
                log::warn!("not updating the stored plug-in grants {}: {}", path.display(), too_large(MAX_GRANTS_FILE));
                false
            }
        };
    }
    let key = match grant_keys::signing_key() {
        Ok(key) => key,
        Err(e) => {
            log::warn!("not updating the stored plug-in grants {}: {e}", path.display());
            return false;
        }
    };
    // A file too large to keep is found before the raise, at the generation it would be written at
    // (three past the kept one, [`raise_generation`]) and a MAC of its length, so the file in place
    // keeps its grants when nothing can be written. When the kept generation can't be read, the raise
    // fails too.
    let expected = grant_keys::current_generation(&folder).ok().and_then(|kept| kept.checked_add(3)).unwrap_or(u64::MAX);
    let mut file =
        SignedGrants { version: GRANTS_VERSION, generation: Some(expected), grants: entries, mac: Some("0".repeat(2 * grant_keys::KEY_LEN)) };
    if grants_bytes(serde_json::to_value(&file).ok()).is_none() {
        log::warn!("not updating the stored plug-in grants {}: {}", path.display(), too_large(MAX_GRANTS_FILE));
        return false;
    }
    let Some(raised) = raise_generation(dir) else { return false };
    let Some(next) = raised.checked_add(1) else {
        log::warn!("not updating the stored plug-in grants {}: the generation can't be raised", path.display());
        return false;
    };
    file.generation = Some(next);
    file.mac = serde_json::to_vec(&file.grants).ok().and_then(|grants| grant_keys::sign(&key, &folder, next, &grants));
    let Some(bytes) = file.mac.as_ref().and_then(|_| grants_bytes(serde_json::to_value(&file).ok())) else {
        log::warn!("not updating the stored plug-in grants {}: {}", path.display(), too_large(MAX_GRANTS_FILE));
        return false;
    };
    put(dir, GRANTS_FILE, &bytes) && keep_generation(dir, &folder, next)
}

/// `file` written as the bytes of a grants file: pretty-printed, or compact when that is larger than
/// [`MAX_GRANTS_FILE`]; `None` when that is too, since the file is read back only up to its cap.
fn grants_bytes(file: Option<serde_json::Value>) -> Option<Vec<u8>> {
    let fits = |bytes: &Vec<u8>| bytes.len() as u64 <= MAX_GRANTS_FILE;
    file.and_then(|file| serde_json::to_vec_pretty(&file).ok().filter(fits).or_else(|| serde_json::to_vec(&file).ok().filter(fits)))
}

/// Keep `generation` for the grants file of the plug-in store `dir` (whose folder is `folder`) →
/// whether the key store kept it; a failure is logged.
fn keep_generation(dir: &Path, folder: &Path, generation: u64) -> bool {
    match grant_keys::set_current_generation(folder, generation) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("the stored plug-in grants {} grant nothing: their generation wasn't kept ({e})", dir.join(GRANTS_FILE).display());
            false
        }
    }
}

/// Remove the signed grants file of the plug-in store `dir` → whether it is gone. Its generation is
/// raised first, so that no copy of the file grants anything once it is gone. The generation stays
/// raised when the file can't be removed: the file left in place then grants nothing, and its
/// plug-ins ask again. The file holds no grant but the one being removed. Nothing is removed when
/// the generation can't be raised. Call it in the store's turn ([`StoreTurn`]).
fn take_signed_grants(dir: &Path) -> bool {
    raise_generation(dir).is_some() && take(dir, GRANTS_FILE)
}

/// Keep a generation two past the one kept for the grants file of the plug-in store `dir`, so that
/// no signed copy of the file made before grants anything → the generation kept now; `None`, with a
/// log record, when the key store can't read or keep it or the generation is within two of the
/// largest 64-bit value. A signed write whose generation wasn't kept is at the generation after the
/// kept one ([`put_grants`]), so the new generation is past it too. The generation comes from the
/// key store alone, never from a file, which anyone can edit, and it is never lowered again.
fn raise_generation(dir: &Path) -> Option<u64> {
    let folder = plugin_store_key(dir);
    let raised = grant_keys::current_generation(&folder).and_then(|generation| {
        // A signed write goes one past the raised generation, so that one must fit too.
        let next = generation.checked_add(3).and(generation.checked_add(2));
        let next = next.ok_or_else(|| grant_keys::KeyStoreError("the generation can't be raised".into()))?;
        grant_keys::set_current_generation(&folder, next).map(|()| next)
    });
    match raised {
        Ok(generation) => Some(generation),
        Err(e) => {
            log::warn!("not changing the stored plug-in grants {}: {e}", dir.join(GRANTS_FILE).display());
            None
        }
    }
}

/// Make sure no signed copy of an earlier grants file of the plug-in store `dir` grants anything →
/// whether none can: the generation is raised ([`raise_generation`]) when the key store holds a key,
/// and left as it is when it holds none, since no file is then signed. `false`, with a log record,
/// when the key store can't read or keep it. Call it in the store's turn ([`StoreTurn`]).
fn retire_signed_copies(dir: &Path) -> bool {
    match grant_keys::current_key() {
        Ok(None) => true,
        Ok(Some(_)) => raise_generation(dir).is_some(),
        Err(e) => {
            log::warn!("not changing the stored plug-in grants {}: {e}", dir.join(GRANTS_FILE).display());
            false
        }
    }
}

/// Why the grants file of a plug-in store gave no grants.
struct GrantsError {
    /// Opening or reading the file failed, so what it holds is unknown. When `false`, the file is
    /// too large or doesn't hold a list, and approves nothing.
    unread: bool,
    /// The detail for a log record.
    why: String,
}

/// The grants file of the plug-in store `dir`: empty when there is none, an error when it is too
/// large, can't be read or doesn't hold a list. A list without a MAC (as an older version wrote it),
/// or with one the user's key didn't make for `dir`, is [`Trust::Unsigned`] ([`trust_of`]). An
/// entry that doesn't parse as a grant, or names an invalid id, is kept in [`GrantsFile::other`]
/// with a log record.
fn read_grants(dir: &Path) -> Result<GrantsFile, GrantsError> {
    let path = dir.join(GRANTS_FILE);
    let bytes = match read_past_cap(&path, MAX_GRANTS_FILE) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(GrantsFile::default()),
        Err(e) => return Err(GrantsError { unread: true, why: e.to_string() }),
    };
    if bytes.len() as u64 > MAX_GRANTS_FILE {
        return Err(GrantsError { unread: false, why: too_large(MAX_GRANTS_FILE) });
    }
    let not_a_list = |why: String| GrantsError { unread: false, why };
    let (entries, trust) = match serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| not_a_list(e.to_string()))? {
        serde_json::Value::Array(entries) => (entries, Trust::Unsigned),
        serde_json::Value::Object(_) => {
            let signed = serde_json::from_slice::<SignedGrants>(&bytes).map_err(|e| not_a_list(e.to_string()))?;
            let trust = trust_of(dir, &signed);
            (signed.grants, trust)
        }
        _ => return Err(not_a_list("not a list of grants".into())),
    };
    if trust == Trust::Unsigned {
        log::warn!(
            "the stored plug-in grants {} were not written by VectorCraft for this folder: the plug-ins that list permissions ask for them again",
            path.display()
        );
    }
    let mut file = GrantsFile { trust, present: true, ..GrantsFile::default() };
    let mut first_error = None;
    for entry in entries {
        let why = match serde_json::from_value::<vectorcraft_plugins::permissions::Grant>(entry.clone()) {
            Ok(g) if vectorcraft_plugins::manifest::valid_id(&g.id) => {
                file.grants.insert(g.id.clone(), g);
                continue;
            }
            Ok(g) => format!("invalid id `{}`", g.id.escape_debug()),
            Err(e) => e.to_string(),
        };
        first_error.get_or_insert(why);
        file.other.push(entry);
    }
    if let Some(why) = first_error {
        log::warn!("skipping {} stored plug-in grants in {} (the first: {why})", file.other.len(), path.display());
    }
    Ok(file)
}

/// The grants kept in the plug-in store `dir` that this version can use, by plug-in id: none when
/// there is no grants file, when VectorCraft didn't write it for `dir` or that can't be checked
/// ([`Trust`]), and none, with a log record, when it is too large, can't be read or doesn't hold a
/// list.
fn stored_grants(dir: &Path) -> BTreeMap<String, vectorcraft_plugins::permissions::Grant> {
    signed_grants(dir, &read_grants(dir))
}

/// [`stored_grants`] from `file`, the grants file of the plug-in store `dir` as [`read_grants`]
/// read it.
fn signed_grants(dir: &Path, file: &Result<GrantsFile, GrantsError>) -> BTreeMap<String, vectorcraft_plugins::permissions::Grant> {
    match file {
        Ok(file) if file.trust == Trust::Signed => file.grants.clone(),
        Ok(_) => BTreeMap::new(),
        Err(e) => {
            log::warn!("skipping the stored plug-in grants {}: {}", dir.join(GRANTS_FILE).display(), e.why);
            BTreeMap::new()
        }
    }
}

/// The presets of the files in `dir`: valid ones only, no name twice (any case), none called like a
/// built-in preset, at most [`MAX_STORE_FILES`].
fn load_presets(dir: &Path) -> Vec<PdfPreset> {
    let mut presets: Vec<PdfPreset> = vec![];
    for path in list_files(dir, PRESET_FORMAT) {
        let file = read_capped(&path, MAX_PRESET_FILE).and_then(|bytes| serde_json::from_slice::<PresetFile>(&bytes).map_err(|e| e.to_string()));
        let file = match file {
            Ok(f) if f.format == PRESET_FORMAT => f,
            Ok(f) => {
                log::warn!("skipping {}: not a PDF presets file (format `{}`)", path.display(), f.format);
                continue;
            }
            Err(e) => {
                log::warn!("skipping the stored PDF preset {}: {e}", path.display());
                continue;
            }
        };
        for mut preset in file.presets {
            let name = preset.name.trim().to_string();
            let usable = !name.is_empty()
                && builtin_preset(&name, APP).is_none()
                && !presets.iter().any(|q| q.name.eq_ignore_ascii_case(&name))
                && preset.settings.check_values().is_ok()
                && presets.len() < MAX_STORE_FILES;
            if usable {
                preset.name = name;
                preset.settings.clear_passwords();
                presets.push(preset);
            } else {
                log::warn!("skipping the PDF preset `{name}` of {}", path.display());
            }
        }
    }
    presets
}
