//! The process-wide set of installed plug-ins.
//!
//! Plug-ins are installed per process, not per document: live effects are drawn from every open
//! document (and from scratch sessions such as previews), and they must find the same plug-ins.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::Error;
use crate::permissions::{Grant, GrantToken};
use crate::storage::Storage;
use crate::{Limits, Plugin, Result};

static PLUGINS: RwLock<BTreeMap<String, Arc<Plugin>>> = RwLock::new(BTreeMap::new());
static REVISION: AtomicU64 = AtomicU64::new(0);
/// What the user approved, by plug-in id.
static GRANTS: RwLock<BTreeMap<String, Grant>> = RwLock::new(BTreeMap::new());
/// Where each plug-in keeps its settings and cache, by plug-in id.
static STORAGE: RwLock<BTreeMap<String, Arc<dyn Storage>>> = RwLock::new(BTreeMap::new());

/// Keeps the settings and cache of plug-in `id` in `storage` from now on (`None`: nowhere, so its
/// storage calls fail as unavailable). A run of the plug-in reaches only its own id's storage.
pub fn set_storage(id: &str, storage: Option<Arc<dyn Storage>>) {
    let mut all = STORAGE.write().unwrap_or_else(|e| e.into_inner());
    match storage {
        Some(s) => all.insert(id.to_string(), s),
        None => all.remove(id),
    };
}

/// Where plug-in `id` keeps its settings and cache, if anywhere.
pub fn storage(id: &str) -> Option<Arc<dyn Storage>> {
    STORAGE.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// Records what the user approved for a plug-in, replacing an earlier grant for its id. Only a
/// [`GrantToken`] is recorded, which the engine makes after an approval or when it loads its grants
/// file.
pub fn grant(token: GrantToken) {
    let g = token.grant().clone();
    GRANTS.write().unwrap_or_else(|e| e.into_inner()).insert(g.id.clone(), g);
}

/// What the user approved for plug-in `id`.
pub fn granted(id: &str) -> Option<Grant> {
    GRANTS.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// Every grant, sorted by id.
pub fn grants() -> Vec<Grant> {
    GRANTS.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
}

/// Forgets what the user approved for `id`; false if nothing was.
pub fn revoke(id: &str) -> bool {
    GRANTS.write().unwrap_or_else(|e| e.into_inner()).remove(id).is_some()
}

/// May `plugin` be installed without asking the user: does it list no permissions, or did the user
/// already grant this very module (its id and SHA-256) every permission it lists? A module with
/// other bytes needs a new approval, even when it lists the same permissions.
pub fn approved(plugin: &Plugin) -> bool {
    let m = plugin.manifest();
    !m.asks_permissions() || granted(&m.id).is_some_and(|g| g.covers(&m.id, plugin.hash(), &m.permissions()))
}

/// Tests: what runs before an install waits for the plug-ins, given the plug-in's id.
#[cfg(test)]
type InstallHook = Arc<dyn Fn(&str) + Send + Sync>;

/// Tests: runs right before [`install`] waits for the plug-ins' write lock.
#[cfg(test)]
static BEFORE_INSTALL_LOCK: Mutex<Option<InstallHook>> = Mutex::new(None);

/// Installs `plugin`, replacing one with the same id, when it is [`approved`]: a module that lists
/// permissions no recorded grant gives this very module is refused ([`Error::NeedsApproval`]), and
/// the plug-in installed under its id, the grants and the revision stay as they were.
pub fn install(plugin: Plugin) -> Result<Arc<Plugin>> {
    // The revision moves while the write lock is held, so a cache that reads the new revision and
    // then the plug-ins finds the new plug-in among them. The approval is checked while it is held
    // too.
    #[cfg(test)]
    {
        let hook = BEFORE_INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(hook) = hook {
            hook(plugin.id());
        }
    }
    let mut plugins = PLUGINS.write().unwrap_or_else(|e| e.into_inner());
    if !approved(&plugin) {
        return Err(Error::NeedsApproval(crate::permissions::names(&plugin.manifest().permissions()).join(", ")));
    }
    Ok(insert(&mut plugins, plugin))
}

/// Installs `plugin` in `plugins`, which the plug-ins' write lock holds, with the next revision.
fn insert(plugins: &mut BTreeMap<String, Arc<Plugin>>, mut plugin: Plugin) -> Arc<Plugin> {
    plugin.set_serial(REVISION.fetch_add(1, Ordering::Relaxed) + 1);
    note_change(plugin.id(), plugin.serial());
    let p = Arc::new(plugin);
    plugins.insert(p.id().to_string(), p.clone());
    p
}

/// [`install`] for a load that installs plug-ins one by one, recording `grant` first when it is
/// given: while the plug-ins' write lock is held, a plug-in whose id was installed or removed
/// elsewhere since the load started ([`LoadChanges::superseded`]) is not installed (`Ok(None)`), and
/// neither the plug-ins nor the grants change. A grant that doesn't give this very module every
/// permission it lists is refused ([`Error::NeedsApproval`]) and not recorded. Otherwise `attach`
/// runs, then the grant is recorded and the plug-in installed. `attach` runs while the plug-ins are
/// held, so it may call [`set_storage`] but no function that reads or changes the plug-ins or grants.
pub fn install_for_load(plugin: Plugin, changes: &LoadChanges, grant: Option<GrantToken>, attach: impl FnOnce()) -> Result<Option<Arc<Plugin>>> {
    let mut plugins = PLUGINS.write().unwrap_or_else(|e| e.into_inner());
    if changes.superseded(plugin.id()) {
        return Ok(None);
    }
    let m = plugin.manifest();
    let approved = match &grant {
        Some(token) => !m.asks_permissions() || token.grant().covers(&m.id, plugin.hash(), &m.permissions()),
        None => approved(&plugin),
    };
    if !approved {
        return Err(Error::NeedsApproval(crate::permissions::names(&m.permissions()).join(", ")));
    }
    attach();
    if let Some(token) = grant {
        let g = token.grant().clone();
        GRANTS.write().unwrap_or_else(|e| e.into_inner()).insert(g.id.clone(), g);
    }
    Ok(Some(insert(&mut plugins, plugin)))
}

/// Forgets the grant for `id` when the plug-in installed under it is the one that took the revision
/// `serial` ([`Plugin::serial`]), checked while the plug-ins are held, so a plug-in installed in its
/// place meanwhile keeps the grant recorded for it → whether a grant was forgotten.
pub fn revoke_if_installed(id: &str, serial: u64) -> bool {
    // The same order as `install`, which reads the grants while it holds the plug-ins.
    let plugins = PLUGINS.read().unwrap_or_else(|e| e.into_inner());
    plugins.get(id).is_some_and(|p| p.serial() == serial) && GRANTS.write().unwrap_or_else(|e| e.into_inner()).remove(id).is_some()
}

/// The [`revision`] of each id's last install or removal ([`changed_since`]).
static CHANGED: RwLock<BTreeMap<String, u64>> = RwLock::new(BTreeMap::new());

/// Records that `id` was installed or removed at revision `rev`; called while the plug-ins' write
/// lock is held.
fn note_change(id: &str, rev: u64) {
    CHANGED.write().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), rev);
}

/// Whether plug-in `id` was installed or removed after [`revision`] gave `rev`.
pub fn changed_since(id: &str, rev: u64) -> bool {
    CHANGED.read().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|&r| r > rev)
}

/// What a load that installs plug-ins one by one has to check, so that a module it loaded before
/// another session installed or removed the same id doesn't undo that change: the [`revision`] when
/// the load listed its files, and the revision of each id it, or a load of the same session that
/// ended before it, installed.
#[derive(Debug, Default)]
pub struct LoadChanges {
    since: Option<u64>,
    own: BTreeMap<String, u64>,
}

impl LoadChanges {
    /// Marks the load as started now, unless it started already. A load calls it as it lists its
    /// files, so a change made between the listing and the loads counts as one made elsewhere.
    pub fn start(&mut self) {
        self.since.get_or_insert_with(revision);
    }

    /// Whether `id` was installed or removed since the load started, other than by the load itself
    /// or a load it [adopted](LoadChanges::adopt).
    pub fn superseded(&self, id: &str) -> bool {
        self.since.is_some_and(|since| changed_since(id, since.max(self.own.get(id).copied().unwrap_or(0))))
    }

    /// Records that the load installed `plugin`.
    pub fn installed(&mut self, plugin: &Plugin) {
        self.own.insert(plugin.id().to_string(), plugin.serial());
    }

    /// Counts what `earlier`, a load of the same session queued before this one that has ended,
    /// installed as this load's own: a module this load installs after it then replaces it, as it
    /// would have with the loads run one after the other.
    pub fn adopt(&mut self, earlier: &LoadChanges) {
        for (id, &serial) in &earlier.own {
            let own = self.own.entry(id.clone()).or_default();
            *own = (*own).max(serial);
        }
    }
}

/// What a load reports for a module it skips because the plug-in under its id was installed or
/// removed elsewhere while it loaded ([`LoadChanges`]).
pub const SUPERSEDED: &str = "skipped: the plug-in with its id was installed or removed elsewhere while it loaded";

/// What [`install_checked`] does to the grant recorded for the plug-in's id.
pub enum GrantUpdate {
    /// Records this grant, replacing an earlier one for the id ([`grant`]).
    Record(GrantToken),
    /// Forgets the grant for the id ([`revoke`]).
    Revoke,
}

/// Installs `plugin` and updates the grant for its id as one step. While it holds the plug-ins and
/// then the grants, `check` is given the plug-in installed under the id and the grant recorded for
/// it. When `check` returns an error, nothing changes and that error is returned. A plug-in that
/// lists permissions the grant `update` records doesn't give this very module is refused the same
/// way, with [`Error::NeedsApproval`] passed through `refused`. Otherwise `attach` runs, the grant
/// is updated and the plug-in is installed, replacing one with the same id. `check` and `attach`
/// must not call the other functions of this module, which wait for the locks held here.
pub fn install_checked<E>(
    mut plugin: Plugin,
    update: GrantUpdate,
    check: impl FnOnce(Option<&Plugin>, Option<&Grant>) -> std::result::Result<(), E>,
    attach: impl FnOnce(),
    refused: impl FnOnce(Error) -> E,
) -> std::result::Result<Arc<Plugin>, E> {
    // The same order as `install`, which reads the grants while it holds the plug-ins.
    let mut plugins = PLUGINS.write().unwrap_or_else(|e| e.into_inner());
    let mut grants = GRANTS.write().unwrap_or_else(|e| e.into_inner());
    let id = plugin.id().to_string();
    check(plugins.get(&id).map(|p| &**p), grants.get(&id))?;
    let m = plugin.manifest();
    let granted = match &update {
        GrantUpdate::Record(token) => Some(token.grant()),
        GrantUpdate::Revoke => None,
    };
    if m.asks_permissions() && !granted.is_some_and(|g| g.covers(&m.id, plugin.hash(), &m.permissions())) {
        return Err(refused(Error::NeedsApproval(crate::permissions::names(&m.permissions()).join(", "))));
    }
    attach();
    match update {
        GrantUpdate::Record(token) => {
            let g = token.grant().clone();
            grants.insert(g.id.clone(), g);
        }
        GrantUpdate::Revoke => {
            grants.remove(&id);
        }
    }
    plugin.set_serial(REVISION.fetch_add(1, Ordering::Relaxed) + 1);
    note_change(&id, plugin.serial());
    let p = Arc::new(plugin);
    plugins.insert(id, p.clone());
    Ok(p)
}

/// Loads `bytes` with the default limits and installs the plug-in ([`install`]: one that lists
/// permissions installs only when they were granted to this very module).
pub fn install_bytes(bytes: &[u8]) -> Result<Arc<Plugin>> {
    install(Plugin::load(bytes, Limits::default())?)
}

pub fn get(id: &str) -> Option<Arc<Plugin>> {
    PLUGINS.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// Installed plug-ins, sorted by id.
pub fn list() -> Vec<Arc<Plugin>> {
    PLUGINS.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
}

/// Uninstalls `id`; false if it wasn't installed.
pub fn remove(id: &str) -> bool {
    // The revision moves while the write lock is held, as in `install`.
    let mut plugins = PLUGINS.write().unwrap_or_else(|e| e.into_inner());
    let gone = plugins.remove(id).is_some();
    if gone {
        note_change(id, REVISION.fetch_add(1, Ordering::Relaxed) + 1);
    }
    gone
}

/// Changes whenever a plug-in is installed or removed (for caches such as menus).
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

/// `s` as a `&'static str`, for catalogues that hold static strings (effect and menu labels). Each
/// distinct string is stored once, so memory stays bounded by the plug-ins ever installed.
pub fn intern(s: &str) -> &'static str {
    static SET: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut set = SET.lock().unwrap_or_else(|e| e.into_inner());
    let set = set.get_or_insert_with(HashSet::new);
    if let Some(v) = set.get(s) {
        return v;
    }
    let v: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(v);
    v
}

/// Reads a `.wasm` file (refusing anything that isn't a regular file or exceeds the module limit),
/// or a JavaScript plug-in's `.vcplugin` manifest and the script it names ([`read_js_package`]),
/// and loads it.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(path: &std::path::Path, limits: Limits) -> Result<Plugin> {
    load_file_where(path, limits, &|_| Ok(()))
}

/// [`load_file`], reading a JavaScript plug-in's script only when `allow` accepts its path.
#[cfg(not(target_arch = "wasm32"))]
fn load_file_where(path: &std::path::Path, limits: Limits, allow: &dyn Fn(&std::path::Path) -> std::result::Result<(), String>) -> Result<Plugin> {
    let bytes = if is_js_package(path) { read_js_package(path, allow)? } else { read_module(path, &limits, || {})? };
    Ok(Plugin::load(&bytes, limits)?.with_source(path.display().to_string()))
}

/// Does `path` name a JavaScript plug-in's manifest file: is its extension `.vcplugin`?
pub fn is_js_package(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case(crate::js::PACKAGE_EXT))
}

/// The JavaScript plug-in whose manifest file is `path`, as the one JSON object [`Plugin::load`]
/// reads: the manifest, with the text of the script its `script` names in `source`. The manifest
/// file must give `"runtime": "js"` and `script`, a `.js` file in the same folder, and no `source`.
/// Each file is refused before it is read when it isn't a regular file or is larger than its limit
/// ([`crate::manifest::MAX_MANIFEST_BYTES`], [`crate::js::MAX_SCRIPT_BYTES`]); the script is read
/// only when `allow` accepts its path, and must be UTF-8.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_js_package(path: &std::path::Path, allow: &dyn Fn(&std::path::Path) -> std::result::Result<(), String>) -> Result<Vec<u8>> {
    read_js_package_with(path, allow, || {})
}

/// [`read_js_package`], with `opening` run between the script's size check and its open.
#[cfg(not(target_arch = "wasm32"))]
fn read_js_package_with(
    path: &std::path::Path,
    allow: &dyn Fn(&std::path::Path) -> std::result::Result<(), String>,
    opening: impl FnOnce(),
) -> Result<Vec<u8>> {
    use serde_json::Value;
    let manifest_limits = Limits { max_module_bytes: crate::manifest::MAX_MANIFEST_BYTES, ..Limits::default() };
    let bytes = read_module(path, &manifest_limits, || {})?;
    let map: serde_json::Map<String, Value> = serde_json::from_slice(&bytes).map_err(|e| Error::Manifest(format!("{}: {e}", path.display())))?;
    if !crate::js::is_js(&map) {
        return Err(Error::Manifest(crate::js::NOT_JS.into()));
    }
    if map.contains_key("source") {
        return Err(Error::Manifest(crate::js::SOURCE_RULE.into()));
    }
    let name = crate::js::script_name(map.get("script").ok_or_else(crate::js::no_script)?)?;
    let file = path.parent().unwrap_or(std::path::Path::new("")).join(&name);
    allow(&file).map_err(Error::Io)?;
    let script_limits = Limits { max_module_bytes: crate::js::MAX_SCRIPT_BYTES, ..Limits::default() };
    let source = read_module(&file, &script_limits, opening)?;
    let source = String::from_utf8(source).map_err(|_| Error::Module("the script is not UTF-8 text".into()))?;
    // `source` is added to the manifest's own text, so its keys keep their order. The text parsed as
    // an object that has `runtime`, so its last character other than white space is its closing `}`.
    let end = bytes.iter().rposition(|b| !b.is_ascii_whitespace()).filter(|&i| bytes[i] == b'}');
    let end = end.ok_or_else(|| Error::Manifest(format!("{}: not a JSON object", path.display())))?;
    let source = serde_json::to_vec(&Value::String(source)).map_err(|e| Error::Manifest(e.to_string()))?;
    let mut out = Vec::with_capacity(bytes.len() + source.len() + 12);
    out.extend_from_slice(&bytes[..end]);
    out.extend_from_slice(b",\"source\":");
    out.extend_from_slice(&source);
    out.push(b'}');
    Ok(out)
}

/// The bytes of the module file at `path`. A path that doesn't name a regular file is refused
/// before it is opened, and so is one over the module limit. On Unix the file is opened without
/// waiting, so a named pipe put in its place after that check doesn't hold the load until something
/// writes to it; what was opened is checked again and refused the same way. `opening` runs between
/// the first check and the open.
#[cfg(not(target_arch = "wasm32"))]
fn read_module(path: &std::path::Path, limits: &Limits, opening: impl FnOnce()) -> Result<Vec<u8>> {
    use std::io::Read;
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let regular = |meta: std::fs::Metadata| {
        if !meta.is_file() {
            return Err(Error::Io(format!("{} is not a file", path.display())));
        }
        if meta.len() > limits.max_module_bytes as u64 {
            return Err(Error::Module(format!("{} is {} bytes (limit {})", path.display(), meta.len(), limits.max_module_bytes)));
        }
        Ok(())
    };
    regular(std::fs::metadata(path).map_err(io)?)?;
    opening();
    let f = open_module(path).map_err(io)?;
    regular(f.metadata().map_err(io)?)?;
    let mut bytes = Vec::new();
    f.take(limits.max_module_bytes as u64 + 1).read_to_end(&mut bytes).map_err(io)?;
    Ok(bytes)
}

/// Opens the module file at `path` for reading. On Unix the open doesn't wait for a writer when
/// the path names a named pipe, and doesn't make a terminal device the controlling terminal.
#[cfg(not(target_arch = "wasm32"))]
fn open_module(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut open, (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOCTTY).bits() as i32);
    open.open(path)
}

/// The most `.wasm` files one [`load_folder`] reads.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FOLDER_FILES: usize = 1000;

/// The first [`MAX_FOLDER_FILES`] `*.wasm` and `*.vcplugin` paths directly in `dir` by file name, sorted, and how
/// many more there are. Only that many paths are held while the folder is listed. An error while
/// the folder is listed is returned, as one opening it is.
#[cfg(not(target_arch = "wasm32"))]
fn folder_files(dir: &std::path::Path) -> Result<(Vec<std::path::PathBuf>, usize)> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", dir.display()));
    let rd = std::fs::read_dir(dir).map_err(io)?;
    let mut kept = std::collections::BinaryHeap::new();
    let mut more = 0;
    for entry in rd {
        let p = entry.map_err(io)?.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wasm")) && !is_js_package(&p) {
            continue;
        }
        kept.push(p);
        if kept.len() > MAX_FOLDER_FILES {
            kept.pop();
            more += 1;
        }
    }
    Ok((kept.into_sorted_vec(), more))
}

/// What [`load_folder`] reports for a plug-in the user disabled.
pub const DISABLED: &str = "disabled: turn it on again in Help › Safe Mode, for the next launch";

/// What [`load_folder`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderReport {
    /// Ids of the plug-ins installed.
    pub loaded: Vec<String>,
    /// `(file, error)` for each `.wasm` that failed to load, including one that isn't a regular file.
    pub failed: Vec<(String, String)>,
    /// How many `.wasm` files were not tried because the folder holds more than a load reads.
    pub left_out: usize,
    /// The disabled items saved for the next launch changed: a plug-in disabled by its id alone was
    /// found in a file, which is now disabled too ([`crate::safe_mode::disable_file_of`]). The app
    /// saves the list again.
    pub disabled_changed: bool,
    /// A file failed because no loader thread was left to load it ([`crate::loader::NO_THREAD`]):
    /// loading the folder again may load it.
    pub no_thread: bool,
}

/// Installs every `*.wasm` and `*.vcplugin` ([`read_js_package`]) directly in `dir` (not recursive), in file-name order, up to the first
/// 1,000 by file name; the report counts the files left out ([`FolderReport::left_out`]). Anything
/// that isn't a regular file, such as a named pipe, is reported and skipped without waiting on it
/// ([`load_file`]). The modules load on worker threads ([`FolderLoad`]) and install in file-name
/// order. A bad module is reported and skipped, the others still load; so is one whose load panics
/// or doesn't finish in time ([`crate::loader`]), a heavy one ([`crate::Budget::Heavy`]), which
/// installs only once the user agrees, and one that lists permissions the user has not granted this
/// very module ([`approved`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_folder(dir: &std::path::Path) -> Result<FolderReport> {
    load_folder_where(dir, |_| Ok(()), approved)
}

/// [`load_folder`], loading only the files `allow` accepts (a JavaScript plug-in's script too), and of the plug-ins that list
/// permissions only those `approve` accepts and a recorded grant covers ([`install`]); the others
/// are reported as failed with the reason. A plug-in the user disabled ([`crate::safe_mode::skipped`]) is reported and
/// skipped, by its file before it loads or by its id after (its file is then disabled too, so the
/// next load skips it before it runs); the launch marker names each file while it loads
/// ([`crate::safe_mode::loading`]). In Safe Mode nothing loads ([`Error::SafeMode`]). When the
/// module a load leaves under an id lists no permissions, the process's grant for that id is
/// forgotten.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_folder_where(
    dir: &std::path::Path,
    allow: impl Fn(&std::path::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
    approve: impl Fn(&Plugin) -> bool,
) -> Result<FolderReport> {
    let mut load = FolderLoad::begin(dir, allow)?;
    while !load.done() {
        load.wait();
        load.install_ready(&approve);
    }
    Ok(load.finish())
}

/// Tests: what runs before a folder module loads, given its path.
#[cfg(all(test, not(target_arch = "wasm32")))]
type FileHook = std::sync::Arc<dyn Fn(&std::path::Path) + Send + Sync>;

/// Tests: runs on the worker thread before the module at the path it is given loads.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) static BEFORE_FOLDER_FILE: Mutex<Option<FileHook>> = Mutex::new(None);

/// A [`load_folder_where`] whose modules load on worker threads ([`crate::loader`]) while the
/// caller goes on: [`FolderLoad::begin`] lists the folder and starts the loads,
/// [`FolderLoad::install_ready`] installs what loaded so far in file-name order, whatever order the
/// loads finish in, and [`FolderLoad::finish`] ends the load once [`FolderLoad::done`]. A module
/// whose load panics or doesn't finish in time is reported as failed, and the others still install.
#[cfg(not(target_arch = "wasm32"))]
pub struct FolderLoad {
    /// Each file in file-name order, with why it was refused before loading, if it was.
    files: Vec<(std::path::PathBuf, Option<String>)>,
    loads: crate::loader::Loads<Plugin>,
    /// Outcomes taken from `loads` and not installed yet, in order.
    loaded: std::collections::VecDeque<Result<Plugin>>,
    /// The next file to install or report.
    next: usize,
    report: FolderReport,
    /// Installs and removals elsewhere since the folder was listed, which a module loaded before
    /// them doesn't undo.
    changes: LoadChanges,
}

#[cfg(not(target_arch = "wasm32"))]
impl FolderLoad {
    /// Lists `dir` and starts loading the files `allow` accepts and the user didn't disable. Each
    /// file is checked with `allow` again on its worker thread, right before it is read. In Safe
    /// Mode nothing loads ([`Error::SafeMode`]); an error listing the folder is returned.
    pub fn begin(
        dir: &std::path::Path,
        allow: impl Fn(&std::path::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
    ) -> Result<FolderLoad> {
        let mut load = Self::list_on(dir, allow, None)?;
        load.start();
        Ok(load)
    }

    /// [`FolderLoad::begin`] without starting the loads: [`FolderLoad::start`] starts them, and
    /// so does waiting for them.
    pub fn list(
        dir: &std::path::Path,
        allow: impl Fn(&std::path::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
    ) -> Result<FolderLoad> {
        Self::list_on(dir, allow, None)
    }

    /// [`FolderLoad::list`] for the one file `file` of a folder, without listing the folder: a file
    /// that is no longer there is reported as failed.
    pub fn list_file(
        file: &std::path::Path,
        allow: impl Fn(&std::path::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
    ) -> Result<FolderLoad> {
        Self::list_found(Ok((vec![file.to_path_buf()], 0)), allow, None)
    }

    /// Starts the loads, unless they started already.
    pub fn start(&mut self) {
        self.changes.start();
        self.loads.begin();
    }

    /// [`FolderLoad::list`] for `workers` threads (`None`: as many as [`crate::loader::Loads::start`]
    /// starts).
    fn list_on(
        dir: &std::path::Path,
        allow: impl Fn(&std::path::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
        workers: Option<usize>,
    ) -> Result<FolderLoad> {
        Self::list_found(Err(dir), allow, workers)
    }

    /// [`FolderLoad::list_on`] for the files of folder `Err(dir)`, or for the files and the count of
    /// files left out given as `Ok`.
    fn list_found(
        found: std::result::Result<(Vec<std::path::PathBuf>, usize), &std::path::Path>,
        allow: impl Fn(&std::path::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
        workers: Option<usize>,
    ) -> Result<FolderLoad> {
        use crate::loader::{Job, Loads};
        use crate::safe_mode::{self, Item};
        safe_mode::check()?;
        // A change made elsewhere once the folder starts being listed is one the load doesn't undo.
        let mut changes = LoadChanges::default();
        changes.start();
        let (found, left_out) = match found {
            Ok(found) => found,
            Err(dir) => folder_files(dir)?,
        };
        let allow = std::sync::Arc::new(allow);
        let mut files = Vec::with_capacity(found.len());
        let mut jobs = Vec::new();
        for f in found {
            let item = Item::file(f.display().to_string());
            let refused = match allow(&f) {
                Err(why) => Some(why),
                Ok(()) if safe_mode::skipped(&item) => Some(DISABLED.to_string()),
                Ok(()) => None,
            };
            if refused.is_none() {
                let (path, allow) = (f.clone(), allow.clone());
                jobs.push(Job::new(Some(item), move || {
                    allow(&path).map_err(Error::Io)?;
                    #[cfg(test)]
                    {
                        let hook = BEFORE_FOLDER_FILE.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        if let Some(hook) = hook {
                            hook(&path);
                        }
                    }
                    load_file_where(&path, Limits::default(), &*allow)
                }));
            }
            files.push((f, refused));
        }
        let loads = match workers {
            Some(n) => Loads::deferred_on(jobs, n),
            None => Loads::deferred(jobs),
        };
        let report = FolderReport { left_out, ..FolderReport::default() };
        Ok(FolderLoad { files, loads, loaded: Default::default(), next: 0, report, changes })
    }

    /// The installs and removals the load checks its modules against ([`LoadChanges`]).
    pub fn changes(&self) -> &LoadChanges {
        &self.changes
    }

    /// [`FolderLoad::changes`], to [adopt](LoadChanges::adopt) an earlier load's.
    pub fn changes_mut(&mut self) -> &mut LoadChanges {
        &mut self.changes
    }

    /// Whether every file has been installed or reported.
    pub fn done(&self) -> bool {
        self.next == self.files.len()
    }

    /// Whether [`FolderLoad::install_ready`] has something to install or report now, without
    /// waiting (or nothing is left).
    pub fn ready(&mut self) -> bool {
        match self.files.get(self.next) {
            None => true,
            Some((_, Some(_))) => true,
            Some((_, None)) => !self.loaded.is_empty() || self.loads.next_ready(),
        }
    }

    /// Waits until [`FolderLoad::ready`], starting the loads first ([`FolderLoad::start`]).
    pub fn wait(&mut self) {
        self.start();
        while !self.ready() && !self.loads.taken() {
            self.loads.wait_next();
        }
    }

    /// Installs the plug-ins that loaded, and reports the files that failed, in file-name order up
    /// to the first file still loading, as [`load_folder_where`] does: those that list permissions
    /// only when `approve` accepts them and a recorded grant covers them → the ids it installed.
    pub fn install_ready(&mut self, approve: impl Fn(&Plugin) -> bool) -> Vec<String> {
        use crate::safe_mode::{self, Item};
        self.loaded.extend(self.loads.ready());
        let mut installed = Vec::new();
        while let Some((f, refused)) = self.files.get(self.next) {
            let name = f.display().to_string();
            if let Some(why) = refused {
                self.report.failed.push((name, why.clone()));
                self.next += 1;
                continue;
            }
            let Some(loaded) = self.loaded.pop_front() else { break };
            self.loads.release(1);
            self.next += 1;
            let report = &mut self.report;
            match loaded {
                Ok(p) if safe_mode::skipped(&Item::of(&p)) => {
                    report.disabled_changed |= safe_mode::disable_file_of(&Item::of(&p));
                    report.failed.push((name, DISABLED.into()));
                }
                // A heavy plug-in installs once the user agrees, which loading a folder doesn't ask.
                Ok(p) if p.manifest().budget == crate::Budget::Heavy => {
                    report.failed.push((name, "asks for a heavy budget, which the user allows when installing it on its own".into()));
                }
                // The very module installed elsewhere meanwhile (another session loading the same
                // folder) counts as loaded. `approve` may record a grant, so it is asked only for
                // that module: a module another session replaced or removed changes no grant.
                Ok(p) if self.changes.superseded(p.id()) => match get(p.id()) {
                    Some(q) if q.hash() == p.hash() && (!p.manifest().asks_permissions() || approve(&p)) => report.loaded.push(p.id().to_string()),
                    Some(q) if q.hash() == p.hash() => {
                        let asked = crate::permissions::names(&p.manifest().permissions()).join(", ");
                        report.failed.push((name, Error::NeedsApproval(asked).to_string()));
                    }
                    _ => report.failed.push((name, SUPERSEDED.into())),
                },
                Ok(p) if p.manifest().asks_permissions() && !approve(&p) => {
                    let asked = crate::permissions::names(&p.manifest().permissions()).join(", ");
                    report.failed.push((name, Error::NeedsApproval(asked).to_string()));
                }
                // `approve` only narrows what installs: `install_for_load` still refuses a module that
                // no recorded grant covers, and one whose id changed elsewhere since the check above.
                Ok(p) => match install_for_load(p, &self.changes, None, || {}) {
                    Ok(Some(p)) => {
                        self.changes.installed(&p);
                        report.loaded.push(p.id().to_string());
                        installed.push(p.id().to_string());
                    }
                    Ok(None) => report.failed.push((name, SUPERSEDED.into())),
                    Err(e) => report.failed.push((name, e.to_string())),
                },
                Err(e) => {
                    report.no_thread |= matches!(&e, Error::Failed(m) if m == crate::loader::NO_THREAD);
                    report.failed.push((name, e.to_string()));
                }
            }
        }
        installed
    }

    /// Whether [`FolderLoad::install_ready`] turned off the file of a disabled plug-in it matched by
    /// id ([`crate::safe_mode::disable_file_of`]) since the list of disabled plug-ins was last saved
    /// ([`FolderLoad::disabled_saved`]).
    pub fn disabled_changed(&self) -> bool {
        self.report.disabled_changed
    }

    /// The list of disabled plug-ins was saved: [`FolderLoad::disabled_changed`] is false until the
    /// load changes it again.
    pub fn disabled_saved(&mut self) {
        self.report.disabled_changed = false;
    }

    /// Drops the loads not taken yet: jobs not started don't start, and running ones are given
    /// until their deadline ([`crate::loader::Loads`]). What was installed stays, and the load is
    /// [`FolderLoad::done`].
    pub fn cancel(&mut self) {
        self.loads = crate::loader::Loads::deferred(Vec::new());
        self.loaded.clear();
        self.next = self.files.len();
    }

    /// Ends the load → its report. When the module the load installed under an id lists no
    /// permissions and is still installed, the process's grant for that id is forgotten. Files not
    /// installed or reported yet ([`FolderLoad::done`] false) are left out.
    pub fn finish(self) -> FolderReport {
        let FolderLoad { report, changes, .. } = self;
        for id in &report.loaded {
            // The serial is checked again while the grant is removed, so a module installed in its
            // place meanwhile keeps its grant.
            if let Some(&serial) = changes.own.get(id)
                && get(id).is_some_and(|p| p.serial() == serial && !p.manifest().asks_permissions())
            {
                revoke_if_installed(id, serial);
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cache keyed by the revision can't store the old list under the new revision: an install
    /// waiting for the set leaves the revision alone until it holds the set.
    #[test]
    fn an_install_changes_the_revision_only_while_it_holds_the_set() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let plugin = Plugin::load(include_bytes!("../tests/fixtures/desaturate.wasm"), Limits::default()).unwrap();
        let id = plugin.id().to_string();
        let (waits, install_waits) = std::sync::mpsc::channel();
        let ours = id.clone();
        *BEFORE_INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(move |p: &str| {
            if p == ours {
                let _ = waits.send(());
            }
        }));
        let held = PLUGINS.read().unwrap_or_else(|e| e.into_inner());
        let before = revision();
        let installing = std::thread::spawn(move || install(plugin));
        // The install has done everything it does before it waits for the set, which the test holds.
        let reached = install_waits.recv();
        let moved = revision() != before;
        drop(held);
        let installed = installing.join();
        *BEFORE_INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        installed.unwrap().unwrap();
        assert!(reached.is_ok(), "the install never asked for the set");
        assert!(!moved, "the revision changed while the install waited for the set");
        assert!(revision() > before && remove(&id));
    }

    /// A module of a filter `id` whose manifest lists `permissions` (a JSON list, or nothing), with
    /// `marker` in its code so that two markers give two SHA-256s.
    fn module(id: &str, permissions: Option<&str>, marker: i64) -> Vec<u8> {
        let listed = permissions.map(|p| format!(r#","permissions":{p}"#)).unwrap_or_default();
        let manifest = format!(r#"{{"id":"{id}","name":"T","kind":"filter"{listed}}}"#);
        wat::parse_str(crate::wat::module(&manifest, &format!("(i64.const {marker}) (drop) (i64.const 0)"), "", 1)).unwrap()
    }

    fn grant_for(id: &str, bytes: &[u8], permissions: Vec<crate::permissions::Permission>) -> Grant {
        Grant { id: id.into(), hash: crate::permissions::sha256_hex(bytes), permissions }
    }

    /// A module that lists permissions installs only once a recorded grant gives this very module
    /// all of them: refused, nothing changes, neither the revision nor the plug-in under its id.
    #[test]
    fn install_bytes_refuses_unapproved_manifest_permissions() {
        // The other tests that install or remove plug-ins hold it, so the revision moves only here.
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let id = "test.registry.unapproved";
        let plain = install_bytes(&module(id, None, 1)).unwrap();
        let before = revision();
        let e = install_bytes(&module(id, Some(r#"["doc.read"]"#), 2)).unwrap_err();
        assert_eq!(e, Error::NeedsApproval("doc.read".into()));
        assert_eq!(revision(), before, "nothing installed");
        assert_eq!(get(id).map(|p| p.hash().to_string()), Some(plain.hash().to_string()), "the plug-in before it stays");
        assert!(granted(id).is_none());
        assert!(remove(id));
    }

    /// `install` itself and a folder load whose `approve` accepts everything refuse the module too.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn install_and_folder_callback_cannot_bypass_approval() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let id = "test.registry.bypass";
        let plain = install_bytes(&module(id, Some("[]"), 3)).unwrap();
        let asking = module(id, Some(r#"["doc.read","doc.write"]"#), 4);
        let e = install(Plugin::load(&asking, Limits::default()).unwrap()).unwrap_err();
        assert_eq!(e, Error::NeedsApproval("doc.read, doc.write".into()));
        let dir = std::env::temp_dir().join(format!("vc-registry-bypass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("asking.wasm"), &asking).unwrap();
        let r = load_folder_where(&dir, |_| Ok(()), |_| true).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(r.loaded.is_empty(), "{r:?}");
        assert_eq!(r.failed.len(), 1);
        assert!(r.failed[0].1.starts_with("needs approval: it asks for doc.read, doc.write"), "{r:?}");
        assert_eq!(get(id).map(|p| p.hash().to_string()), Some(plain.hash().to_string()), "the plug-in before it stays");
        assert!(remove(id));
    }

    /// A recorded grant lets in only the module with its id and SHA-256 that lists nothing it
    /// doesn't grant; a module whose manifest lists no permissions, or an empty list, needs none.
    #[test]
    fn matching_grant_allows_only_exact_bytes() {
        use crate::permissions::Permission::{DocRead, DocWrite};
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let id = "test.registry.exact";
        let bytes = module(id, Some(r#"["doc.read","doc.write"]"#), 5);
        let plugin = Plugin::load(&bytes, Limits::default()).unwrap();
        assert_eq!(plugin.hash(), crate::permissions::sha256_hex(&bytes), "a grant's hash is the loaded module's");
        let refused = |g: Grant, why: &str| {
            let other = g.id.clone();
            revoke(id);
            grant(GrantToken::after_engine_approval(g));
            assert!(matches!(install_bytes(&bytes), Err(Error::NeedsApproval(_))), "{why}");
            assert!(get(id).is_none(), "{why}");
            revoke(&other);
        };
        refused(grant_for(id, &module(id, Some(r#"["doc.read","doc.write"]"#), 6), vec![DocRead, DocWrite]), "other bytes");
        refused(grant_for("test.registry.other", &bytes, vec![DocRead, DocWrite]), "another id");
        refused(Grant { id: id.into(), hash: "not a hash".into(), permissions: vec![DocRead, DocWrite] }, "a malformed hash");
        refused(
            Grant { id: id.into(), hash: crate::permissions::sha256_hex(&bytes).to_uppercase(), permissions: vec![DocRead, DocWrite] },
            "an upper-case hash",
        );
        refused(grant_for(id, &bytes, vec![DocRead]), "a missing permission");
        grant(GrantToken::from_engine_grants_file(grant_for(id, &bytes, vec![DocRead, DocWrite])));
        assert_eq!(install_bytes(&bytes).unwrap().hash(), plugin.hash());
        assert!(remove(id) && revoke(id));
        for listed in [None, Some("[]")] {
            assert!(install_bytes(&module(id, listed, 7)).is_ok(), "{listed:?}");
            assert!(granted(id).is_none() && remove(id), "{listed:?}");
        }
    }

    /// `install_checked` gives `check` the plug-in and grant held for the id. When `check` refuses,
    /// or the grant it would record doesn't approve the module, the plug-in, the grant and the
    /// revision stay as they were and `attach` doesn't run; otherwise it records the grant (or
    /// forgets it) and installs the module.
    #[test]
    fn install_checked_changes_nothing_when_refused() {
        use crate::permissions::Permission::DocRead;
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let id = "test.registry.checked";
        let old = module(id, Some(r#"["doc.read"]"#), 8);
        let new = module(id, Some(r#"["doc.read"]"#), 9);
        grant(GrantToken::after_engine_approval(grant_for(id, &old, vec![DocRead])));
        install_bytes(&old).unwrap();
        let load = |bytes: &[u8]| Plugin::load(bytes, Limits::default()).unwrap();
        let record = |bytes: &[u8]| GrantUpdate::Record(GrantToken::after_engine_approval(grant_for(id, bytes, vec![DocRead])));
        let unchanged = |why: &str| {
            assert_eq!(get(id).map(|p| p.hash().to_string()), Some(crate::permissions::sha256_hex(&old)), "{why}");
            assert_eq!(granted(id), Some(grant_for(id, &old, vec![DocRead])), "{why}");
        };
        let attached = std::cell::Cell::new(false);
        let before = revision();

        let seen = std::cell::RefCell::new(None);
        let e = install_checked(
            load(&new),
            record(&new),
            |p, g| {
                *seen.borrow_mut() = Some((p.map(|p| p.hash().to_string()), g.cloned()));
                Err("refused")
            },
            || attached.set(true),
            |_| "not approved",
        )
        .unwrap_err();
        assert_eq!(e, "refused");
        assert_eq!(seen.take(), Some((Some(crate::permissions::sha256_hex(&old)), Some(grant_for(id, &old, vec![DocRead])))));
        unchanged("check refused");

        let same = std::convert::identity::<Error>;
        let e = install_checked(load(&new), record(&old), |_, _| Ok(()), || attached.set(true), same).unwrap_err();
        assert_eq!(e, Error::NeedsApproval("doc.read".into()));
        unchanged("a grant for other bytes");
        let e = install_checked(load(&new), GrantUpdate::Revoke, |_, _| Ok(()), || attached.set(true), same).unwrap_err();
        assert_eq!(e, Error::NeedsApproval("doc.read".into()));
        unchanged("no grant");
        assert!(!attached.get() && revision() == before, "nothing ran or moved");

        let p = install_checked(load(&new), record(&new), |_, _| Ok(()), || attached.set(true), same).unwrap();
        assert!(attached.get() && revision() > before);
        assert_eq!(get(id).map(|p| p.hash().to_string()), Some(p.hash().to_string()));
        assert_eq!(granted(id), Some(grant_for(id, &new, vec![DocRead])));
        let plain = module(id, Some("[]"), 10);
        install_checked(load(&plain), GrantUpdate::Revoke, |_, _| Ok(()), || {}, same).unwrap();
        assert!(granted(id).is_none(), "a module without permissions forgets the grant");
        assert!(remove(id));
    }

    /// An empty folder for one test, with anything an earlier run left there removed.
    #[cfg(not(target_arch = "wasm32"))]
    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vc-registry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Makes a named pipe at `path`.
    #[cfg(unix)]
    fn make_pipe(path: &std::path::Path) {
        assert!(std::process::Command::new("mkfifo").arg(path).status().unwrap().success(), "mkfifo {}", path.display());
    }

    /// Limits that let a test go past the module size with a few bytes.
    #[cfg(not(target_arch = "wasm32"))]
    fn small_limits() -> Limits {
        Limits { max_module_bytes: 16, ..Limits::default() }
    }

    /// A module file is opened so that a named pipe in its place doesn't make the open wait for a
    /// writer. The test holds the pipe open for reading and writing itself, so the open returns
    /// either way.
    #[cfg(unix)]
    #[test]
    fn a_module_file_is_opened_without_waiting_for_a_writer() {
        let dir = test_dir("open");
        let pipe = dir.join("x.wasm");
        make_pipe(&pipe);
        let held = std::fs::OpenOptions::new().read(true).write(true).open(&pipe).unwrap();
        let opened = open_module(&pipe).unwrap();
        let flags = rustix::fs::fcntl_getfl(&opened).unwrap();
        drop((opened, held));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(flags.contains(rustix::fs::OFlags::NONBLOCK), "the open would wait for a writer: {flags:?}");
    }

    /// A `.wasm` that a named pipe replaces after it was checked is refused as not a file, even
    /// when the pipe holds more bytes than the module limit allows.
    #[cfg(unix)]
    #[test]
    fn a_file_replaced_by_a_pipe_is_refused() {
        use std::io::Write;
        let dir = test_dir("pipe");
        let path = dir.join("x.wasm");
        let pipe = dir.join("pipe");
        std::fs::write(&path, b"\0asm").unwrap();
        make_pipe(&pipe);
        let mut held = std::fs::OpenOptions::new().read(true).write(true).open(&pipe).unwrap();
        held.write_all(&[0; 32]).unwrap();
        let read = read_module(&path, &small_limits(), || std::fs::rename(&pipe, &path).unwrap());
        drop(held);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(&read, Err(Error::Io(e)) if e.ends_with("is not a file")), "{read:?}");
    }

    /// A `.wasm` that grows past the module limit after it was checked is refused by its size.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_file_that_grows_after_the_check_is_refused() {
        let dir = test_dir("grow");
        let path = dir.join("x.wasm");
        std::fs::write(&path, b"\0asm").unwrap();
        let read = read_module(&path, &small_limits(), || std::fs::write(&path, [0; 32]).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(&read, Err(Error::Module(e)) if e.contains("limit 16")), "{read:?}");
    }

    /// A JavaScript plug-in's manifest file and its script, in `dir`.
    #[cfg(not(target_arch = "wasm32"))]
    fn write_js(dir: &std::path::Path, id: &str, manifest: serde_json::Value, script: Option<&str>) -> std::path::PathBuf {
        let mut m = serde_json::json!({"id": id, "name": "JS", "kind": "filter", "runtime": "js", "script": "f.js"});
        if let (Some(m), serde_json::Value::Object(extra)) = (m.as_object_mut(), manifest) {
            m.extend(extra);
        }
        let path = dir.join(format!("{id}.vcplugin"));
        std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
        if let Some(script) = script {
            std::fs::write(dir.join("f.js"), script).unwrap();
        }
        path
    }

    /// A `.vcplugin` in a folder installs with its script, and its SHA-256 is the one an install of
    /// the same package by bytes gets.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_installs_a_javascript_plugin() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("js");
        let id = "test.registry.js";
        write_js(&dir, id, serde_json::json!({}), Some("function run(i) { return i; }"));
        let r = load_folder_where(&dir, |_| Ok(()), |_| false).unwrap();
        let bytes = read_js_package(&dir.join(format!("{id}.vcplugin")), &|_| Ok(())).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(r.loaded, vec![id.to_string()], "{r:?}");
        let p = get(id).unwrap();
        assert_eq!((p.script(), p.hash()), (Some("f.js"), crate::permissions::sha256_hex(&bytes).as_str()));
        assert!(remove(id));
    }

    /// A manifest with `"runtime": "js"` and no `script` is refused, and the error names the key;
    /// so is one that names a script `allow` refuses or that isn't there.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_javascript_manifest_needs_its_script() {
        let dir = test_dir("js-script");
        let no_key = write_js(&dir, "a", serde_json::json!({"script": null}), None);
        let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&no_key).unwrap()).unwrap();
        m.as_object_mut().unwrap().remove("script");
        std::fs::write(&no_key, serde_json::to_vec(&m).unwrap()).unwrap();
        let missing = write_js(&dir, "b", serde_json::json!({}), None);
        let inline = write_js(&dir, "c", serde_json::json!({"source": "function run(i) { return i; }"}), None);
        let not_js = write_js(&dir, "d", serde_json::json!({"runtime": "wasm"}), None);
        let refused = read_js_package(&missing, &|p| Err(format!("{} is outside", p.display())));
        let results = [
            read_js_package(&no_key, &|_| Ok(())),
            read_js_package(&missing, &|_| Ok(())),
            read_js_package(&inline, &|_| Ok(())),
            read_js_package(&not_js, &|_| Ok(())),
        ];
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(&results[0], Err(Error::Manifest(e)) if e.starts_with("`runtime` \"js\" needs `script`")), "{:?}", results[0]);
        assert!(matches!(&results[1], Err(Error::Io(e)) if e.contains("f.js")), "{:?}", results[1]);
        assert!(matches!(&results[2], Err(Error::Manifest(e)) if e.contains("`source`")), "{:?}", results[2]);
        assert!(matches!(&results[3], Err(Error::Manifest(e)) if e == crate::js::NOT_JS), "{:?}", results[3]);
        assert!(matches!(&refused, Err(Error::Io(e)) if e.ends_with("is outside")), "{refused:?}");
    }

    /// A script larger than the limit is refused by its size before it is opened.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_oversized_script_is_refused_before_it_is_read() {
        let dir = test_dir("js-big");
        let path = write_js(&dir, "big", serde_json::json!({}), Some(""));
        std::fs::File::options().write(true).open(dir.join("f.js")).unwrap().set_len(crate::js::MAX_SCRIPT_BYTES as u64 + 1).unwrap();
        let opened = std::cell::Cell::new(false);
        let read = read_js_package_with(&path, &|_| Ok(()), || opened.set(true));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(&read, Err(Error::Module(e)) if e.contains(&format!("limit {}", crate::js::MAX_SCRIPT_BYTES))), "{read:?}");
        assert!(!opened.get(), "the script was opened");
    }

    /// A manifest file's parameters keep their declaration order through the package it is read
    /// into, which is its own text with `source` added.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_javascript_package_keeps_the_manifest_s_order() {
        let dir = test_dir("js-order");
        let num = r#"{"type": "number", "min": 0, "max": 10, "default": 1}"#;
        let text = format!(
            "{{\"id\": \"order\", \"name\": \"JS\", \"kind\": \"filter\", \"runtime\": \"js\", \"script\": \"f.js\", \"params\": {{\"z\": {num}, \"a\": {num}}}}}\n"
        );
        let path = dir.join("order.vcplugin");
        std::fs::write(&path, &text).unwrap();
        std::fs::write(dir.join("f.js"), "function run(i) { return i; }").unwrap();
        let bytes = read_js_package(&path, &|_| Ok(()));
        std::fs::remove_dir_all(&dir).unwrap();
        let bytes = bytes.unwrap();
        let expected = format!("{},\"source\":\"function run(i) {{ return i; }}\"}}", text.trim_end().strip_suffix('}').unwrap());
        assert_eq!(String::from_utf8_lossy(&bytes), expected);
        let (manifest, _) = crate::js::read_bundle(&bytes).unwrap();
        assert_eq!(manifest.params.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["z", "a"]);
    }

    /// A folder load reports a subfolder, a named pipe and a link to a named pipe with `.wasm`
    /// names as not files, and goes on.
    #[cfg(unix)]
    #[test]
    fn a_folder_reports_entries_that_are_not_files() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("special");
        std::fs::create_dir(dir.join("a.wasm")).unwrap();
        make_pipe(&dir.join("b.wasm"));
        std::os::unix::fs::symlink(dir.join("b.wasm"), dir.join("c.wasm")).unwrap();
        let r = load_folder_where(&dir, |_| Ok(()), |_| false);
        std::fs::remove_dir_all(&dir).unwrap();
        let r = r.unwrap();
        let failed: Vec<(String, bool)> = r.failed.iter().map(|(f, e)| (f.clone(), e.ends_with("is not a file"))).collect();
        let expected: Vec<(String, bool)> = ["a", "b", "c"].iter().map(|n| (dir.join(format!("{n}.wasm")).display().to_string(), true)).collect();
        assert_eq!(failed, expected, "{r:?}");
        assert!(r.loaded.is_empty() && r.left_out == 0, "{r:?}");
    }

    /// A folder with more `.wasm` files than a load reads tries the first ones by file name and
    /// counts the files it left out.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_over_the_file_limit_reports_the_files_left_out() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("limit");
        // Written last first, so the listing's order is not the file-name order.
        for i in (0..MAX_FOLDER_FILES + 2).rev() {
            std::fs::write(dir.join(format!("{i:05}.wasm")), b"").unwrap();
        }
        let r = load_folder_where(&dir, |_| Ok(()), |_| false).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(r.loaded.is_empty(), "{r:?}");
        assert_eq!(r.left_out, 2, "{r:?}");
        let tried: Vec<String> = r.failed.iter().map(|(f, _)| f.clone()).collect();
        let first: Vec<String> = (0..MAX_FOLDER_FILES).map(|i| dir.join(format!("{i:05}.wasm")).display().to_string()).collect();
        assert_eq!(tried, first, "the first files by name, in that order");
    }

    /// Clears [`BEFORE_FOLDER_FILE`] when a test ends, failed or not.
    #[cfg(not(target_arch = "wasm32"))]
    struct Hooked;

    #[cfg(not(target_arch = "wasm32"))]
    impl Hooked {
        fn set(hook: impl Fn(&std::path::Path) + Send + Sync + 'static) -> Hooked {
            *BEFORE_FOLDER_FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(hook));
            Hooked
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for Hooked {
        fn drop(&mut self) {
            *BEFORE_FOLDER_FILE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// The plug-ins of a folder install in file-name order, and take their revisions in that order,
    /// when the first file's load finishes after the others.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_installs_in_file_name_order_whatever_order_the_loads_finish_in() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("order");
        let ids = ["test.registry.order.a", "test.registry.order.b", "test.registry.order.c"];
        for (n, id) in ["a", "b", "c"].iter().zip(ids) {
            std::fs::write(dir.join(format!("{n}.wasm")), module(id, None, 11)).unwrap();
        }
        let first = dir.join("a.wasm");
        let (go, a_waits) = std::sync::mpsc::channel::<()>();
        let a_waits = Mutex::new(a_waits);
        let (reached, a_reached) = std::sync::mpsc::channel::<()>();
        let held = first.clone();
        let _hook = Hooked::set(move |p| {
            if p == held {
                let _ = reached.send(());
                let _ = a_waits.lock().unwrap_or_else(|e| e.into_inner()).recv();
            }
        });
        // Two workers, so the other files load while the first one waits.
        let mut load = FolderLoad::list_on(&dir, |_| Ok(()), Some(2)).unwrap();
        load.start();
        // Bound after `load`, so a failing assertion lets the first file go before dropping `load`
        // waits for it.
        let go = go;
        a_reached.recv().unwrap();
        // The other two files load on the other worker while the first one waits: their outcomes
        // come in, and the launch marker no longer names them.
        let others = [load.loads.record_next(), load.loads.record_next()];
        let all = ["a", "b", "c"].map(|n| dir.join(format!("{n}.wasm")));
        let named = |p: &std::path::Path| {
            let item = crate::safe_mode::Item::file(p.display().to_string());
            crate::safe_mode::still_loading().iter().any(|i| i.same(&item))
        };
        assert!(others == [Some(1), Some(2)] && !named(&all[1]) && !named(&all[2]), "the other files didn't load meanwhile: {others:?}");
        let early = load.install_ready(|_| false);
        assert!(!load.done(), "the load ended while the first file waited");
        assert!(early.is_empty() && ids.iter().all(|id| get(id).is_none()), "nothing installs before the first file has loaded");
        go.send(()).unwrap();
        while !load.done() {
            load.wait();
            load.install_ready(|_| false);
        }
        let r = load.finish();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(r.loaded, ids.to_vec(), "{r:?}");
        let serials: Vec<u64> = ids.iter().map(|id| get(id).unwrap().serial()).collect();
        assert!(serials.windows(2).all(|w| w[0] < w[1]), "installed in file-name order: {serials:?}");
        for id in ids {
            assert!(remove(id));
        }
    }

    /// A folder module that loaded before something else installed or removed the same id, while
    /// an earlier file still loaded, doesn't undo that change: it is reported as skipped, and the
    /// plug-in installed meanwhile stays, as does the removal.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_module_doesnt_undo_a_change_made_while_it_waited() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("superseded");
        let ids = ["test.registry.superseded.a", "test.registry.superseded.b", "test.registry.superseded.c"];
        // b's file asks for a permission: asking whether it may install could record a grant.
        for (n, id) in ["a", "b", "c"].iter().zip(ids) {
            let asks = (*n == "b").then_some(r#"["doc.read"]"#);
            std::fs::write(dir.join(format!("{n}.wasm")), module(id, asks, 13)).unwrap();
        }
        install_bytes(&module(ids[2], None, 14)).unwrap();
        let first = dir.join("a.wasm");
        let (go, a_waits) = std::sync::mpsc::channel::<()>();
        let a_waits = Mutex::new(a_waits);
        let (reached, a_reached) = std::sync::mpsc::channel::<()>();
        let held = first.clone();
        let _hook = Hooked::set(move |p| {
            if p == held {
                let _ = reached.send(());
                let _ = a_waits.lock().unwrap_or_else(|e| e.into_inner()).recv();
            }
        });
        let mut load = FolderLoad::list_on(&dir, |_| Ok(()), Some(2)).unwrap();
        load.start();
        a_reached.recv().unwrap();
        // Elsewhere, while the first file still loads: `b` is installed and `c` removed.
        let theirs = install_bytes(&module(ids[1], None, 15)).unwrap().serial();
        assert!(remove(ids[2]));
        go.send(()).unwrap();
        let asked = Mutex::new(Vec::new());
        while !load.done() {
            load.wait();
            load.install_ready(|p| {
                asked.lock().unwrap_or_else(|e| e.into_inner()).push(p.id().to_string());
                false
            });
        }
        let r = load.finish();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(r.loaded, vec![ids[0]], "{r:?}");
        assert!(r.failed.iter().all(|(_, why)| why == SUPERSEDED) && r.failed.len() == 2, "{r:?}");
        assert!(asked.lock().unwrap_or_else(|e| e.into_inner()).is_empty(), "no grant is asked for a superseded module");
        assert_eq!(get(ids[1]).map(|p| p.serial()), Some(theirs), "the plug-in installed meanwhile stays");
        assert!(get(ids[2]).is_none(), "the removal stays");
        for id in &ids[..2] {
            assert!(remove(id));
        }
    }

    /// A plug-in installed elsewhere after the folder was listed and before its loads started
    /// stays: the folder's module with its id is skipped. One a load of the same session installed
    /// before it ([`LoadChanges::adopt`]) is replaced.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_module_doesnt_undo_a_change_made_before_its_loads_started() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("superseded-listed");
        let ids = ["test.registry.listed.a", "test.registry.listed.b"];
        for (n, id) in ["a", "b"].iter().zip(ids) {
            std::fs::write(dir.join(format!("{n}.wasm")), module(id, None, 13)).unwrap();
        }
        let mut load = FolderLoad::list_on(&dir, |_| Ok(()), Some(2)).unwrap();
        // Before the loads start: another session installs `a`, and an earlier load of this
        // session installs `b`.
        let theirs = install_bytes(&module(ids[0], None, 14)).unwrap().serial();
        let mut earlier = LoadChanges::default();
        earlier.start();
        earlier.installed(&install_bytes(&module(ids[1], None, 15)).unwrap());
        load.changes_mut().adopt(&earlier);
        load.start();
        while !load.done() {
            load.wait();
            load.install_ready(|_| false);
        }
        let r = load.finish();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(r.loaded, vec![ids[1]], "{r:?}");
        assert!(r.failed.iter().all(|(_, why)| why == SUPERSEDED) && r.failed.len() == 1, "{r:?}");
        assert_eq!(get(ids[0]).map(|p| p.serial()), Some(theirs), "the plug-in installed elsewhere stays");
        for id in ids {
            assert!(remove(id));
        }
    }

    /// A folder whose second module panics while it loads reports it as failed and installs the
    /// others.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_module_that_panics_while_loading_fails_alone() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("panics");
        let ids = ["test.registry.panics.a", "test.registry.panics.b", "test.registry.panics.c"];
        for (n, id) in ["a", "b", "c"].iter().zip(ids) {
            std::fs::write(dir.join(format!("{n}.wasm")), module(id, None, 12)).unwrap();
        }
        let broken = dir.join("b.wasm");
        let hooked = broken.clone();
        let hook = Hooked::set(move |p| {
            if p == hooked {
                panic!("a defect while loading");
            }
        });
        let r = load_folder_where(&dir, |_| Ok(()), |_| false);
        drop(hook);
        std::fs::remove_dir_all(&dir).unwrap();
        let item = crate::safe_mode::Item::file(broken.display().to_string());
        let marked = crate::safe_mode::still_loading().iter().any(|i| i.same(&item));
        let r = r.unwrap();
        assert_eq!(r.loaded, vec![ids[0], ids[2]], "{r:?}");
        assert_eq!(r.failed.len(), 1, "{r:?}");
        assert!(r.failed[0].0 == broken.display().to_string() && r.failed[0].1.contains("panicked"), "{r:?}");
        assert!(!marked, "the launch marker stops naming the module whose load failed");
        for id in [ids[0], ids[2]] {
            assert!(remove(id));
        }
    }

    /// A folder module isn't installed over a plug-in installed elsewhere with its id while the load
    /// asked whether it may install: that plug-in stays, and the module is reported as skipped.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_module_doesnt_replace_one_installed_while_it_was_approved() {
        use crate::permissions::Permission::DocRead;
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("approving");
        let id = "test.registry.approving";
        let asking = module(id, Some(r#"["doc.read"]"#), 17);
        std::fs::write(dir.join("a.wasm"), &asking).unwrap();
        grant(GrantToken::after_engine_approval(grant_for(id, &asking, vec![DocRead])));
        let theirs = std::cell::Cell::new(None);
        let r = load_folder_where(
            &dir,
            |_| Ok(()),
            |_| {
                theirs.set(Some(install_bytes(&module(id, None, 18)).unwrap().serial()));
                true
            },
        );
        std::fs::remove_dir_all(&dir).unwrap();
        let now = get(id).map(|p| p.serial());
        assert!(remove(id));
        revoke(id);
        let r = r.unwrap();
        assert_eq!(now, theirs.get(), "the plug-in installed meanwhile was replaced: {r:?}");
        assert!(r.loaded.is_empty() && r.failed.len() == 1 && r.failed[0].1 == SUPERSEDED, "{r:?}");
    }

    /// A grant is forgotten only while the plug-in that took the given revision is installed under
    /// its id: a plug-in installed in its place keeps the grant recorded for it.
    #[test]
    fn a_grant_is_revoked_only_for_the_plugin_still_installed() {
        use crate::permissions::Permission::DocRead;
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let id = "test.registry.revokeinstalled";
        let first = install_bytes(&module(id, None, 19)).unwrap().serial();
        let asking = module(id, Some(r#"["doc.read"]"#), 20);
        grant(GrantToken::after_engine_approval(grant_for(id, &asking, vec![DocRead])));
        let second = install_bytes(&asking).unwrap().serial();
        assert!(!revoke_if_installed(id, first), "the grant of the plug-in installed in its place was forgotten");
        assert!(granted(id).is_some());
        assert!(revoke_if_installed(id, second));
        assert!(granted(id).is_none() && remove(id));
    }

    /// A folder file that no loader thread was left to load fails with that reason, and the report
    /// says a file found no thread ([`FolderReport::no_thread`]).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_file_no_thread_was_left_to_load_is_reported() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = test_dir("nothread");
        let id = "test.registry.nothread";
        std::fs::write(dir.join("a.wasm"), module(id, None, 16)).unwrap();
        let mut load = FolderLoad::list_on(&dir, |_| Ok(()), Some(1)).unwrap();
        load.loads.refuse_threads = true;
        load.start();
        while !load.done() {
            load.wait();
            load.install_ready(|_| false);
        }
        let r = load.finish();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(get(id).is_none(), "{r:?}");
        assert_eq!(r.failed.len(), 1, "{r:?}");
        assert!(r.no_thread && r.failed[0].1.contains(crate::loader::NO_THREAD), "{r:?}");
    }
}
