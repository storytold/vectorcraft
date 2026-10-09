//! Data Recovery: copies of modified documents kept in a recovery store, so a crash loses at most
//! the last few minutes of work.
//!
//! - `file.recovery.save` writes a copy of every modified document changed since its last copy
//!   (the app runs it every Preferences → File Handling → Data Recovery interval, in the
//!   background like Background Save: [`jobs`] snapshots, [`RecoveryJob::write`] encodes and
//!   stores, [`RecoveryJob::finish`] records the copy).
//! - A copy is removed when its document is saved, reverted or closed ([`forget`]), so copies
//!   left in the store after the app has quit are what a crash left behind:
//!   `file.recovery.list` lists them, `file.recovery.restore` opens them as
//!   "<name> [Recovered]" (modified, never saved, the original path remembered for Save As) and
//!   `file.recovery.discard` deletes them.
//!
//! Every running app (each browser tab) keeps its copies in an area of its own, `<area>/…`, and
//! holds it while it runs: a lock on the area for its lifetime where the store has locks (folders:
//! [`FolderStore`]), else a heartbeat it refreshes ([`heartbeat`]; browser storage). Only areas
//! nobody holds any more (their app is gone) are offered, and while their copies are restored or
//! discarded the area is held, so two apps launched together never both take it.
//!
//! Each copy is two entries: `<area>/<file>.vectorcraft` (the native document, written as a save
//! writes it) and `<area>/<file>.json` (its title, original path and format, and when it was
//! written). The store is a folder on the desktop ([`FolderStore`]: the `recoveryFolder`
//! preference, else the folder the app sets), browser storage on the web, or any
//! [`RecoveryStore`] the host installs (tests use [`MemoryStore`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::Document;

use super::fileio::{self, SAVE_FORMATS, SaveJob, SaveMode, SavePlan};
use super::*;
use crate::DocState;

/// Documents with more objects than this are complex: with Preferences → Turn off Data Recovery
/// for complex documents, they get no recovery copy.
pub const COMPLEX_OBJECTS: usize = 20_000;

/// The extension of a copy's document entry.
const DOC_EXT: &str = ".vectorcraft";
/// The extension of a copy's details entry.
const META_EXT: &str = ".json";
/// The heartbeat entry of an area in a store without locks: when its app last said it runs (Unix
/// seconds, as text).
const BEAT: &str = "heartbeat";
/// The least time without a heartbeat after which an area's app counts as gone (seconds); else
/// three Data Recovery intervals.
const MIN_STALE: i64 = 180;
/// How often a running app refreshes its heartbeat (seconds).
pub const HEARTBEAT_EVERY: f64 = 60.0;

/// Holds an area while it lives (dropping it releases the lock).
pub type Hold = Box<dyn std::any::Any + Send + Sync>;

/// What [`RecoveryStore::lock`] got.
pub enum Lock {
    /// The area is ours until the hold is dropped.
    Held(Hold),
    /// A running app holds it.
    Busy,
    /// The store has no locks: areas are held with heartbeats.
    Unsupported,
}

/// Where recovery copies are kept: named entries of bytes, `<area>/<name>`. Called from the UI
/// thread and the background worker.
pub trait RecoveryStore: Send + Sync {
    /// The names of the entries (`<area>/<name>`).
    fn list(&self) -> std::result::Result<Vec<String>, String>;
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String>;
    /// Create or replace an entry (a failed write must not leave a damaged entry behind).
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String>;
    /// Delete an entry (one that isn't there is no error).
    fn remove(&self, name: &str) -> std::result::Result<(), String>;
    /// Where the entries are, as the user knows it (a folder, "browser storage").
    fn location(&self) -> String;
    /// The areas in the store (default: those with entries).
    fn areas(&self) -> std::result::Result<Vec<String>, String> {
        let names = self.list()?;
        let set: BTreeSet<String> = names.iter().filter_map(|n| n.split_once('/')).map(|(a, _)| a.to_string()).collect();
        Ok(set.into_iter().collect())
    }
    /// Hold area `area` exclusively, if the store has locks (default: it hasn't).
    fn lock(&self, _area: &str) -> std::result::Result<Lock, String> {
        Ok(Lock::Unsupported)
    }
    /// Tidy away area `area` once it holds no copies and nobody holds it (its lock file and folder).
    fn remove_area(&self, _area: &str) {}
    /// The clock heartbeats are judged by (Unix seconds; `None`: none, so no area counts as gone).
    fn now(&self) -> Option<i64> {
        vectorcraft_doc::metadata::now_unix()
    }
}

/// A store in memory (tests, and hosts without storage). Areas are locked in memory, so sessions
/// sharing one store behave like apps sharing a folder; [`MemoryStore::without_locks`] uses
/// heartbeats like browser storage, judged by a clock tests set ([`MemoryStore::set_now`]).
pub struct MemoryStore {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    held: Arc<Mutex<BTreeSet<String>>>,
    locks: bool,
    now: Mutex<Option<i64>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self { entries: Default::default(), held: Default::default(), locks: true, now: Mutex::new(Some(1_000_000)) }
    }
}

impl MemoryStore {
    /// A store without locks: areas are held with heartbeats.
    pub fn without_locks() -> Self {
        Self { locks: false, ..Self::default() }
    }

    /// Set the clock heartbeats are judged by.
    pub fn set_now(&self, t: i64) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// An area of a [`MemoryStore`] held until dropped.
struct MemoryHold {
    held: Arc<Mutex<BTreeSet<String>>>,
    area: String,
}

impl Drop for MemoryHold {
    fn drop(&mut self) {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.area);
    }
}

impl RecoveryStore for MemoryStore {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        Ok(self.entries().keys().cloned().collect())
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.entries().get(name).cloned().ok_or_else(|| format!("{name}: no such recovery entry"))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        self.entries().insert(name.to_string(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.entries().remove(name);
        Ok(())
    }
    fn location(&self) -> String {
        "memory".into()
    }
    fn lock(&self, area: &str) -> std::result::Result<Lock, String> {
        if !self.locks {
            return Ok(Lock::Unsupported);
        }
        if !self.held.lock().unwrap_or_else(|e| e.into_inner()).insert(area.to_string()) {
            return Ok(Lock::Busy);
        }
        Ok(Lock::Held(Box::new(MemoryHold { held: self.held.clone(), area: area.to_string() })))
    }
    fn now(&self) -> Option<i64> {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A folder of files (desktop): an area is a sub-folder whose `.lock` file its app keeps locked
/// while it runs. Created when the first copy is written.
#[cfg(not(target_arch = "wasm32"))]
pub struct FolderStore(std::path::PathBuf);

/// The lock file of a [`FolderStore`] area.
#[cfg(not(target_arch = "wasm32"))]
const LOCK_FILE: &str = ".lock";

#[cfg(not(target_arch = "wasm32"))]
impl FolderStore {
    pub fn new(folder: impl Into<std::path::PathBuf>) -> Self {
        Self(folder.into())
    }

    /// Is `part` a plain file or folder name (no separators, not hidden)?
    fn plain(part: &str) -> bool {
        !part.starts_with('.') && std::path::Path::new(part).file_name().is_some_and(|f| f == part)
    }

    /// The folder of area `area`.
    fn area(&self, area: &str) -> std::result::Result<std::path::PathBuf, String> {
        if !Self::plain(area) {
            return Err(format!("`{area}` is no recovery area name"));
        }
        Ok(self.0.join(area))
    }

    /// The file of entry `name` (`<area>/<file>`: anything else is refused).
    fn file(&self, name: &str) -> std::result::Result<std::path::PathBuf, String> {
        match name.split_once('/') {
            Some((area, file)) if Self::plain(file) => Ok(self.area(area)?.join(file)),
            _ => Err(format!("`{name}` is no recovery entry name")),
        }
    }

    /// The plain names of the files (or, `dirs`, folders) in `dir`; none when it doesn't exist.
    fn names(dir: &std::path::Path, dirs: bool) -> std::result::Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        };
        Ok(entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| if dirs { t.is_dir() } else { t.is_file() }))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            // Lock files, and files left by an interrupted atomic write.
            .filter(|n| Self::plain(n))
            .collect())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RecoveryStore for FolderStore {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        let mut out = vec![];
        for area in self.areas()? {
            out.extend(Self::names(&self.0.join(&area), false)?.into_iter().map(|f| format!("{area}/{f}")));
        }
        Ok(out)
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        let f = self.file(name)?;
        std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        let f = self.file(name)?;
        if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        vectorcraft_format::write_atomic(&f, bytes).map_err(|e| format!("{}: {e}", f.display()))
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        let f = self.file(name)?;
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", f.display())),
            _ => Ok(()),
        }
    }
    fn location(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn areas(&self) -> std::result::Result<Vec<String>, String> {
        Self::names(&self.0, true)
    }
    fn lock(&self, area: &str) -> std::result::Result<Lock, String> {
        let dir = self.area(area)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Lock::Held(Box::new(file))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(Lock::Busy),
            Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
        }
    }
    fn remove_area(&self, area: &str) {
        // Best effort: whatever is left is tidied at a later launch. A folder that isn't empty
        // (copies came back meanwhile) stays.
        if let Ok(dir) = self.area(area) {
            let _ = std::fs::remove_file(dir.join(LOCK_FILE));
            let _ = std::fs::remove_dir(&dir);
        }
    }
}

/// The area this session keeps its copies in, while it holds it.
struct Own {
    /// The store it is in (copies are removed from there even after the preference moves on).
    store: Arc<dyn RecoveryStore>,
    area: String,
    /// The lock held for the session's lifetime; none in stores with heartbeats.
    hold: Option<Hold>,
}

/// The session's recovery store settings and its own area ([`crate::Session::recovery`]).
#[derive(Default)]
pub struct Recovery {
    /// A store the host installed (the web's browser storage, tests): wins over folders.
    store: Option<Arc<dyn RecoveryStore>>,
    /// The folder the desktop app keeps copies in when the `recoveryFolder` preference is empty.
    default_folder: Option<String>,
    own: Option<Own>,
    /// Gone apps' areas this session took over by heartbeat (stores without locks), so its own
    /// fresh heartbeat there doesn't make them look running.
    adopted: BTreeSet<String>,
}

impl Recovery {
    /// Keep copies in `store` (instead of a folder).
    pub fn set_store(&mut self, store: Arc<dyn RecoveryStore>) {
        self.store = Some(store);
    }

    /// The folder copies go to while the `recoveryFolder` preference is empty (desktop; `None`:
    /// none, so Data Recovery is off until the preference names one).
    pub fn set_default_folder(&mut self, folder: Option<String>) {
        self.default_folder = folder;
    }

    /// This session's area, once it has written a copy.
    pub fn own_area(&self) -> Option<&str> {
        self.own.as_ref().map(|o| o.area.as_str())
    }
}

/// The store copies go to now (see [`Recovery`]), if any.
pub fn store(s: &Session) -> Option<Arc<dyn RecoveryStore>> {
    if let Some(st) = &s.recovery.store {
        return Some(st.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let folder = Some(s.prefs.recovery_folder.trim()).filter(|f| !f.is_empty()).map(str::to_string).or_else(|| s.recovery.default_folder.clone());
        folder.map(|f| Arc::new(FolderStore::new(f)) as Arc<dyn RecoveryStore>)
    }
    #[cfg(target_arch = "wasm32")]
    None
}

fn has_store(s: &Session) -> std::result::Result<(), String> {
    match store(s) {
        Some(_) => Ok(()),
        None => Err("Data Recovery has nowhere to keep copies: set Preferences → File Handling → Data Recovery → Folder (recoveryFolder)".into()),
    }
}

fn need_store(s: &Session, cmd: &str) -> Result<Arc<dyn RecoveryStore>> {
    store(s).ok_or_else(|| bad(cmd, has_store(s).err().unwrap_or_default()))
}

/// Seconds without a heartbeat after which an area's app counts as gone: three Data Recovery
/// intervals, at least [`MIN_STALE`].
fn stale_after(s: &Session) -> i64 {
    (i64::from(s.prefs.autosave_interval.max(1)) * 180).max(MIN_STALE)
}

/// Record that `area`'s app runs (stores without locks).
fn beat(store: &dyn RecoveryStore, area: &str) -> std::result::Result<(), String> {
    let now = store.now().ok_or("no clock to keep a heartbeat by")?;
    store.write(&format!("{area}/{BEAT}"), now.to_string().as_bytes())
}

/// Has `area`'s app said it runs within `stale` seconds (stores without locks)? Without a clock
/// every app counts as running.
fn beating(store: &dyn RecoveryStore, area: &str, stale: i64) -> bool {
    let Some(now) = store.now() else { return true };
    let at = store.read(&format!("{area}/{BEAT}")).ok().and_then(|b| String::from_utf8(b).ok()).and_then(|t| t.trim().parse::<i64>().ok());
    at.is_some_and(|t| now.saturating_sub(t) <= stale)
}

/// Refresh this session's heartbeat in a store without locks (the app does this every
/// [`HEARTBEAT_EVERY`] seconds). Best effort: a missed beat only lets another app offer the copies
/// once three intervals have passed.
pub fn heartbeat(s: &Session) {
    if let Some(own) = s.recovery.own.as_ref().filter(|o| o.hold.is_none()) {
        let _ = beat(own.store.as_ref(), &own.area);
    }
}

/// This session's area in `store`, taken now if it has none there yet: a fresh area, held for the
/// session's lifetime (locked, or with a heartbeat).
pub fn claim(s: &mut Session, store: &Arc<dyn RecoveryStore>) -> Result<String> {
    if let Some(own) = &s.recovery.own {
        if own.store.location() == store.location() {
            return Ok(own.area.clone());
        }
        // The store moved (the preference names another folder): copies start again there.
        release(s);
    }
    let taken = store.areas().map_err(EngineError::Other)?;
    let base = store.now().unwrap_or(0);
    for n in 1..=1000 {
        let area = format!("{base}-{n}");
        if taken.contains(&area) {
            continue;
        }
        let hold = match store.lock(&area).map_err(EngineError::Other)? {
            Lock::Held(h) => Some(h),
            Lock::Busy => continue,
            Lock::Unsupported => {
                beat(store.as_ref(), &area).map_err(EngineError::Other)?;
                None
            }
        };
        s.recovery.own = Some(Own { store: store.clone(), area: area.clone(), hold });
        return Ok(area);
    }
    Err(EngineError::Other("no free recovery area".into()))
}

/// Give up this session's area: its copies, heartbeat and lock go (quitting with nothing unsaved).
pub fn release(s: &mut Session) {
    for st in &mut s.docs {
        st.recovery = None;
    }
    let Some(Own { store, area, hold }) = s.recovery.own.take() else { return };
    let mine = |n: &String| n.split_once('/').is_some_and(|(a, _)| a == area);
    for name in store.list().unwrap_or_default().iter().filter(|n| mine(n)) {
        let _ = store.remove(name);
    }
    drop(hold);
    store.remove_area(&area);
}

/// A document's recovery copy: its entry and the document it holds.
#[derive(Clone, Debug)]
pub struct RecoveryCopy {
    /// The copy's name in the store (`<area>/<name>`, without extension).
    pub file: String,
    /// The document as last copied (unchanged since: no new copy needed).
    pub doc: Arc<Document>,
}

/// What a copy's details entry says.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Meta {
    title: String,
    /// The document's file when it was copied, if it had one.
    path: Option<String>,
    /// The format Save writes for it ([`SAVE_FORMATS`]).
    format: String,
    /// When the copy was written (Unix seconds; none without a clock).
    saved: Option<i64>,
}

fn read_meta(store: &dyn RecoveryStore, file: &str) -> Meta {
    store.read(&format!("{file}{META_EXT}")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// A copy of one document, snapshotted and ready to encode and store anywhere (the background
/// worker too): [`jobs`], then [`RecoveryJob::write`], then [`RecoveryJob::finish`].
#[derive(Clone)]
pub struct RecoveryJob {
    /// The document's [`DocState::uid`].
    pub uid: u64,
    /// The copy's name in the store (`<area>/<name>`).
    pub file: String,
    /// The document's title (status messages).
    pub title: String,
    meta: Meta,
    save: SaveJob,
}

impl RecoveryJob {
    /// Encode the snapshot and store it with its details → `{file, title, bytes}`.
    pub fn write(&self, store: &dyn RecoveryStore) -> std::result::Result<Value, String> {
        let enc = self.save.encode().map_err(|e| e.to_string())?;
        let bytes = enc.files.first().map(|f| f.1.as_slice()).ok_or("nothing encoded")?;
        let meta = serde_json::to_vec_pretty(&self.meta).map_err(|e| e.to_string())?;
        store.write(&format!("{}{DOC_EXT}", self.file), bytes)?;
        store.write(&format!("{}{META_EXT}", self.file), &meta)?;
        Ok(json!({ "file": self.file, "title": self.title, "bytes": bytes.len() }))
    }

    /// Once stored: the document (if still open) remembers the copy, so it isn't written again
    /// while unchanged and goes when the document is saved or closed. A document saved or closed
    /// meanwhile has nothing left to recover: its copy goes at once.
    pub fn finish(self, s: &mut Session) {
        let snapshot = self.save.snapshot().clone();
        let copy = RecoveryCopy { file: self.file.clone(), doc: snapshot };
        match s.document_mut(self.uid).filter(|st| st.is_dirty()) {
            Some(st) => st.recovery = Some(copy),
            None => remove_copy(s, &self.file),
        }
    }
}

/// How many objects `doc` has (its complexity).
fn object_count(doc: &Document) -> usize {
    doc.layers.iter().map(|l| l.count()).sum()
}

/// Why document `st` gets no recovery copy now, if it doesn't: unchanged since its last copy (or
/// never changed), or complex while the preference turns Data Recovery off for those.
pub fn skip_reason(s: &Session, st: &DocState) -> Option<&'static str> {
    if !st.is_dirty() {
        return Some("no unsaved changes");
    }
    if st.recovery.as_ref().is_some_and(|c| Arc::ptr_eq(&c.doc, &st.doc)) {
        return Some("unchanged since its last recovery copy");
    }
    if s.prefs.recovery_off_for_complex && object_count(&st.doc) > COMPLEX_OBJECTS {
        return Some("complex document (Data Recovery is off for complex documents)");
    }
    None
}

/// A file name part from a title: letters, digits, `-`, `_` and spaces (others as `-`).
fn safe_stem(title: &str) -> String {
    let stem: String = fileio::file_stem(title).chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '-' }).collect();
    let stem = stem.trim().chars().take(60).collect::<String>();
    if stem.is_empty() { "Untitled".into() } else { stem }
}

/// The copies among store entries `names`, each once, as `(area, "<area>/<name>")`.
fn copies_in(names: &[String]) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> =
        names.iter().filter_map(|n| n.strip_suffix(DOC_EXT)).filter_map(|f| f.split_once('/').map(|(a, _)| (a.to_string(), f.to_string()))).collect();
    v.sort();
    v.dedup();
    v
}

/// A copy name `<area>/<stem>-<n>` none of `taken` has.
fn fresh_name(area: &str, stem: &str, taken: &[String]) -> String {
    (1..=10_000).map(|n| format!("{area}/{stem}-{n}")).find(|f| !taken.contains(f)).unwrap_or_else(|| format!("{area}/{stem}-x"))
}

/// The jobs for every document that needs a copy now, in this session's area of `store` (taken
/// when it has none and a copy is needed) → (jobs, skipped: `[{title, reason}]` for modified
/// documents that get none).
pub fn jobs(s: &mut Session, store: &Arc<dyn RecoveryStore>) -> Result<(Vec<RecoveryJob>, Vec<Value>)> {
    let skipped: Vec<Value> = s
        .documents()
        .iter()
        .filter(|st| st.is_dirty())
        .filter_map(|st| skip_reason(s, st).map(|why| json!({ "title": st.title(), "reason": why })))
        .collect();
    if s.documents().iter().all(|st| skip_reason(s, st).is_some()) {
        return Ok((vec![], skipped));
    }
    let area = claim(s, store)?;
    let mut taken: Vec<String> =
        copies_in(&store.list().map_err(EngineError::Other)?).into_iter().filter(|(a, _)| *a == area).map(|(_, f)| f).collect();
    taken.extend(s.documents().iter().filter_map(|d| d.recovery.as_ref()).map(|c| c.file.clone()));
    let native = fileio::format("vectorcraft").ok_or_else(|| EngineError::Other("no native format".into()))?;
    let saved = store.now();
    let mut out = vec![];
    for st in s.documents().iter().filter(|st| skip_reason(s, st).is_none()) {
        let title = st.title();
        let file = match &st.recovery {
            Some(c) => c.file.clone(),
            None => {
                let f = fresh_name(&area, &safe_stem(&title), &taken);
                taken.push(f.clone());
                f
            }
        };
        // Written as a save writes it, links relative to the document's own file.
        let plan = SavePlan {
            mode: SaveMode::Copy,
            path: st.path.clone(),
            format: native,
            options: Default::default(),
            name: format!("{file}{DOC_EXT}"),
            folder: None,
            modified: None,
        };
        let save = fileio::job_for(s, st, plan)?;
        let meta = Meta { title: title.clone(), path: st.path.clone(), format: st.format.to_string(), saved };
        out.push(RecoveryJob { uid: st.uid, file, title, meta, save });
    }
    Ok((out, skipped))
}

/// Remove copy `file` from this session's area. Best effort: a copy that can't be removed is
/// offered again after the next launch, which loses nothing.
fn remove_copy(s: &Session, file: &str) {
    if let Some(own) = &s.recovery.own {
        let _ = own.store.remove(&format!("{file}{DOC_EXT}"));
        let _ = own.store.remove(&format!("{file}{META_EXT}"));
    }
}

/// Document `uid` was saved, reverted or closed: its recovery copy goes.
pub fn forget(s: &mut Session, uid: u64) {
    let Some(copy) = s.document_mut(uid).and_then(|st| st.recovery.take()) else { return };
    remove_copy(s, &copy.file);
}

/// Quitting with nothing unsaved: the copies of every open document go, with this session's area.
pub fn forget_all(s: &mut Session) {
    release(s);
}

/// The copies of documents with no unsaved changes left (undone back to the saved state) go.
pub fn forget_clean(s: &mut Session) {
    let uids: Vec<u64> = s.documents().iter().filter(|d| !d.is_dirty() && d.recovery.is_some()).map(|d| d.uid).collect();
    for uid in uids {
        forget(s, uid);
    }
}

fn save(s: &mut Session, _: &Value) -> Result<Value> {
    let store = need_store(s, "file.recovery.save")?;
    let (jobs, skipped) = jobs(s, &store)?;
    let mut saved = vec![];
    for j in jobs {
        saved.push(j.write(store.as_ref()).map_err(EngineError::Other)?);
        j.finish(s);
    }
    forget_clean(s);
    heartbeat(s);
    Ok(json!({ "saved": saved, "skipped": skipped, "location": store.location() }))
}

/// Who an area of the store belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// This session.
    Mine,
    /// Another app that is running.
    Running,
    /// An app that is gone (crashed): its copies are offered.
    Gone,
}

/// Whose `area` of `store` is. A gone app's area is held by the caller (its lock in `hold` until
/// dropped) and, with `take` in a store without locks, taken over with a fresh heartbeat, so
/// another app leaves it alone meanwhile.
fn owner(s: &mut Session, store: &dyn RecoveryStore, area: &str, hold: &mut Vec<Hold>, take: bool) -> Owner {
    if s.recovery.own.as_ref().is_some_and(|o| o.area == area && o.store.location() == store.location()) {
        return Owner::Mine;
    }
    match store.lock(area) {
        Ok(Lock::Held(h)) => {
            hold.push(h);
            Owner::Gone
        }
        Ok(Lock::Busy) | Err(_) => Owner::Running,
        Ok(Lock::Unsupported) => {
            if !s.recovery.adopted.contains(area) && beating(store, area, stale_after(s)) {
                return Owner::Running;
            }
            if take {
                // Best effort: without it another app might offer the copies too, which loses
                // nothing.
                let _ = beat(store, area);
                s.recovery.adopted.insert(area.to_string());
            }
            Owner::Gone
        }
    }
}

/// Tidy away a gone app's area `area` once no copies are left in it, then let it go (`hold`).
fn tidy(store: &dyn RecoveryStore, area: &str, hold: Vec<Hold>) {
    let names = store.list().unwrap_or_default();
    let mine: Vec<&String> = names.iter().filter(|n| n.split_once('/').is_some_and(|(a, _)| a == area)).collect();
    if mine.iter().any(|n| n.ends_with(DOC_EXT)) {
        return;
    }
    for n in mine {
        let _ = store.remove(n);
    }
    drop(hold);
    store.remove_area(area);
}

/// The copies of documents open now in this session.
fn open_copies(s: &Session) -> Vec<String> {
    s.documents().iter().filter_map(|d| d.recovery.as_ref()).map(|c| c.file.clone()).collect()
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let store = need_store(s, "file.recovery.list")?;
    let open = open_copies(s);
    let copies = copies_in(&store.list().map_err(EngineError::Other)?);
    let mut rows = vec![];
    for area in store.areas().map_err(EngineError::Other)? {
        let mut hold = vec![];
        let who = owner(s, store.as_ref(), &area, &mut hold, false);
        let files: Vec<&String> = copies.iter().filter(|(a, _)| *a == area).map(|(_, f)| f).collect();
        for f in &files {
            let meta = read_meta(store.as_ref(), f);
            let title = if meta.title.is_empty() { f.rsplit('/').next().unwrap_or(f).to_string() } else { meta.title };
            rows.push(json!({
                "file": f, "title": title, "path": meta.path, "format": meta.format, "saved": meta.saved,
                "open": who == Owner::Mine && open.contains(f), "running": who == Owner::Running,
            }));
        }
        // A gone app's empty area is tidied away.
        if who == Owner::Gone && files.is_empty() {
            tidy(store.as_ref(), &area, hold);
        }
    }
    Ok(json!({ "copies": rows, "location": store.location() }))
}

/// A gone app's area to act on: its name, the copies to act on, and its lock.
type Target = (String, Vec<String>, Vec<Hold>);

/// The copies a command acts on, by gone app's area, each area held until its hold is dropped:
/// `file` (a copy a crash left behind), else every one.
fn targets(s: &mut Session, store: &dyn RecoveryStore, cmd: &str, p: &Value) -> Result<Vec<Target>> {
    let copies = copies_in(&store.list().map_err(EngineError::Other)?);
    let file = str_param(p, "file");
    if let Some(f) = file
        && !copies.iter().any(|(_, c)| c == f)
    {
        return Err(bad(cmd, format!("no recovery copy `{f}` (see file.recovery.list)")));
    }
    let picked = |c: &str| file.is_none_or(|f| f == c);
    let areas: BTreeSet<&str> = copies.iter().filter(|(_, c)| picked(c)).map(|(a, _)| a.as_str()).collect();
    let mut out = vec![];
    for area in areas {
        let mut hold = vec![];
        match (owner(s, store, area, &mut hold, true), file) {
            (Owner::Gone, _) => {
                let files = copies.iter().filter(|(a, c)| a == area && picked(c)).map(|(_, c)| c.clone()).collect();
                out.push((area.to_string(), files, hold));
            }
            (Owner::Mine, Some(f)) => return Err(bad(cmd, format!("`{f}` is the copy of a document open now"))),
            (Owner::Running, Some(f)) => return Err(bad(cmd, format!("`{f}` belongs to a Vector W3K2 that is running"))),
            _ => {}
        }
    }
    Ok(out)
}

/// Open copy `file` as a new document: "<name> [Recovered]", modified, with its original path
/// (for Save As) and format. The copy moves into this session's area (another crash keeps it).
fn restore_one(s: &mut Session, store: &Arc<dyn RecoveryStore>, file: &str) -> Result<Value> {
    let bytes = store.read(&format!("{file}{DOC_EXT}")).map_err(EngineError::Other)?;
    let meta_bytes = store.read(&format!("{file}{META_EXT}")).unwrap_or_default();
    let meta: Meta = serde_json::from_slice(&meta_bytes).unwrap_or_default();
    let (mut doc, _, warnings) = fileio::native_file(&bytes).map_err(|e| EngineError::Other(format!("recovery copy `{file}`: {e}")))?;
    let area = claim(s, store)?;
    let mut taken: Vec<String> = copies_in(&store.list().map_err(EngineError::Other)?).into_iter().map(|(_, f)| f).collect();
    taken.extend(open_copies(s));
    let stem = file.rsplit('/').next().unwrap_or(file);
    let mine = match format!("{area}/{stem}") {
        f if taken.contains(&f) => fresh_name(&area, stem, &taken),
        f => f,
    };
    store.write(&format!("{mine}{DOC_EXT}"), &bytes).map_err(EngineError::Other)?;
    store.write(&format!("{mine}{META_EXT}"), &meta_bytes).map_err(EngineError::Other)?;
    store.remove(&format!("{file}{DOC_EXT}")).map_err(EngineError::Other)?;
    store.remove(&format!("{file}{META_EXT}")).map_err(EngineError::Other)?;
    if doc.title.is_empty() {
        doc.title = if meta.title.is_empty() { stem.to_string() } else { meta.title.clone() };
    }
    // Linked files are looked for from the document's own folder.
    let links = super::links::resolve(&mut doc, meta.path.as_deref(), s.prefs.update_links == "automatically");
    let index = s.add_document(doc, meta.path.clone());
    let st = s.doc_mut()?;
    st.recovered = true;
    st.format = SAVE_FORMATS.iter().copied().find(|f| *f == meta.format).unwrap_or("vectorcraft");
    st.mark_unsaved();
    // The copy stays until the document is saved or closed.
    st.recovery = Some(RecoveryCopy { file: mine.clone(), doc: st.doc.clone() });
    let title = st.title();
    Ok(super::fileio::merge(json!({ "index": index, "title": title, "file": mine, "path": meta.path, "warnings": warnings }), links.to_json()))
}

fn restore(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.recovery.restore";
    let store = need_store(s, C)?;
    let one = str_param(p, "file").is_some();
    let (mut restored, mut failed) = (vec![], vec![]);
    for (area, files, hold) in targets(s, store.as_ref(), C, p)? {
        for f in files {
            match restore_one(s, &store, &f) {
                Ok(r) => restored.push(r),
                Err(e) if one => return Err(e),
                // A damaged copy doesn't hold the others back.
                Err(e) => failed.push(json!({ "file": f, "error": e.to_string() })),
            }
        }
        tidy(store.as_ref(), &area, hold);
    }
    Ok(json!({ "restored": restored, "failed": failed }))
}

fn discard(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.recovery.discard";
    let store = need_store(s, C)?;
    let mut discarded = vec![];
    for (area, files, hold) in targets(s, store.as_ref(), C, p)? {
        for f in files {
            store.remove(&format!("{f}{DOC_EXT}")).map_err(EngineError::Other)?;
            store.remove(&format!("{f}{META_EXT}")).map_err(EngineError::Other)?;
            discarded.push(f);
        }
        tidy(store.as_ref(), &area, hold);
    }
    Ok(json!({ "discarded": discarded }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.recovery.save",
            "Save Recovery Data",
            [],
            None,
            "{} write a recovery copy of every modified document changed since its last copy, in this app's own area of the store (the app does this every autosaveInterval minutes while autosaveRecovery is on; documents of more than 20000 objects are skipped while recoveryOffForComplex is on). A copy goes when its document is saved, reverted or closed → {saved: [{file, title, bytes}], skipped: [{title, reason}], location} (location: the recovery folder, or browser storage on the web)",
            has_store,
            save
        ),
        cmd!(
            query "file.recovery.list",
            "Recovery Data",
            [],
            None,
            "{} the recovery copies in the store → {copies: [{file (\"<area>/<name>\"), title, path (the document's file, if it had one), format, saved (Unix seconds or null), open (the copy of a document open here), running (kept by another Vector W3K2 that is running)}], location}; copies neither open nor running were left behind by a crash",
            has_store,
            list
        ),
        cmd!(
            "file.recovery.restore",
            "Restore Recovered Documents",
            [],
            None,
            "{file?} open a copy left behind by a crash (default: every one; a running Vector W3K2's copies are never taken) as a new document titled \"<name> [Recovered]\": modified, and Save asks where to save it (suggesting its original file) → {restored: [{index, title, file, path, warnings, missingLinks, modifiedLinks, updatedLinks}], failed: [{file, error}] (damaged copies, when restoring every one; a named one fails the command)}. The copy moves to this app's area and stays until the document is saved or closed",
            has_store,
            restore
        ),
        cmd!(
            "file.recovery.discard",
            "Discard Recovered Documents",
            [],
            None,
            "{file?} delete a copy left behind by a crash (default: every one; copies of open documents and of running Vector W3K2 apps stay) → {discarded: [file…]}",
            has_store,
            discard
        ),
    ]
}
