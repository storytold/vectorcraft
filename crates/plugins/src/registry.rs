//! The process-wide set of installed plug-ins.
//!
//! Plug-ins are installed per process, not per document: live effects are drawn from every open
//! document (and from scratch sessions such as previews), and they must find the same plug-ins.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

#[cfg(not(target_arch = "wasm32"))]
use crate::Error;
use crate::permissions::Grant;
use crate::{Limits, Plugin, Result};

static PLUGINS: RwLock<BTreeMap<String, Arc<Plugin>>> = RwLock::new(BTreeMap::new());
static REVISION: AtomicU64 = AtomicU64::new(0);
/// What the user approved, by plug-in id.
static GRANTS: RwLock<BTreeMap<String, Grant>> = RwLock::new(BTreeMap::new());

/// Records what the user approved for a plug-in, replacing an earlier grant for its id.
pub fn grant(g: Grant) {
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
/// already grant its id every permission it lists? A module with other bytes but no new permission
/// needs no new approval.
pub fn approved(plugin: &Plugin) -> bool {
    let m = plugin.manifest();
    !m.asks_permissions() || granted(&m.id).is_some_and(|g| g.covers(&m.permissions()))
}

/// Installs `plugin`, replacing one with the same id.
pub fn install(mut plugin: Plugin) -> Arc<Plugin> {
    plugin.set_serial(REVISION.fetch_add(1, Ordering::Relaxed) + 1);
    let p = Arc::new(plugin);
    PLUGINS.write().unwrap_or_else(|e| e.into_inner()).insert(p.id().to_string(), p.clone());
    p
}

/// Loads `bytes` with the default limits and installs the plug-in.
pub fn install_bytes(bytes: &[u8]) -> Result<Arc<Plugin>> {
    Ok(install(Plugin::load(bytes, Limits::default())?))
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
    let gone = PLUGINS.write().unwrap_or_else(|e| e.into_inner()).remove(id).is_some();
    if gone {
        REVISION.fetch_add(1, Ordering::Relaxed);
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

/// Reads a `.wasm` file (refusing anything that isn't a regular file or exceeds the module limit)
/// and loads it.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(path: &std::path::Path, limits: Limits) -> Result<Plugin> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::Io(format!("{} is not a file", path.display())));
    }
    if meta.len() > limits.max_module_bytes as u64 {
        return Err(Error::Module(format!("{} is {} bytes (limit {})", path.display(), meta.len(), limits.max_module_bytes)));
    }
    let f = std::fs::File::open(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let mut bytes = Vec::new();
    f.take(limits.max_module_bytes as u64 + 1).read_to_end(&mut bytes).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    Ok(Plugin::load(&bytes, limits)?.with_source(path.display().to_string()))
}

/// What [`load_folder`] reports for a plug-in the user disabled.
pub const DISABLED: &str = "disabled: turn it on again in Help › Safe Mode, for the next launch";

/// What [`load_folder`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderReport {
    /// Ids of the plug-ins installed.
    pub loaded: Vec<String>,
    /// `(file, error)` for each `.wasm` that failed to load.
    pub failed: Vec<(String, String)>,
    /// The disabled items saved for the next launch changed: a plug-in disabled by its id alone was
    /// found in a file, which is now disabled too ([`crate::safe_mode::disable_file_of`]). The app
    /// saves the list again.
    pub disabled_changed: bool,
}

/// Installs every `*.wasm` directly in `dir` (not recursive), in file-name order. A bad module is
/// reported and skipped, the others still load; so is a heavy one ([`crate::Budget::Heavy`]),
/// which installs only once the user agrees, and one that lists permissions the user has not
/// granted its id ([`approved`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_folder(dir: &std::path::Path) -> Result<FolderReport> {
    load_folder_where(dir, |_| Ok(()), approved)
}

/// [`load_folder`], loading only the files `allow` accepts, and of the plug-ins that list
/// permissions only those `approve` accepts; the others are reported as failed with the reason. A plug-in the user disabled ([`crate::safe_mode::skipped`]) is reported and
/// skipped, by its file before it loads or by its id after (its file is then disabled too, so the
/// next load skips it before it runs); the launch marker names each file while it loads
/// ([`crate::safe_mode::loading`]). In Safe Mode nothing loads ([`Error::SafeMode`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_folder_where(
    dir: &std::path::Path,
    allow: impl Fn(&std::path::Path) -> std::result::Result<(), String>,
    approve: impl Fn(&Plugin) -> bool,
) -> Result<FolderReport> {
    use crate::safe_mode::{self, Item};
    safe_mode::check()?;
    let rd = std::fs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
    let mut files: Vec<std::path::PathBuf> =
        rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wasm"))).collect();
    files.sort();
    files.truncate(1000);
    let mut report = FolderReport::default();
    for f in files {
        if let Err(why) = allow(&f) {
            report.failed.push((f.display().to_string(), why));
            continue;
        }
        let item = Item::file(f.display().to_string());
        if safe_mode::skipped(&item) {
            report.failed.push((f.display().to_string(), DISABLED.into()));
            continue;
        }
        let token = safe_mode::loading(&item);
        let loaded = load_file(&f, Limits::default());
        safe_mode::loaded(token);
        match loaded {
            Ok(p) if safe_mode::skipped(&Item::of(&p)) => {
                report.disabled_changed |= safe_mode::disable_file_of(&Item::of(&p));
                report.failed.push((f.display().to_string(), DISABLED.into()));
            }
            // A heavy plug-in installs once the user agrees, which loading a folder doesn't ask.
            Ok(p) if p.manifest().budget == crate::Budget::Heavy => {
                report.failed.push((f.display().to_string(), "asks for a heavy budget, which the user allows when installing it on its own".into()));
            }
            Ok(p) if p.manifest().asks_permissions() && !approve(&p) => {
                let asked = crate::permissions::names(&p.manifest().permissions()).join(", ");
                report.failed.push((
                    f.display().to_string(),
                    format!("needs approval: it asks for {asked}, which the user grants when installing it on its own"),
                ));
            }
            Ok(p) => report.loaded.push(install(p).id().to_string()),
            Err(e) => report.failed.push((f.display().to_string(), e.to_string())),
        }
    }
    Ok(report)
}
