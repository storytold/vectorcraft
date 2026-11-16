//! Safe Mode, disabled plug-ins and the launch marker.
//!
//! **Safe Mode**: while it is on, no plug-in code runs in this process. [`crate::Plugin::load`] and
//! every run of an installed plug-in fail with [`Error::SafeMode`], so a plug-in installed before
//! Safe Mode was turned on can't run either, and live effects draw their object's geometry
//! unchanged. The apps turn it on at launch (`--safe-mode`, or the choice after a launch that
//! stopped while a plug-in was loading) and off only when the user starts normally from that
//! choice.
//!
//! **Disabled plug-ins**: the items the user turned off. Loading skips them by file or by id
//! ([`skipped`]); nothing is uninstalled. [`set_disabled`] sets both the list this launch skips and
//! the list saved for the next one; [`set_enabled`] changes only the saved list, so turning an item
//! back on takes effect at the next launch.
//!
//! **Launch marker** (native builds): a small file in a folder the app picks, written before the
//! stored plug-ins load, updated before and after each module load and each live effect's first
//! evaluation (and each evaluation that starts while one of those is running) to name the oldest
//! item still loading (several threads can load at once), and removed
//! when the app quits ([`begin_launch`], [`loading`], [`loaded`], [`end_launch`]). A launch that
//! ends while an item is still loading (a panic the app caught while a plug-in loaded) keeps its
//! marker naming that item. A marker left behind by a
//! launch that is no longer running names the item that was loading when it stopped, which may or
//! may not be what stopped it. Until the user
//! chooses what to do about that item ([`forget_previous`]), the next launch's marker names it too
//! and stays when that launch quits, so a launch that asks nothing (`--safe-mode`) or one closed in
//! Safe Mode leaves the choice to the launch after it. Each launch has
//! its own marker and holds a lock on a file beside it, so a launch that is still running (a second
//! window) is never taken for one that stopped. A marker that is missing, empty, unreadable,
//! malformed, larger than [`MAX_MARKER_BYTES`] or of another version names nothing.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Error, Result};

/// The longest id, file or name a stored item keeps, in characters.
const MAX_FIELD_CHARS: usize = 1024;
/// The most bytes one stored item takes in JSON: three fields of [`MAX_FIELD_CHARS`] characters,
/// each written in at most 6 bytes (`\u001f`), plus keys, quotes and indentation.
const MAX_ITEM_JSON_BYTES: usize = 3 * MAX_FIELD_CHARS * 6 + 256;
/// The largest launch marker read, in bytes: room for one stored item.
pub const MAX_MARKER_BYTES: u64 = (MAX_ITEM_JSON_BYTES + 256) as u64;
/// The longest name an item is shown with, in characters (longer ones end with "…").
pub const MAX_NAME_CHARS: usize = 64;
/// The most disabled items kept.
pub const MAX_DISABLED: usize = 1000;
/// The largest disabled-items file read, in bytes: room for [`MAX_DISABLED`] stored items.
pub const MAX_DISABLED_FILE_BYTES: u64 = (MAX_DISABLED * MAX_ITEM_JSON_BYTES + 256) as u64;
/// The `format` of a launch marker, and its version.
const MARKER_FORMAT: &str = "vectorcraft-launch";
const MARKER_VERSION: u64 = 1;

/// Why Safe Mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The `--safe-mode` option.
    Flag,
    /// The launch after one that stopped while a plug-in was loading, until the user chooses.
    Startup,
}

impl Source {
    /// Its name in `plugin.safeMode`.
    pub fn name(self) -> &'static str {
        match self {
            Source::Flag => "flag",
            Source::Startup => "startup",
        }
    }
}

/// 0: off; otherwise a [`Source`].
static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Turns Safe Mode on for `source`, or off with `None`.
pub fn set_active(source: Option<Source>) {
    ACTIVE.store(
        match source {
            None => 0,
            Some(Source::Flag) => 1,
            Some(Source::Startup) => 2,
        },
        Ordering::SeqCst,
    );
}

/// Why Safe Mode is on; `None` while it is off.
pub fn source() -> Option<Source> {
    match ACTIVE.load(Ordering::SeqCst) {
        1 => Some(Source::Flag),
        2 => Some(Source::Startup),
        _ => None,
    }
}

/// Is Safe Mode on?
pub fn active() -> bool {
    source().is_some()
}

/// [`Error::SafeMode`] while Safe Mode is on.
pub fn check() -> Result<()> {
    if active() { Err(Error::SafeMode) } else { Ok(()) }
}

/// A plug-in as the launch marker and the disabled list name it: its id when known, the file it
/// was loaded from, and its name. Two items are the same plug-in when their ids match, or, when
/// either has no id, their files match. An id or file longer than [`MAX_FIELD_CHARS`] characters
/// is kept, and compared, as its start followed by a hash of the whole ([`bound`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Item {
    /// The item of a module loaded from `file`, before its id is known.
    pub fn file(file: impl Into<String>) -> Self {
        Item { file: Some(file.into()), ..Item::default() }
    }

    /// The item of an installed plug-in.
    pub fn of(plugin: &crate::Plugin) -> Self {
        Item { id: Some(plugin.id().to_string()), file: plugin.source().map(str::to_string), name: Some(plugin.manifest().name.clone()) }
    }

    /// Is `other` the same plug-in?
    pub fn same(&self, other: &Item) -> bool {
        match (&self.id, &other.id) {
            (Some(a), Some(b)) => bound(a) == bound(b),
            _ => match (&self.file, &other.file) {
                (Some(a), Some(b)) => bound(a) == bound(b),
                _ => false,
            },
        }
    }

    /// The name it is shown with: its name, else its id, else its file's name, through
    /// [`display_name`].
    pub fn label(&self) -> String {
        let file_name = || self.file.as_deref().map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f));
        display_name(self.name.as_deref().filter(|n| !n.trim().is_empty()).or(self.id.as_deref()).or_else(file_name).unwrap_or(""))
    }

    /// Its fields, each at most [`MAX_FIELD_CHARS`] characters (the id and the file through
    /// [`bound`], the name cut); `None` when it has neither an id nor a file.
    fn bounded(self) -> Option<Item> {
        let keep = |s: Option<String>| s.filter(|s| !s.is_empty()).map(|s| bound(&s).into_owned());
        let name = self.name.filter(|s| !s.is_empty()).map(|s| s.chars().take(MAX_FIELD_CHARS).collect::<String>());
        let item = Item { id: keep(self.id), file: keep(self.file), name };
        (item.id.is_some() || item.file.is_some()).then_some(item)
    }

    /// As `plugin.safeMode` lists it: `{id, file, name, label}`.
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "file": self.file, "name": self.name, "label": self.label()})
    }
}

/// `s` when it has at most [`MAX_FIELD_CHARS`] characters; else its start, `#` and a hash of the
/// whole of it, [`MAX_FIELD_CHARS`] characters in all, so two long values that share a start stay
/// apart. A value made this way is its own bound.
fn bound(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars().nth(MAX_FIELD_CHARS).is_none() {
        return std::borrow::Cow::Borrowed(s);
    }
    // FNV-1a: stable across versions and platforms.
    let hash = s.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let start: String = s.chars().take(MAX_FIELD_CHARS - 17).collect();
    std::borrow::Cow::Owned(format!("{start}#{hash:016x}"))
}

/// `s` safe to show in a sentence: control characters and the characters that reorder text
/// (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069) removed, at most
/// [`MAX_NAME_CHARS`] characters (a longer name ends with "…").
pub fn display_name(s: &str) -> String {
    let reorders = |c: char| matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let clean: Vec<char> = s.trim().chars().filter(|&c| !c.is_control() && !reorders(c)).collect();
    if clean.len() > MAX_NAME_CHARS {
        clean[..MAX_NAME_CHARS - 1].iter().chain(std::iter::once(&'…')).collect()
    } else {
        clean.into_iter().collect()
    }
}

/// The disabled items this launch skips, the ones saved for the next launch, and the changes made
/// to the saved list since this launch last read or wrote the file the apps keep it in.
struct Disabled {
    skipped: Vec<Item>,
    saved: Vec<Item>,
    changes: Vec<Change>,
    /// Counts the [`set_disabled`] calls.
    read: u64,
}

/// One change to the saved list, made again to the file's list when it is saved ([`merge_saved`]).
#[derive(Clone)]
enum Change {
    /// [`set_enabled`] (or [`disable`], turning it off).
    Enabled(Item, bool),
    /// [`disable_file_of`].
    FileOf(Item),
}

impl Change {
    /// Makes the change to `list` → whether it changed.
    fn apply(&self, list: &mut Vec<Item>) -> bool {
        match self {
            Change::Enabled(item, enabled) => enable_in(list, item, *enabled),
            Change::FileOf(plugin) => file_of_in(list, plugin),
        }
    }
}

static DISABLED: RwLock<Disabled> = RwLock::new(Disabled { skipped: Vec::new(), saved: Vec::new(), changes: Vec::new(), read: 0 });

/// Sets the disabled items, for this launch and the next, as read from the file the apps keep them
/// in (at most [`MAX_DISABLED`]; one without an id or a file is dropped).
pub fn set_disabled(items: Vec<Item>) {
    let items: Vec<Item> = items.into_iter().filter_map(Item::bounded).take(MAX_DISABLED).collect();
    let mut d = DISABLED.write().unwrap_or_else(|e| e.into_inner());
    d.skipped = items.clone();
    d.saved = items;
    d.changes.clear();
    d.read += 1;
}

/// What [`merge_saved`] made: the content to write to the file, and which changes it holds.
#[derive(Debug)]
pub struct Merged {
    /// The file's new content.
    pub bytes: Vec<u8>,
    /// The changes it holds: the first `made` of them.
    made: usize,
    /// The [`set_disabled`] they follow.
    read: u64,
    /// Of those, the ones turning a plug-in off that a full list (another window's) had no room
    /// for: they stay to be made at the next save.
    full: Vec<usize>,
}

/// Before the saved list is written: takes in `theirs`, the list the file holds now (another
/// window sharing the preferences may have saved it since this launch read it; `None` when it
/// can't be read, and the saved list stays as it is). The changes this launch made to the saved
/// list since it last read or wrote the file ([`saved_written`]) are made again, in order, to
/// `theirs`, and the result becomes the saved list. This launch's skipped list stays as it is.
pub fn merge_saved(theirs: Option<Vec<Item>>) -> Merged {
    let mut d = DISABLED.write().unwrap_or_else(|e| e.into_inner());
    let made = d.changes.len();
    let mut full = Vec::new();
    if let Some(theirs) = theirs {
        let mut merged: Vec<Item> = theirs.into_iter().filter_map(Item::bounded).take(MAX_DISABLED).collect();
        for (i, change) in d.changes.iter().enumerate() {
            if !change.apply(&mut merged)
                && let Change::Enabled(item, false) = change
                && !merged.iter().any(|m| m.same(item))
            {
                full.push(i);
            }
        }
        d.saved = merged;
    }
    Merged { bytes: file_bytes(&d.saved), made, read: d.read, full }
}

impl Merged {
    /// Does the content hold every change? A plug-in turned off that a full list had no room for
    /// is not in it (it waits for the next save).
    pub fn complete(&self) -> bool {
        self.full.is_empty()
    }
}

/// `merged`'s content went to the file: later saves make again only the changes made after
/// [`merge_saved`], and the ones it had no room for ([`merge_saved`]).
pub fn saved_written(merged: &Merged) {
    let mut d = DISABLED.write().unwrap_or_else(|e| e.into_inner());
    if d.read != merged.read {
        return;
    }
    let made = merged.made.min(d.changes.len());
    let kept: Vec<Change> = merged.full.iter().filter_map(|&i| d.changes.get(i).filter(|_| i < made).cloned()).collect();
    d.changes.drain(..made);
    d.changes.splice(0..0, kept);
}

/// Adds `item` to the disabled items, for this launch and the next → whether the list saved for
/// the next launch has it (`false` when that list is full). This launch skips it either way.
pub fn disable(item: Item) -> bool {
    let Some(item) = item.bounded() else { return false };
    let mut d = DISABLED.write().unwrap_or_else(|e| e.into_inner());
    let d = &mut *d;
    if !d.skipped.iter().any(|s| s.same(&item)) {
        d.skipped.push(item.clone());
    }
    if enable_in(&mut d.saved, &item, false) {
        d.changes.push(Change::Enabled(item, false));
        return true;
    }
    d.saved.iter().any(|s| s.same(&item))
}

/// Turns `item` on or off for the next launch, leaving this launch as it is → whether the saved
/// list changed.
pub fn set_enabled(item: &Item, enabled: bool) -> bool {
    let mut d = DISABLED.write().unwrap_or_else(|e| e.into_inner());
    let changed = enable_in(&mut d.saved, item, enabled);
    if changed {
        d.changes.push(Change::Enabled(item.clone(), enabled));
    }
    changed
}

/// Turns `item` on (removes every entry for it) or off (adds it unless an entry has it, within
/// [`MAX_DISABLED`]) in `list` → whether it changed.
fn enable_in(list: &mut Vec<Item>, item: &Item, enabled: bool) -> bool {
    let before = list.len();
    if enabled {
        list.retain(|i| !i.same(item));
        list.len() != before
    } else if let Some(item) = item.clone().bounded().filter(|i| !list.iter().any(|s| s.same(i)) && before < MAX_DISABLED) {
        list.push(item);
        true
    } else {
        false
    }
}

/// `plugin`, loaded from its file and then skipped by its id: each list that disables it gets its
/// file too (the entry naming its id without a file takes it, or a new entry is added), so a later
/// load skips that file before running any of its code → whether the saved list changed.
pub fn disable_file_of(plugin: &Item) -> bool {
    let mut d = DISABLED.write().unwrap_or_else(|e| e.into_inner());
    let d = &mut *d;
    file_of_in(&mut d.skipped, plugin);
    let changed = file_of_in(&mut d.saved, plugin);
    if changed {
        d.changes.push(Change::FileOf(plugin.clone()));
    }
    changed
}

/// [`disable_file_of`] for one list → whether it changed.
fn file_of_in(list: &mut Vec<Item>, plugin: &Item) -> bool {
    let Some(file) = plugin.file.as_deref().filter(|f| !f.is_empty()) else { return false };
    let by_file = Item::file(file);
    if list.iter().any(|i| i.same(&by_file)) || !list.iter().any(|i| i.same(plugin)) {
        return false;
    }
    if let Some(entry) = list.iter_mut().find(|i| i.same(plugin) && i.file.is_none()) {
        entry.file = Some(bound(file).into_owned());
    } else if let Some(item) = plugin.clone().bounded().filter(|_| list.len() < MAX_DISABLED) {
        list.push(item);
    } else {
        return false;
    }
    true
}

/// The disabled items saved for the next launch.
pub fn disabled() -> Vec<Item> {
    DISABLED.read().unwrap_or_else(|e| e.into_inner()).saved.clone()
}

/// Does this launch skip `item` (a module file before it loads, or a plug-in once its id is known)?
pub fn skipped(item: &Item) -> bool {
    DISABLED.read().unwrap_or_else(|e| e.into_inner()).skipped.iter().any(|i| i.same(item))
}

/// The ids of the disabled items this launch skips.
pub fn skipped_ids() -> Vec<String> {
    DISABLED.read().unwrap_or_else(|e| e.into_inner()).skipped.iter().filter_map(|i| i.id.clone()).collect()
}

/// The disabled items saved for the next launch, as the file the apps keep them in.
pub fn disabled_file() -> Vec<u8> {
    file_bytes(&disabled())
}

/// `items` as the file the apps keep the disabled items in.
fn file_bytes(items: &[Item]) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({"format": "vectorcraft-disabled-plugins", "version": 1, "items": items})).unwrap_or_default()
}

/// The items of a file [`disabled_file`] wrote; none when it isn't one.
pub fn parse_disabled_file(bytes: &[u8]) -> Vec<Item> {
    read_disabled_file(bytes).unwrap_or_default()
}

/// The items of a file [`disabled_file`] wrote, `None` when the file is JSON of another format or
/// version (a later version's list, which a save must not replace). Bytes that aren't JSON hold no
/// items any launch can read: none.
pub fn read_disabled_file(bytes: &[u8]) -> Option<Vec<Item>> {
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else { return Some(vec![]) };
    if v["format"] != "vectorcraft-disabled-plugins" || v["version"] != 1 {
        return None;
    }
    Some(
        v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| serde_json::from_value::<Item>(i.clone()).ok()?.bounded())
            .take(MAX_DISABLED)
            .collect(),
    )
}

/// The item the launch before this one was loading when it stopped, once [`begin_launch`] read it.
static PREVIOUS: Mutex<Option<Item>> = Mutex::new(None);

/// The item the launch before this one was loading when it stopped ([`begin_launch`]); `None` once
/// the user chose what to do about it ([`forget_previous`]).
pub fn previous() -> Option<Item> {
    PREVIOUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The user chose what to do about [`previous`]: this launch's marker stops naming it.
pub fn forget_previous() {
    *PREVIOUS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    refresh();
}

/// The items still loading, oldest first: one entry for each [`loading`] call, until the [`loaded`]
/// given its [`Loading`].
struct Running {
    next: u64,
    items: Vec<(u64, Item)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Running {
    /// The item this launch's marker names: the oldest item still loading, else [`previous`].
    fn pending(&self) -> Option<Item> {
        self.items.first().map(|(_, i)| i.clone()).or_else(previous)
    }
}

static LOADING: Mutex<Running> = Mutex::new(Running { next: 0, items: Vec::new() });

/// One item [`loading`] named, until it is given to [`loaded`].
#[must_use]
#[derive(Debug)]
pub struct Loading(u64);

/// Writes this launch's marker naming the item [`Running::pending`] gives, while holding the lock
/// on the items still loading, so two threads' writes land in the order of their changes.
fn refresh() {
    #[cfg(not(target_arch = "wasm32"))]
    marker::write(LOADING.lock().unwrap_or_else(|e| e.into_inner()).pending().as_ref());
}

/// The marker's content naming `item`, cut as a stored item is (none: nothing is loading).
#[cfg(not(target_arch = "wasm32"))]
fn marker_bytes(item: Option<&Item>) -> Vec<u8> {
    let item = item.cloned().and_then(Item::bounded);
    serde_json::to_vec(&json!({"format": MARKER_FORMAT, "version": MARKER_VERSION, "item": item})).unwrap_or_default()
}

/// The item a marker's content names; `None` when it names none or isn't a marker of this version.
pub fn parse_marker(bytes: &[u8]) -> Option<Item> {
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return None;
    }
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v["format"] != MARKER_FORMAT || v["version"] != MARKER_VERSION {
        return None;
    }
    serde_json::from_value::<Item>(v.get("item").filter(|i| i.is_object())?.clone()).ok()?.bounded()
}

#[cfg(not(target_arch = "wasm32"))]
pub use marker::{begin_launch, end_launch};

/// Adds `item` to the items still loading and writes this launch's marker naming the oldest of
/// them (no marker: only the list changes) → the [`Loading`] to give to [`loaded`] when it is done.
pub fn loading(item: &Item) -> Loading {
    let mut running = LOADING.lock().unwrap_or_else(|e| e.into_inner());
    let token = running.next;
    running.next = running.next.wrapping_add(1);
    running.items.push((token, item.clone()));
    #[cfg(not(target_arch = "wasm32"))]
    marker::write(running.pending().as_ref());
    Loading(token)
}

/// Removes the item `token` names from the items still loading and writes this launch's marker
/// naming the oldest one left; with none left, the item of [`previous`] or nothing.
pub fn loaded(token: Loading) {
    let mut running = LOADING.lock().unwrap_or_else(|e| e.into_inner());
    running.items.retain(|(t, _)| *t != token.0);
    #[cfg(not(target_arch = "wasm32"))]
    marker::write(running.pending().as_ref());
}

#[cfg(not(target_arch = "wasm32"))]
mod marker {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::{Item, LOADING, MAX_MARKER_BYTES, PREVIOUS, marker_bytes, parse_marker};

    /// The most marker files looked at in the folder.
    const MAX_MARKERS: usize = 100;
    const PREFIX: &str = "launch-";

    /// This launch's marker: its folder, its name and the open lock file.
    struct Current {
        dir: PathBuf,
        token: String,
        _lock: File,
    }

    static CURRENT: Mutex<Option<Current>> = Mutex::new(None);

    fn json_path(dir: &Path, token: &str) -> PathBuf {
        dir.join(format!("{PREFIX}{token}.json"))
    }
    fn lock_path(dir: &Path, token: &str) -> PathBuf {
        dir.join(format!("{PREFIX}{token}.lock"))
    }
    fn tmp_path(dir: &Path, token: &str) -> PathBuf {
        dir.join(format!("{PREFIX}{token}.json.tmp"))
    }

    /// Is the launch `token` still running (does it hold its lock)?
    fn running(dir: &Path, token: &str) -> bool {
        let Ok(f) = File::options().read(true).write(true).open(lock_path(dir, token)) else { return false };
        match f.try_lock() {
            Ok(()) => {
                let _ = f.unlock();
                false
            }
            Err(_) => true,
        }
    }

    /// The content of `path` up to [`MAX_MARKER_BYTES`]; `None` when it can't be read or is larger.
    fn read(path: &Path) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(path).ok()?.take(MAX_MARKER_BYTES + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_MARKER_BYTES).then_some(bytes)
    }

    /// Starts this launch's marker in `dir` → the item a launch that is no longer running was
    /// loading when it stopped, if one was (its marker is then removed, and this launch's marker
    /// names the item until [`super::forget_previous`]). A folder that can't be written leaves this
    /// launch without a marker; it still starts.
    pub fn begin_launch(dir: &Path) -> Option<Item> {
        let mut previous = None;
        let mut tokens: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter_map(|n| Some(n.strip_prefix(PREFIX)?.strip_suffix(".json")?.to_string()))
            .collect();
        tokens.sort();
        tokens.truncate(MAX_MARKERS);
        for token in tokens {
            if running(dir, &token) {
                continue;
            }
            if previous.is_none() {
                previous = read(&json_path(dir, &token)).as_deref().and_then(parse_marker);
            }
            for path in [json_path(dir, &token), tmp_path(dir, &token), lock_path(dir, &token)] {
                let _ = std::fs::remove_file(path);
            }
        }
        *PREVIOUS.lock().unwrap_or_else(|e| e.into_inner()) = previous.clone();
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let token = format!("{}-{nanos:x}", std::process::id());
        let lock = std::fs::create_dir_all(dir)
            .and_then(|()| File::options().create(true).truncate(false).read(true).write(true).open(lock_path(dir, &token)));
        if let Ok(lock) = lock.and_then(|f| f.try_lock().map(|()| f).map_err(std::io::Error::other)) {
            *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Current { dir: dir.to_path_buf(), token, _lock: lock });
            super::refresh();
        }
        previous
    }

    /// Replaces this launch's marker with one naming `item`: written to a temporary file first,
    /// then renamed over the marker, so a reader finds the old marker or the new one. A write that
    /// fails leaves the marker as it was.
    pub(super) fn write(item: Option<&Item>) {
        let current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = current.as_ref() else { return };
        let tmp = tmp_path(&c.dir, &c.token);
        let _ = File::create(&tmp)
            .and_then(|mut f| f.write_all(&marker_bytes(item)).and_then(|()| f.sync_all()))
            .and_then(|()| std::fs::rename(&tmp, json_path(&c.dir, &c.token)));
    }

    /// Removes this launch's marker: the app is quitting normally. While an item is still loading
    /// ([`super::loading`] without [`super::loaded`]), the marker stays, naming the oldest one;
    /// otherwise, while the user has not chosen what to do about [`super::previous`], the marker
    /// stays, naming that, so the next launch asks.
    pub fn end_launch() {
        let running = LOADING.lock().unwrap_or_else(|e| e.into_inner());
        let pending = running.pending();
        if pending.is_some() {
            write(pending.as_ref());
        }
        let Some(c) = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        let Current { dir, token, _lock } = c;
        drop(_lock);
        let marker = pending.is_none().then(|| json_path(&dir, &token));
        for path in marker.into_iter().chain([tmp_path(&dir, &token), lock_path(&dir, &token)]) {
            let _ = std::fs::remove_file(path);
        }
    }

    #[cfg(test)]
    pub(super) fn path_of_current() -> Option<PathBuf> {
        CURRENT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|c| json_path(&c.dir, &c.token))
    }
}

/// Serializes the tests that change this module's process-wide state.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Safe Mode off, no launch marker, nothing loading, pending or disabled.
    fn reset() {
        LOADING.lock().unwrap_or_else(|e| e.into_inner()).items.clear();
        end_launch();
        set_active(None);
        forget_previous();
        set_disabled(vec![]);
    }

    /// Holds [`TEST_LOCK`] for one test and resets the module's state when the test starts and when
    /// it ends, so a test that fails leaves the next one as it would find a new process.
    struct Fresh {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    fn fresh() -> Fresh {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        Fresh { _guard: guard }
    }

    impl Drop for Fresh {
        fn drop(&mut self) {
            reset();
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("vc-safe-mode-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_are_bounded_and_cannot_reorder_or_hide_text() {
        assert_eq!(display_name("Good\u{202E}gnp.exe"), "Goodgnp.exe");
        assert_eq!(display_name("a\u{0007}b\nc\u{2067}d\u{200F}"), "abcd");
        let long = "x".repeat(200);
        let shown = display_name(&long);
        assert_eq!(shown.chars().count(), MAX_NAME_CHARS);
        assert!(shown.ends_with('…'));
        // The label shows the name, but the id still picks the plug-in.
        let item = Item { id: Some("org.example.a".into()), file: None, name: Some("\u{202E}evil".into()) };
        assert_eq!(item.label(), "evil");
        assert!(item.same(&Item { id: Some("org.example.a".into()), file: None, name: Some("other".into()) }));
        assert!(!item.same(&Item { id: Some("org.example.b".into()), file: None, name: Some("\u{202E}evil".into()) }));
        assert_eq!(Item::file("C:\\Plug-ins\\x.wasm").label(), "x.wasm");
    }

    #[test]
    fn only_a_marker_of_this_version_names_an_item() {
        let item = Item { id: Some("org.example.x".into()), file: Some("/p/x.wasm".into()), name: None };
        assert_eq!(parse_marker(&marker_bytes(Some(&item))), Some(item));
        assert_eq!(parse_marker(&marker_bytes(None)), None, "nothing was loading");
        for bad in [
            &b""[..],
            b"{",
            b"[]",
            br#"{"format":"vectorcraft-launch","version":2,"item":{"id":"x"}}"#,
            br#"{"format":"other","version":1,"item":{"id":"x"}}"#,
            br#"{"format":"vectorcraft-launch","version":1,"item":"x"}"#,
            br#"{"format":"vectorcraft-launch","version":1,"item":{}}"#,
            br#"{"format":"vectorcraft-launch","version":1,"item":{"id":"x""#,
        ] {
            assert_eq!(parse_marker(bad), None, "{}", String::from_utf8_lossy(bad));
        }
        let mut big =
            format!(r#"{{"format":"vectorcraft-launch","version":1,"item":{{"id":"{}"}}}}"#, "y".repeat(MAX_MARKER_BYTES as usize)).into_bytes();
        assert_eq!(parse_marker(&big), None, "larger than the limit");
        big.truncate(10);
        assert_eq!(parse_marker(&big), None, "cut short");
    }

    #[test]
    fn a_launch_that_stopped_names_its_item_and_a_running_one_does_not() {
        let _f = fresh();
        let dir = temp_dir("marker");
        // A launch that stopped while loading x: its marker without a held lock, and a stray
        // temporary file from a write it didn't finish.
        let x = Item { id: Some("org.example.x".into()), file: None, name: Some("X".into()) };
        std::fs::write(dir.join("launch-1-a.json"), marker_bytes(Some(&x))).unwrap();
        std::fs::write(dir.join("launch-1-a.json.tmp"), b"{\"format\":\"vectorcr").unwrap();
        assert_eq!(begin_launch(&dir), Some(x.clone()));
        assert_eq!(previous(), Some(x.clone()));
        assert!(!dir.join("launch-1-a.json").exists() && !dir.join("launch-1-a.json.tmp").exists(), "read once");
        let mine = marker::path_of_current().unwrap();
        assert_eq!(std::fs::read(&mine).ok().as_deref().and_then(parse_marker), Some(x.clone()), "named until the user chooses");
        forget_previous();
        assert_eq!(std::fs::read(&mine).ok().as_deref().and_then(parse_marker), None);
        let token = loading(&x);
        assert_eq!(std::fs::read(&mine).ok().as_deref().and_then(parse_marker), Some(x.clone()));
        assert!(!mine.with_extension("json.tmp").exists(), "the temporary file is renamed over the marker");
        loaded(token);
        assert_eq!(std::fs::read(&mine).ok().as_deref().and_then(parse_marker), None);
        end_launch();
        assert!(!mine.exists(), "a normal quit removes the marker");
        assert_eq!(begin_launch(&dir), None, "nothing left behind");
        end_launch();
        forget_previous();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_launch_that_quits_before_the_user_chooses_leaves_the_choice_to_the_next() {
        let _f = fresh();
        let dir = temp_dir("pending");
        let x = Item { id: Some("org.example.x".into()), file: None, name: Some("X".into()) };
        std::fs::write(dir.join("launch-1-a.json"), marker_bytes(Some(&x))).unwrap();
        assert_eq!(begin_launch(&dir), Some(x.clone()));
        end_launch();
        assert_eq!(begin_launch(&dir), Some(x.clone()), "asked again");
        forget_previous();
        end_launch();
        assert_eq!(begin_launch(&dir), None, "once the user chose");
        end_launch();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_launch_that_ends_while_a_plug_in_loads_names_it_at_the_next_launch() {
        let _f = fresh();
        let dir = temp_dir("ends-loading");
        let x = Item { id: Some("org.example.x".into()), file: None, name: Some("X".into()) };
        assert_eq!(begin_launch(&dir), None);
        let token = loading(&x);
        // A panic the app caught while x loaded: the app quits without `loaded`.
        end_launch();
        loaded(token);
        assert_eq!(begin_launch(&dir), Some(x.clone()), "the next launch asks");
        forget_previous();
        end_launch();
        assert_eq!(begin_launch(&dir), None);
        end_launch();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_marker_names_a_plug_in_still_loading_when_another_one_is_done() {
        let _f = fresh();
        let dir = temp_dir("two-loading");
        let a = Item { id: Some("org.example.a".into()), file: None, name: Some("A".into()) };
        let b = Item { id: Some("org.example.b".into()), file: None, name: Some("B".into()) };
        assert_eq!(begin_launch(&dir), None);
        let mine = marker::path_of_current().unwrap();
        let marker = || std::fs::read(&mine).ok().as_deref().and_then(parse_marker);
        // Two threads run their plug-ins' first evaluations at once.
        let ta = loading(&a);
        let tb = loading(&b);
        assert_eq!(marker(), Some(a.clone()), "the oldest item still loading");
        loaded(ta);
        assert_eq!(marker(), Some(b.clone()), "b is still loading");
        // The app stops while b loads.
        end_launch();
        loaded(tb);
        assert_eq!(begin_launch(&dir), Some(b.clone()), "the next launch offers b");
        forget_previous();
        end_launch();
        assert_eq!(begin_launch(&dir), None);
        end_launch();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_file_path_names_and_matches_its_plug_in() {
        let _f = fresh();
        let dir = temp_dir("long-path");
        // Each character written as 6 or 2 bytes, so the item is far larger in JSON than in characters.
        let long = format!("C:\\{}\\x.wasm", "\u{1}é".repeat(700));
        assert_eq!(begin_launch(&dir), None);
        let token = loading(&Item::file(long.clone()));
        end_launch();
        loaded(token);
        let stopped = begin_launch(&dir).expect("the marker names the module");
        assert!(stopped.same(&Item::file(long.clone())), "the stored item matches the full path");
        disable(stopped);
        assert!(skipped(&Item::file(long.clone())), "Disable and Continue skips it");
        assert!(!skipped(&Item::file(format!("D{}", &long[1..]))), "a path that differs within the kept part is another plug-in");
        let other = long.replace("x.wasm", "y.wasm");
        assert!(!skipped(&Item::file(other.clone())), "a path that differs after the kept part is another plug-in");
        assert!(!Item::file(long.clone()).same(&Item::file(other)));
        forget_previous();
        end_launch();
        set_disabled(vec![]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_list_of_long_items_fits_the_file_the_apps_read() {
        let _f = fresh();
        let field = |i: usize, tag: &str| format!("{tag}{i:04}{}", "\u{1}".repeat(MAX_FIELD_CHARS));
        set_disabled((0..MAX_DISABLED).map(|i| Item { id: Some(field(i, "i")), file: Some(field(i, "f")), name: Some(field(i, "n")) }).collect());
        let file = disabled_file();
        assert!(file.len() as u64 <= MAX_DISABLED_FILE_BYTES, "{} bytes", file.len());
        assert_eq!(parse_disabled_file(&file).len(), MAX_DISABLED);
        // A full list has no room for one more: this launch skips it, and the caller is told.
        let late = Item::file("late.wasm");
        assert!(!disable(late.clone()));
        assert!(skipped(&late) && !disabled().contains(&late));
        assert!(disable(Item { id: Some(field(0, "i")), ..Item::default() }), "one the list has");
        set_disabled(vec![]);
    }

    /// A plug-in turned on here is removed from the file's list even when another window saved its
    /// entry with more details. A change whose save failed, one made while the file was written and
    /// one that a full list had no room for are made again at the next save.
    #[test]
    fn merging_the_saved_list_matches_plug_ins_and_keeps_unsaved_changes() {
        let _f = fresh();
        let id = |s: &str| Item { id: Some(s.into()), ..Item::default() };
        set_disabled(vec![id("x")]);
        assert!(set_enabled(&id("x"), true));
        let theirs_x = Item { id: Some("x".into()), file: Some("/p/x.wasm".into()), name: Some("X".into()) };
        let merged = merge_saved(Some(vec![theirs_x, id("y")]));
        assert_eq!(disabled(), [id("y")], "x is turned on, whatever details the other window saved");
        assert_eq!(parse_disabled_file(&merged.bytes), [id("y")]);
        saved_written(&merged);

        assert!(set_enabled(&id("a"), false));
        // Another window has turned c off.
        let _ = merge_saved(Some(vec![id("y"), id("c")]));
        // The write failed: no saved_written. The other window turns c on again.
        assert!(set_enabled(&id("b"), false));
        let merged = merge_saved(Some(vec![id("y")]));
        assert_eq!(disabled(), [id("y"), id("a"), id("b")], "a is still saved, and c stays on");
        // A change made while the file is written is made again at the next save.
        assert!(set_enabled(&id("d"), false));
        saved_written(&merged);
        let _ = merge_saved(Some(vec![id("y"), id("a"), id("b")]));
        assert_eq!(disabled(), [id("y"), id("a"), id("b"), id("d")]);
        set_disabled(vec![]);

        // Another window filled the list: the plug-in turned off here waits for room.
        let full: Vec<Item> = (0..MAX_DISABLED).map(|i| id(&format!("full{i}"))).collect();
        assert!(set_enabled(&id("e"), false));
        let merged = merge_saved(Some(full.clone()));
        assert_eq!(disabled(), full);
        saved_written(&merged);
        let _ = merge_saved(Some(full[1..].to_vec()));
        assert_eq!(disabled().last(), Some(&id("e")), "made once there is room");

        // This launch adds x's file to its entry: the file's entry for x takes it, unless another
        // window turned x on meanwhile.
        let x_file = Item { id: Some("x".into()), file: Some("/p/x.wasm".into()), name: Some("X".into()) };
        let x_entry = Item { name: None, ..x_file.clone() };
        for (theirs, expected) in [(vec![id("x"), id("y")], vec![x_entry, id("y")]), (vec![id("y")], vec![id("y")])] {
            set_disabled(vec![id("x"), id("y")]);
            assert!(disable_file_of(&x_file));
            let _ = merge_saved(Some(theirs));
            assert_eq!(disabled(), expected);
        }
    }

    #[test]
    fn a_marker_whose_launch_holds_its_lock_is_left_alone() {
        let _f = fresh();
        let dir = temp_dir("running");
        let x = Item { id: Some("org.example.x".into()), file: None, name: None };
        std::fs::write(dir.join("launch-9-b.json"), marker_bytes(Some(&x))).unwrap();
        let lock = std::fs::File::create(dir.join("launch-9-b.lock")).unwrap();
        lock.try_lock().unwrap();
        assert_eq!(begin_launch(&dir), None, "that launch is still running");
        assert!(dir.join("launch-9-b.json").exists());
        end_launch();
        drop(lock);
        assert_eq!(begin_launch(&dir), Some(x), "once it stopped");
        end_launch();
        forget_previous();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enabling_takes_effect_at_the_next_launch() {
        let _f = fresh();
        let x = Item { id: Some("org.example.x".into()), file: Some("/p/x.wasm".into()), name: None };
        let y = Item::file("/p/y.wasm");
        set_disabled(vec![x.clone(), Item::default()]);
        assert_eq!(disabled(), std::slice::from_ref(&x), "an item that names nothing is dropped");
        assert!(skipped(&Item { id: Some("org.example.x".into()), ..Item::default() }));
        assert!(!skipped(&y));
        assert!(set_enabled(&x, true));
        assert!(disabled().is_empty());
        assert!(skipped(&x), "still skipped until the next launch");
        assert!(set_enabled(&y, false) && !set_enabled(&y, false));
        assert!(!skipped(&y), "turning one off waits for the next launch too");
        disable(y.clone());
        assert!(skipped(&y));
        assert_eq!(parse_disabled_file(&disabled_file()), disabled());
        assert!(parse_disabled_file(b"{\"format\":\"other\",\"version\":1,\"items\":[{\"id\":\"x\"}]}").is_empty());
        set_disabled(vec![]);
    }
}
