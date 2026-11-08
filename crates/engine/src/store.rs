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
//! * **Plug-ins**: the module of each plug-in `plugin.install` installs, as a `.wasm` file named
//!   after its id (as Illustrator keeps plug-ins in its Plug-ins folder); `plugin.remove` deletes it.
//!   Attaching the folder installs them again, except in Safe Mode and except the ones the user
//!   disabled, which `disabled.json` in the same folder lists (`plugin.setEnabled`; a window locks
//!   `disabled.lock` while it writes the list). The Additional Plug-ins Folder preference is not
//!   copied: it is loaded from where it is. `grants.json` holds what the user granted the plug-ins
//!   stored there that list permissions (id, module hash, permissions); a stored plug-in whose
//!   permissions it doesn't grant is skipped with a log record, and stays in the folder.
//! * **Plug-in settings and caches**: `storage/id-<id>/` in the plug-in store, the id written as
//!   [`case_safe`] writes it, holds what a plug-in
//!   keeps with the `settings` and `cache` permissions ([`PluginStorage`]): `settings.json` and a
//!   `cache` folder with one file per entry. The host names every file; a plug-in only names keys.
//!   Replacing a plug-in keeps them, and `plugin.remove` deletes them.
//!
//! The files are untrusted input like any other: sizes and counts are capped, a file that does not
//! parse is skipped with a log record, and every write is atomic.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use vectorcraft_pdf::{PdfPreset, builtin_preset};
use vectorcraft_plugins::safe_mode;
use vectorcraft_plugins::storage::{self, Meter, Storage, StorageError};

use crate::Session;
use crate::cmd::pdfcmds::{PRESET_FORMAT, PresetFile};

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
/// The file in the plug-in folder that a window locks while it reads and writes [`DISABLED_FILE`].
pub const DISABLED_LOCK: &str = "disabled.lock";
/// The file in the plug-in store that holds the permissions the user granted.
const GRANTS_FILE: &str = "grants.json";
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
/// each other's settings.
static PLUGIN_DATA_LOCK: Mutex<()> = Mutex::new(());
/// The closed flag of each plug-in folder's storage, shared by every storage made for the folder
/// until the plug-in is removed, which sets it and drops it.
static PLUGIN_DATA_OPEN: Mutex<BTreeMap<PathBuf, Arc<AtomicBool>>> = Mutex::new(BTreeMap::new());

/// The file name (without extension) a stored item called `name` gets: its name made safe for any
/// file system, then a hash of the name, so that names that differ only in characters a file name
/// can't hold, or in case, can't share a file. Names equal in any case are one item.
pub(crate) fn file_stem(name: &str) -> String {
    let safe: String = name.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).take(48).collect();
    // FNV-1a over the lower-cased name: stable across versions and platforms, unlike `DefaultHasher`.
    let hash = name.trim().to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{}-{hash:016x}", safe.trim())
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

/// The content of `path` when it is at most `max` bytes; the file is never read past `max`.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    file.take(max + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(bytes)
}

/// What a file over its cap is told (a detail after the file's name, in the log or an error).
fn too_large(max: u64) -> String {
    format!("larger than {max} bytes")
}

/// Write `bytes` to `dir/file`, making the folder; nothing is written when the file has them
/// already. A failure is logged: remembering is best effort and never fails the command.
fn put(dir: &Path, file: &str, bytes: &[u8]) -> bool {
    let path = dir.join(file);
    if std::fs::read(&path).is_ok_and(|old| old == bytes) {
        return true;
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

/// [`DISABLED_LOCK`] in `dir`, locked until the file is dropped, waiting while another window holds
/// it; `None` when it can't be opened or locked.
#[cfg(not(target_arch = "wasm32"))]
fn lock_disabled(dir: &Path) -> Option<std::fs::File> {
    let opened = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::File::options().create(true).truncate(false).read(true).write(true).open(dir.join(DISABLED_LOCK)));
    let file = opened.and_then(|f| f.lock().map(|()| f));
    if let Err(e) = &file {
        log::warn!("can't lock {}: {e}", dir.join(DISABLED_LOCK).display());
    }
    file.ok()
}

/// The browser has no folders to keep anything in.
#[cfg(target_arch = "wasm32")]
fn lock_disabled(_: &Path) -> Option<std::fs::File> {
    None
}

/// Remove `dir/file`; a file that isn't there is what was wanted.
fn take(dir: &Path, file: &str) {
    let path = dir.join(file);
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("can't remove {}: {e}", path.display());
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

    /// Keep the plug-ins this session installs in `dir`, take the disabled plug-ins saved there
    /// ([`DISABLED_FILE`]), and install the stored ones ([`Session::load_stored_plugins`]).
    pub fn attach_plugin_store(&mut self, dir: PathBuf) {
        let disabled =
            read_capped(&dir.join(DISABLED_FILE), safe_mode::MAX_DISABLED_FILE_BYTES).map(|b| safe_mode::parse_disabled_file(&b)).unwrap_or_default();
        safe_mode::set_disabled(disabled);
        self.plugin_dir = Some(dir);
        self.load_stored_plugins();
    }

    /// Install the plug-ins kept in the plug-in folder: none in Safe Mode, and none the user
    /// disabled (matched by file name, which is made from the id). The launch marker names each
    /// file while it loads. A module that doesn't load, and one that lists permissions the grants
    /// there don't grant that very module ([`grant_covers`]), is skipped with a log record, and
    /// stays in the folder with its grant. A grant for an id whose stored module lists no
    /// permissions is forgotten.
    pub fn load_stored_plugins(&mut self) {
        let Some(dir) = self.plugin_dir.clone() else { return };
        if safe_mode::active() {
            return;
        }
        let grants = stored_grants(&dir);
        let limits = vectorcraft_plugins::Limits::default();
        let mut left = MAX_PLUGIN_STORE;
        let mut asks_nothing = BTreeSet::new();
        for path in list_files(&dir, PLUGIN_EXT) {
            if left == 0 {
                log::warn!("skipping the other stored plug-ins: they hold more than {MAX_PLUGIN_STORE} bytes together");
                break;
            }
            let item = safe_mode::Item::file(path.display().to_string());
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string());
            let by_id = safe_mode::skipped_ids().iter().any(|id| stem.as_deref() == Some(file_stem(id).as_str()));
            if safe_mode::skipped(&item) || by_id {
                log::info!("skipping the stored plug-in {}: disabled", path.display());
                continue;
            }
            let token = safe_mode::loading(&item);
            let loaded = read_capped(&path, (limits.max_module_bytes as u64).min(left)).and_then(|bytes| {
                left -= bytes.len() as u64;
                vectorcraft_plugins::Plugin::load(&bytes, limits.clone()).map_err(|e| e.to_string())
            });
            safe_mode::loaded(token);
            match loaded {
                // Only this store's grants count: one given elsewhere in the process doesn't let a
                // module stored here load.
                Ok(plugin) if plugin.manifest().asks_permissions() => {
                    let m = plugin.manifest();
                    match grants.get(&m.id).filter(|g| grant_covers(g, &plugin)) {
                        Some(g) => {
                            asks_nothing.remove(&m.id);
                            vectorcraft_plugins::registry::grant(g.clone());
                            vectorcraft_plugins::registry::set_storage(&m.id, plugin_storage(Some(&dir), &m.id));
                            vectorcraft_plugins::registry::install(plugin.with_source(path.display().to_string()));
                        }
                        None => {
                            log::warn!("skipping the stored plug-in {}: the user hasn't granted this module the permissions it lists", path.display())
                        }
                    }
                }
                Ok(plugin) => {
                    asks_nothing.insert(plugin.id().to_string());
                    vectorcraft_plugins::registry::set_storage(plugin.id(), plugin_storage(Some(&dir), plugin.id()));
                    vectorcraft_plugins::registry::install(plugin.with_source(path.display().to_string()));
                }
                Err(e) => log::warn!("skipping the stored plug-in {}: {e}", path.display()),
            }
        }
        let kept = read_grants(&dir).unwrap_or_default();
        for id in &asks_nothing {
            vectorcraft_plugins::registry::revoke(id);
            if kept.grants.contains_key(id) || kept.other.iter().any(|e| e.get("id").and_then(serde_json::Value::as_str) == Some(id.as_str())) {
                self.store_grant(id, None);
            }
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
        let Some(dir) = self.plugin_dir.as_deref() else { return false };
        let Some(_turn) = lock_disabled(dir) else { return false };
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
    /// plug-in store when there is one ([`grant_covers`]), else in this process's
    /// ([`vectorcraft_plugins::registry::approved`]).
    pub(crate) fn plugin_approved(&self, plugin: &vectorcraft_plugins::Plugin) -> bool {
        self.plugin_approver()(plugin)
    }

    /// [`Session::plugin_approved`] for many plug-ins: reads the store's grants once, when it is
    /// made, and checks each plug-in against them.
    pub(crate) fn plugin_approver(&self) -> impl Fn(&vectorcraft_plugins::Plugin) -> bool + use<> {
        let grants = self.plugin_dir.as_deref().map(stored_grants);
        move |plugin| {
            let m = plugin.manifest();
            match &grants {
                Some(grants) if m.asks_permissions() => grants.get(&m.id).is_some_and(|g| grant_covers(g, plugin)),
                _ => vectorcraft_plugins::registry::approved(plugin),
            }
        }
    }

    /// Write the module `bytes` of the plug-in `id` to the store → whether they went there (`false`
    /// without a folder).
    pub(crate) fn store_plugin(&self, id: &str, bytes: &[u8]) -> bool {
        self.plugin_dir.as_deref().is_some_and(|dir| put(dir, &format!("{}.{PLUGIN_EXT}", file_stem(id)), bytes))
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
    /// write ([`Session::stored_plugin_bytes`]): with `None`, the file is removed.
    pub(crate) fn put_back_plugin(&self, id: &str, previous: Option<&[u8]>) {
        match previous {
            Some(bytes) => {
                self.store_plugin(id, bytes);
            }
            None => self.unstore_plugin(id),
        }
    }

    /// The file the plug-in `id` is stored in, spelled as [`Session::load_stored_plugins`] names it
    /// (`None` without a plug-in folder).
    pub(crate) fn stored_plugin_file(&self, id: &str) -> Option<String> {
        self.plugin_dir.as_ref().map(|dir| dir.join(format!("{}.{PLUGIN_EXT}", file_stem(id))).display().to_string())
    }

    /// Record in the store what the user granted plug-in `id` (`None`: nothing), leaving the store's
    /// other grants as they are; with none left, remove the file. Entries this version can't use
    /// are written back as they were, except one for `id`. A grants file that is too large, can't
    /// be read or doesn't hold a list is left as it is, with a log record, and nothing is recorded;
    /// so is one whose new content would be larger than `MAX_GRANTS_FILE`. → whether the store now
    /// holds what was asked (`false` without a folder).
    pub(crate) fn store_grant(&self, id: &str, grant: Option<vectorcraft_plugins::permissions::Grant>) -> bool {
        let Some(dir) = &self.plugin_dir else { return false };
        let mut file = match read_grants(dir) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("not updating the stored plug-in grants {}: {e}", dir.join(GRANTS_FILE).display());
                return false;
            }
        };
        let others = file.other.len();
        file.other.retain(|entry| entry.get("id").and_then(serde_json::Value::as_str) != Some(id));
        let changed = match grant {
            Some(g) => file.grants.insert(id.to_string(), g.clone()).as_ref() != Some(&g),
            None => file.grants.remove(id).is_some(),
        } || file.other.len() != others;
        if !changed {
            return true;
        }
        let mut entries: Vec<serde_json::Value> = file.grants.into_values().filter_map(|g| serde_json::to_value(g).ok()).collect();
        entries.extend(file.other);
        if entries.is_empty() {
            take(dir, GRANTS_FILE);
            return true;
        }
        // The file is read back only up to its cap: written compactly when pretty-printed is larger,
        // and not at all when that is too.
        let fits = |bytes: &Vec<u8>| bytes.len() as u64 <= MAX_GRANTS_FILE;
        match serde_json::to_vec_pretty(&entries).ok().filter(fits).or_else(|| serde_json::to_vec(&entries).ok().filter(fits)) {
            Some(bytes) => put(dir, GRANTS_FILE, &bytes),
            None => {
                log::warn!("not updating the stored plug-in grants {}: {}", dir.join(GRANTS_FILE).display(), too_large(MAX_GRANTS_FILE));
                false
            }
        }
    }

    /// Remove the plug-in `id` from the store.
    pub(crate) fn unstore_plugin(&self, id: &str) {
        if let Some(dir) = &self.plugin_dir {
            take(dir, &format!("{}.{PLUGIN_EXT}", file_stem(id)));
        }
    }

    /// Keep the settings and cache of plug-in `id` in the attached plug-in store from now on; without
    /// one, nowhere (its storage calls then fail as unavailable).
    pub(crate) fn attach_plugin_storage(&self, id: &str) {
        vectorcraft_plugins::registry::set_storage(id, plugin_storage(self.plugin_dir.as_deref(), id));
    }

    /// Does the attached plug-in store have a folder of settings and cache for plug-in `id`? A
    /// folder behind a `storage` folder that is a link is not counted: the plug-in's storage calls
    /// can't reach it and removing the plug-in leaves it.
    pub(crate) fn has_plugin_storage(&self, id: &str) -> bool {
        self.plugin_dir
            .as_deref()
            .is_some_and(|dir| !is_link(&dir.join(PLUGIN_DATA_DIR)) && std::fs::symlink_metadata(plugin_data_dir(dir, id)).is_ok())
    }

    /// Delete the settings and cache of plug-in `id` from the store, and stop keeping any for it.
    /// The folder is first renamed out of the way (`<folder>.removed`), and that fails, leaving it
    /// as it was, when the rename does; what the deletion then leaves stays under that name, with a
    /// log record, and is deleted at the plug-in's next removal.
    pub(crate) fn remove_plugin_storage(&self, id: &str) -> Result<(), StorageError> {
        vectorcraft_plugins::registry::set_storage(id, None);
        let Some(dir) = &self.plugin_dir else { return Ok(()) };
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = plugin_data_dir(dir, id);
        // Every storage handed out for the folder, including those runs still hold, is closed once
        // the folder is out of the way; until then they keep working.
        let close = || {
            if let Some(closed) = PLUGIN_DATA_OPEN.lock().unwrap_or_else(|e| e.into_inner()).remove(&path) {
                closed.store(true, Ordering::Release);
            }
        };
        // A `storage` folder that is a link leads out of the store: nothing there is deleted. A link
        // in place of the plug-in's own folder is removed itself (`remove_dir_all` doesn't follow it).
        let data = dir.join(PLUGIN_DATA_DIR);
        if is_link(&data) {
            log::warn!("{} is a link: the plug-in settings and cache it leads to are left", data.display());
            close();
            return Ok(());
        }
        let mut aside = path.clone().into_os_string();
        aside.push(".removed");
        let aside = PathBuf::from(aside);
        let _ = std::fs::remove_dir_all(&aside);
        match std::fs::rename(&path, &aside) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StorageError::Io(format!("{}: {e}", path.display()))),
            Ok(()) => {}
        }
        close();
        if std::fs::symlink_metadata(&aside).is_ok()
            && let Err(e) = std::fs::remove_dir_all(&aside)
        {
            log::warn!("can't delete {}: {e}", aside.display());
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

/// Where plug-in `id` keeps its settings and cache in the plug-in store `dir`.
fn plugin_storage(dir: Option<&Path>, id: &str) -> Option<Arc<dyn Storage>> {
    let dir = dir?;
    let data = plugin_data_dir(dir, id);
    let closed = PLUGIN_DATA_OPEN.lock().unwrap_or_else(|e| e.into_inner()).entry(data.clone()).or_default().clone();
    Some(Arc::new(PluginStorage { root: dir.to_path_buf(), dir: data, closed }))
}

/// The settings and cache of one plug-in, in its folder of the plug-in store (see the module docs).
/// Keys and entry names are [`storage::valid_name`]s; an entry's file name is its name as
/// [`case_safe`] writes it, so that names differing only in case get different files. A link
/// anywhere in the folder's path below the store is refused, and so is a cache file that is not a
/// plain file.
pub(crate) struct PluginStorage {
    /// The plug-in store, and the plug-in's folder in it.
    root: PathBuf,
    dir: PathBuf,
    /// Set, under [`PLUGIN_DATA_LOCK`], once the plug-in is removed; shared by every storage of the
    /// folder ([`PLUGIN_DATA_OPEN`]).
    closed: Arc<AtomicBool>,
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

    /// The settings, by key. A file that is too large or doesn't parse is moved aside
    /// ([`BAD_SETTINGS_FILE`]) with a log record, and the settings are empty. `meter` is charged the
    /// file's size before it is read.
    fn read_settings(&self, meter: &mut Meter) -> Result<BTreeMap<String, Vec<u8>>, StorageError> {
        let path = self.settings_path();
        self.no_links(&path)?;
        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(io(&e)),
        };
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
            Some(settings) => Ok(settings),
            None => {
                let aside = self.dir.join(BAD_SETTINGS_FILE);
                log::warn!("the plug-in settings {} don't read: moved to {} and started empty", path.display(), aside.display());
                std::fs::rename(&path, &aside).map_err(|e| io(&e))?;
                Ok(BTreeMap::new())
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
        if self.closed.load(Ordering::Acquire) { Err(StorageError::Unavailable) } else { Ok(()) }
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
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        Ok(self.read_settings(meter)?.remove(key))
    }

    fn set_setting(&self, key: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError> {
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let mut settings = self.read_settings(meter)?;
        if value.len() > storage::MAX_SETTING_BYTES || (!settings.contains_key(key) && settings.len() >= storage::MAX_SETTINGS) {
            return Err(StorageError::Full);
        }
        settings.insert(key.to_string(), value.to_vec());
        self.write_settings(&settings, meter)
    }

    fn delete_setting(&self, key: &str, meter: &mut Meter) -> Result<bool, StorageError> {
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let mut settings = self.read_settings(meter)?;
        if settings.remove(key).is_none() {
            return Ok(false);
        }
        self.write_settings(&settings, meter).map(|()| true)
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
        write_atomic(&path, value).map_err(|e| io(&e))
    }

    fn remove_cache_entry(&self, name: &str) -> Result<bool, StorageError> {
        let _held = PLUGIN_DATA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.open()?;
        let path = self.entry_path(name);
        self.no_links(&path)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(&e)),
        }
    }

    fn clear_cache(&self) -> Result<u64, StorageError> {
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
        Ok(freed)
    }

    fn cache_bytes(&self) -> u64 {
        if self.open().is_err() {
            return 0;
        }
        self.cache_files().map(|(files, _)| sum_sizes(&files)).unwrap_or(0)
    }
}

/// What the grants file of a plug-in store holds.
#[derive(Default)]
struct GrantsFile {
    /// The grants this version can use, by plug-in id.
    grants: BTreeMap<String, vectorcraft_plugins::permissions::Grant>,
    /// The entries it can't use (a permission it doesn't know, an invalid id), as they were.
    other: Vec<serde_json::Value>,
}

/// The grants file of the plug-in store `dir`: empty when there is none, an error when it is too
/// large, can't be read or doesn't hold a list. An entry that doesn't parse as a grant, or names an
/// invalid id, is kept in [`GrantsFile::other`] with a log record.
fn read_grants(dir: &Path) -> Result<GrantsFile, String> {
    let path = dir.join(GRANTS_FILE);
    if !path.exists() {
        return Ok(GrantsFile::default());
    }
    let entries =
        read_capped(&path, MAX_GRANTS_FILE).and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).map_err(|e| e.to_string()))?;
    let mut file = GrantsFile::default();
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
/// there is no grants file, and none, with a log record, when it is too large, can't be read or
/// doesn't hold a list.
fn stored_grants(dir: &Path) -> BTreeMap<String, vectorcraft_plugins::permissions::Grant> {
    match read_grants(dir) {
        Ok(file) => file.grants,
        Err(e) => {
            log::warn!("skipping the stored plug-in grants {}: {e}", dir.join(GRANTS_FILE).display());
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
                && builtin_preset(&name).is_none()
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
