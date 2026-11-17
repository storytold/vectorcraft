//! Shared `command` plug-ins and installing from bundles, catalogs and GitHub (`craft-plugin`,
//! RFC-0041).
//!
//! VectorCraft's own plug-in kinds (object filters, live effects, exporters, importers: the
//! `vectorcraft-plugins` crate, [`super::plugin`]) keep their ABI, manifest, permissions and store.
//! This module adds what the Craft apps share:
//!
//! - **Command plug-ins** (`kind: "command"`): a module that acts on the document only through
//!   VectorCraft's commands, the ones the menus, the CLI, the control channel and MCP run. A run is
//!   one undo step named after the plug-in ([`super::plugin_call::transaction`]), and a run that
//!   fails (a trap, an exhausted budget, a refusal it passes on, a non-zero status) leaves every
//!   document, its selection and its history as they were. Each call is checked before it runs
//!   ([`refusal`]: [`craft_plugin::ALWAYS_DENIED`], VectorCraft's own refused commands as a
//!   [`Policy`], the manifest's allowlist, and the same rules filters' `vc_call` follows: the
//!   reviewed commands only, never a file, a folder or a plug-in) and runs with no file readable
//!   or writable. After it ran, a call that changed the document of a plug-in with `read` access,
//!   or changed which documents are open, is put back ([`super::plugin_call::Checkpoint`]) and
//!   refused.
//! - **The shared store**: `.craftplugin` bundles of command plug-ins are kept with
//!   [`craft_plugin::store::Store`] in the same `Plug-ins` folder as VectorCraft's own modules
//!   (one folder per plug-in named by a hash of its id, beside the flat `*.wasm` files, which keep
//!   being read as before). A command plug-in's settings are kept in its folder there.
//! - **Installing** from a bundle file, a catalog (`index.json`) or a GitHub repository's latest
//!   release, with the shared checks (GitHub's hosts only, sizes, SHA-256, the manifest's `apps`).
//!   A bundle of one of VectorCraft's own kinds is checked here and then installed as a `.wasm`
//!   module would be (`plugin.install`'s own rules: permissions to grant, heavy budget, replace).
//!
//! The `plugin.*` commands that use it are in [`super::plugin`]. See `docs/plugins.md`.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

pub use craft_plugin;
use craft_plugin::fetch::{self, Fetch};
use craft_plugin::{API_VERSION, AppInfo, Instance, KIND_COMMAND, Limits, Manifest, Policy, RunOutput, UiChange, bundle, catalog, github};
use serde_json::{Map, Value, json};

use super::plugin_call::{Calls, Checkpoint};
use super::{EngineError, Result, Session, bad};

/// VectorCraft as shared manifests name it, for command plug-ins: every version of the shared
/// command ABI, and only manifests that name `vectorcraft` in `apps`.
pub const APP: AppInfo<'static> =
    AppInfo { id: "vectorcraft", version: env!("CARGO_PKG_VERSION"), api: 1..=API_VERSION, kinds: &[KIND_COMMAND], accept_legacy: false };

/// VectorCraft's own kinds as a bundle's `craft-plugin.json` names them, with VectorCraft's own
/// plug-in API levels: a bundle of one of them carries a module `plugin.install` installs as it
/// installs a `.wasm` file.
pub const OWN_KINDS: AppInfo<'static> = AppInfo {
    id: "vectorcraft",
    version: env!("CARGO_PKG_VERSION"),
    api: 1..=vectorcraft_plugins::permissions::API_LEVEL,
    kinds: &["filter", "effect", "exporter", "importer"],
    accept_legacy: false,
};

/// What one plug-in may keep in its settings (keys and JSON values, in bytes), and its longest key.
pub const MAX_SETTINGS_BYTES: usize = 64 * 1024;
pub const MAX_SETTING_KEY: usize = 128;
/// Catalogs a person may add.
pub const MAX_CATALOGS: usize = 32;
/// Messages a run may show (`notify`), and their length in characters.
const MAX_NOTES: usize = 20;
const MAX_NOTE_CHARS: usize = 1000;
/// The file in a command plug-in's store folder that holds its settings.
#[cfg(not(target_arch = "wasm32"))]
const SETTINGS_FILE: &str = "settings.json";
/// The file in the `Plug-ins` folder that lists the catalogs added (`plugin.catalog.add`).
#[cfg(not(target_arch = "wasm32"))]
const CATALOGS_FILE: &str = "catalogs.json";
/// The largest catalogs file read.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CATALOGS_FILE: u64 = 64 * 1024;

/// The rules every command plug-in's calls follow on top of [`craft_plugin::ALWAYS_DENIED`]:
/// VectorCraft's refused command prefixes ([`super::plugin_call::DENIED_PREFIXES`], the list
/// filters' `vc_call` follows), each as a `prefix*` pattern.
pub fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| Policy { denied: super::plugin_call::DENIED_PREFIXES.iter().map(|p| format!("{p}*")).collect(), exceptions: Vec::new() })
}

/// Why plug-in `m` may not call `command` with `params`, before it runs: [`craft_plugin::check_call`]
/// with [`policy`], a command that doesn't exist, and VectorCraft's own rules for plug-in calls (a
/// command no plug-in may call, whatever its manifest lists, and params that name a file, a
/// folder or a plug-in).
pub fn refusal(m: &Manifest, command: &str, params: &Value) -> Option<String> {
    if let Err(why) = craft_plugin::check_call(m, policy(), command) {
        return Some(why);
    }
    let Some(spec) = super::find_command(command) else {
        return Some(super::params::unknown_command(command).to_string());
    };
    if super::plugin_call::denied(spec).is_some() || super::plugin_call::names_plugin(params) || super::plugin_call::names_file(command, params) {
        return Some(format!("`{command}` isn't available to plugins"));
    }
    None
}

/// An installed command plug-in.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub manifest: Manifest,
    /// The SHA-256 of its bundle (lower-case hex).
    pub sha256: String,
    /// Where it came from: `{type: file, path}`, `{type: catalog, catalog, package}` or
    /// `{type: github, repo, tag, asset}` (craft-plugin's `store::Source`).
    pub source: Value,
    /// Kept in the user's `Plug-ins` folder; otherwise for this session only.
    pub stored: bool,
    wasm: Arc<Vec<u8>>,
}

impl Plugin {
    /// What `plugin.list` says about it.
    pub fn describe(&self) -> Value {
        let mut v = serde_json::to_value(&self.manifest).unwrap_or_else(|_| json!({"id": self.manifest.id}));
        v["kind"] = json!(self.manifest.kind);
        v["sha256"] = json!(self.sha256);
        v["hash"] = json!(self.sha256);
        v["source"] = self.source.clone();
        v["stored"] = json!(self.stored);
        v["shared"] = json!(true);
        v
    }
}

/// A bundle obtained for installing: its bytes, where it came from and, for a repository, whether
/// a catalog in use lists it.
pub struct Candidate {
    pub bytes: Vec<u8>,
    pub source: Value,
    /// `Some(true)` from a catalog; for a repository, whether a catalog in use lists it.
    pub in_catalog: Option<bool>,
    /// The `plugin.install` parameters that name it ([`request_key`]).
    pub request: Value,
}

/// A command plug-in as the menus list it: its id, its name and its commands (id, label, whether
/// it has parameters to ask for).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub id: String,
    pub name: String,
    pub commands: Vec<(String, String, bool)>,
}

/// The command plug-ins the menus list: the ones the session that changed its plug-ins last has
/// (menus are built without a session, as for VectorCraft's own plug-ins' process-wide registry).
static MENU: RwLock<Vec<MenuEntry>> = RwLock::new(Vec::new());

/// The command plug-ins for the menus ([`MenuEntry`]).
pub fn menu_entries() -> Vec<MenuEntry> {
    MENU.read().map(|m| m.clone()).unwrap_or_else(|e| e.into_inner().clone())
}

/// Show `plugins` in the menus.
fn publish(plugins: &[Plugin]) {
    let entries = plugins
        .iter()
        .map(|p| MenuEntry {
            id: p.manifest.id.clone(),
            name: p.manifest.name.clone(),
            commands: p.manifest.commands.iter().map(|c| (c.id.clone(), c.label.clone(), c.params.iter().any(|x| x.is_field()))).collect(),
        })
        .collect();
    *MENU.write().unwrap_or_else(|e| e.into_inner()) = entries;
}

/// The session's command plug-ins.
#[derive(Default)]
pub struct Runtime {
    /// The user's `Plug-ins` folder ([`Session::attach_plugin_store`]); without one, plug-ins are
    /// installed for the session only.
    #[cfg(not(target_arch = "wasm32"))]
    store: Option<craft_plugin::store::Store>,
    plugins: Vec<Plugin>,
    /// Plug-in folders that couldn't be read, or plug-ins this version can't run.
    problems: Vec<String>,
    /// The catalogs added, by URL.
    catalogs: Vec<String>,
    /// Each plug-in's settings, by its id.
    settings: BTreeMap<String, Map<String, Value>>,
    /// HTTPS for catalogs and GitHub releases ([`Session::set_plugin_fetch`]); without one, only
    /// files and `file://` catalogs install.
    fetch: Option<Box<dyn Fetch + Send>>,
    /// A bundle shown for review (`plugin.install {review: true}`) or handed in
    /// ([`Session::hand_in_plugin_bundle`]), kept so the install that follows doesn't download it
    /// again.
    pending: Option<Candidate>,
}

impl Runtime {
    /// The installed command plug-ins.
    pub fn list(&self) -> &[Plugin] {
        &self.plugins
    }

    /// Why plug-in folders couldn't be used.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Command plug-in `id` (ignoring case, as the store does).
    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.manifest.id.eq_ignore_ascii_case(id))
    }

    /// The catalogs added.
    pub fn catalogs(&self) -> &[String] {
        &self.catalogs
    }

    /// Whether plug-ins are kept in the user's folder (else for this session only).
    pub fn has_store(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.store.is_some();
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// Keep command plug-ins in the `Plug-ins` folder `dir` and read the ones installed there.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_store(&mut self, dir: &std::path::Path) {
        self.store = Some(craft_plugin::store::Store::new(dir));
        self.catalogs = read_catalogs(dir);
        self.reload();
    }

    /// Read the store again (the session's own plug-ins stay).
    #[cfg(not(target_arch = "wasm32"))]
    fn reload(&mut self) {
        let Some(store) = &self.store else { return };
        let (installed, problems) = store.list();
        self.plugins.retain(|p| !p.stored);
        self.problems = problems.into_iter().map(|p| format!("{}: {}", p.dir.display(), p.error)).collect();
        for i in installed {
            if let Err(why) = i.manifest.target(&APP) {
                self.problems.push(format!("{}: {why}", i.manifest.name));
                continue;
            }
            let settings = read_settings(&i.dir);
            let source = serde_json::to_value(&i.record.source).unwrap_or(Value::Null);
            let plugin = Plugin { manifest: i.manifest, sha256: i.record.sha256, source, stored: true, wasm: Arc::new(i.bundle.wasm) };
            if !settings.is_empty() {
                self.settings.insert(plugin.manifest.id.clone(), settings);
            }
            self.plugins.retain(|p| !p.manifest.id.eq_ignore_ascii_case(&plugin.manifest.id));
            self.plugins.push(plugin);
        }
        publish(&self.plugins);
    }

    /// Keep plug-in `id`'s settings where it is stored (best effort: they stay for the session).
    fn save_settings(&self, id: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(store) = &self.store
            && self.get(id).is_some_and(|p| p.stored)
        {
            let mine = self.settings.get(id).cloned().unwrap_or_default();
            if let Ok(bytes) = serde_json::to_vec(&mine) {
                crate::store::put(&store.dir_of(id), SETTINGS_FILE, &bytes);
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = id;
    }

    /// Keep the catalogs list in the `Plug-ins` folder (best effort).
    fn save_catalogs(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(store) = &self.store
            && let Ok(bytes) = serde_json::to_vec_pretty(&self.catalogs)
        {
            crate::store::put(store.root(), CATALOGS_FILE, &bytes);
        }
    }
}

/// A plug-in's settings kept in its store folder `dir`; none when the file is missing or damaged.
#[cfg(not(target_arch = "wasm32"))]
fn read_settings(dir: &std::path::Path) -> Map<String, Value> {
    crate::store::read_capped(&dir.join(SETTINGS_FILE), MAX_SETTINGS_BYTES as u64 * 2)
        .ok()
        .and_then(|b| serde_json::from_slice::<Map<String, Value>>(&b).ok())
        .filter(|m| m.iter().map(|(k, v)| k.len() + v.to_string().len()).sum::<usize>() <= MAX_SETTINGS_BYTES)
        .unwrap_or_default()
}

/// The catalogs listed in the `Plug-ins` folder `dir`: addresses VectorCraft reads, at most
/// [`MAX_CATALOGS`].
#[cfg(not(target_arch = "wasm32"))]
fn read_catalogs(dir: &std::path::Path) -> Vec<String> {
    let Ok(bytes) = crate::store::read_capped(&dir.join(CATALOGS_FILE), MAX_CATALOGS_FILE) else { return Vec::new() };
    let list: Vec<String> = serde_json::from_slice(&bytes).unwrap_or_default();
    list.into_iter().filter(|u| catalog::check_url(u, None).is_ok()).take(MAX_CATALOGS).collect()
}

impl Session {
    /// Download catalogs and GitHub releases with `fetch` (the app's HTTPS client). Without one,
    /// `plugin.install` reads files and `file://` catalogs only.
    pub fn set_plugin_fetch(&mut self, fetch: Box<dyn Fetch + Send>) {
        self.craft_plugins.fetch = Some(fetch);
    }

    /// The shared command plug-ins installed, their store problems and the catalogs added.
    pub fn command_plugins(&self) -> &Runtime {
        &self.craft_plugins
    }

    /// Hand in a bundle downloaded away from the session (the app fetches on another thread, with
    /// [`obtain`]): the next `plugin.install` whose parameters name it takes it instead of
    /// downloading it again.
    pub fn hand_in_plugin_bundle(&mut self, candidate: Candidate) {
        self.craft_plugins.pending = Some(candidate);
    }
}

/// A fetcher for sessions without network access: `file://` catalogs still read.
struct NoNetwork;

impl Fetch for NoNetwork {
    fn get(&mut self, url: &str, _max: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        let shown: String = url.chars().take(120).collect();
        Err(format!("there is no network access here to read {shown}: install the plug-in from a file, or in the VectorCraft app"))
    }
}

/// Run `f` with the session's HTTPS client (or [`NoNetwork`]).
fn with_fetch<T>(s: &mut Session, f: impl FnOnce(&mut dyn Fetch) -> T) -> T {
    match s.craft_plugins.fetch.as_deref_mut() {
        Some(http) => f(http),
        None => f(&mut NoNetwork),
    }
}

/// A plug-in error as the engine reports it.
fn failed(name: &str, e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{name}: {e}"))
}

// ---------- running ----------

/// The sandbox's view of the session: every call is checked by [`refusal`] first, runs with no
/// file or folder readable or writable, and one that changes which documents are open or, from a
/// read-only plug-in, changes the document is put back and refused.
struct EngineHost<'s> {
    s: &'s mut Session,
    manifest: &'s Manifest,
    calls: Calls,
    notes: Vec<String>,
}

impl craft_plugin::Host for EngineHost<'_> {
    fn call(&mut self, command: &str, params: &str) -> std::result::Result<String, String> {
        let params: Value = serde_json::from_str(params).map_err(|e| format!("the params for `{command}` aren't JSON: {e}"))?;
        if let Some(why) = refusal(self.manifest, command, &params) {
            return Err(why);
        }
        let s = &mut *self.s;
        let checkpoint = Checkpoint::of(s);
        let before = s.active().map(|d| d.doc.clone());
        let r = super::plugin_call::execute_confined(s, command, &params);
        let changed = match (&before, s.active()) {
            (Some(b), Some(d)) => !Arc::ptr_eq(b, &d.doc),
            (None, None) => false,
            _ => true,
        };
        let refused = if !self.calls.kept(s) {
            Some(format!("`{command}` changes which documents are open, which plug-ins can't"))
        } else if changed {
            craft_plugin::check_read_only(self.manifest, command, true).err()
        } else {
            None
        };
        if let Some(why) = refused {
            checkpoint.restore(s);
            return Err(why);
        }
        r.map(|v| v.to_string()).map_err(|e| e.to_string())
    }

    fn selection(&mut self) -> String {
        let ids: Vec<u64> = self.s.active().map(|d| d.selection.objects.iter().map(|i| i.0).collect()).unwrap_or_default();
        json!(ids).to_string()
    }

    fn notify(&mut self, message: &str) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(message.chars().take(MAX_NOTE_CHARS).collect());
        }
    }

    fn log(&mut self, line: &str) {
        log::info!("plug-in {}: {line}", self.manifest.id);
    }

    fn setting(&mut self, key: &str) -> Option<String> {
        Some(self.s.craft_plugins.settings.get(&self.manifest.id)?.get(key)?.to_string())
    }

    fn set_setting(&mut self, key: &str, value: Option<&str>) -> std::result::Result<(), String> {
        let id = self.manifest.id.clone();
        set_setting(&mut self.s.craft_plugins.settings, &id, key, value)?;
        self.s.craft_plugins.save_settings(&id);
        Ok(())
    }
}

/// Store (`Some`) or forget (`None`) plug-in `id`'s setting `key` in `all`, within
/// [`MAX_SETTINGS_BYTES`] and [`MAX_SETTING_KEY`].
fn set_setting(all: &mut BTreeMap<String, Map<String, Value>>, id: &str, key: &str, value: Option<&str>) -> std::result::Result<(), String> {
    let Some(text) = value else {
        if let Some(mine) = all.get_mut(id) {
            mine.remove(key);
            if mine.is_empty() {
                all.remove(id);
            }
        }
        return Ok(());
    };
    if key.is_empty() || key.chars().count() > MAX_SETTING_KEY || key.chars().any(char::is_control) {
        return Err(format!("a setting's key must be 1–{MAX_SETTING_KEY} characters without control characters"));
    }
    let too_big = || format!("a plug-in's settings are limited to {} KB", MAX_SETTINGS_BYTES / 1024);
    if key.len() + text.len() > MAX_SETTINGS_BYTES {
        return Err(too_big());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("a setting must be JSON: {e}"))?;
    let others: usize =
        all.get(id).map_or(0, |mine| mine.iter().filter(|(k, _)| k.as_str() != key).map(|(k, v)| k.len() + v.to_string().len()).sum());
    if others + key.len() + v.to_string().len() > MAX_SETTINGS_BYTES {
        return Err(too_big());
    }
    all.entry(id.to_string()).or_default().insert(key.to_string(), v);
    Ok(())
}

/// Run command plug-in `id`'s `command` (default: its first) with `params` as one undo step named
/// after the plug-in (and the command, when it has several). A run that fails (a trap, an exhausted
/// budget, a refusal it passes on, a non-zero status) leaves every document, its selection and its
/// history as they were → `{plugin, command, output, notes, logs, calls, fuel, panel}`.
pub fn run(s: &mut Session, id: &str, command: Option<&str>, params: &Value, fuel: Option<u64>) -> Result<Value> {
    const C: &str = "plugin.run";
    let p = s.craft_plugins.get(id).cloned().ok_or_else(|| bad(C, format!("no plug-in `{id}` is installed (see plugin.list)")))?;
    let m = &p.manifest;
    let api = m.target(&APP).map_err(|e| bad(C, format!("{} can't run here: {e}", m.name)))?.api();
    let c = m.command(command).ok_or_else(|| bad(C, format!("{} has no command `{}`", m.name, command.unwrap_or_default())))?;
    let params = c.resolve_params(params).map_err(|e| bad(C, e))?;
    let mut limits = Limits::default();
    if let Some(f) = fuel {
        limits.fuel = f.clamp(1000, limits.fuel);
    }
    let mut instance = Instance::new(&p.wasm, &limits).map_err(|e| failed(&m.name, e))?;
    let label = if m.commands.len() > 1 { format!("{}: {}", m.name, c.label.trim_end_matches(['…', '.'])) } else { m.name.clone() };
    let command_id = c.id.clone();
    let mut notes = Vec::new();
    let ran = super::plugin_call::transaction(s, &label, |s| {
        let selection: Vec<u64> = s.active().map(|d| d.selection.objects.iter().map(|i| i.0).collect()).unwrap_or_default();
        let input = json!({"api": api, "command": command_id, "params": params, "selection": selection}).to_string();
        let calls = Calls::of(s, true, true);
        let mut host = EngineHost { s, manifest: m, calls, notes: Vec::new() };
        let ran: std::result::Result<RunOutput, craft_plugin::PluginError> = instance.call(&input, &mut host);
        notes = std::mem::take(&mut host.notes);
        let out = ran.map_err(|e| failed(&m.name, e))?;
        let output = serde_json::from_str::<Value>(&out.output).unwrap_or(Value::String(out.output));
        // Panels (`ui_show`) aren't shown in VectorCraft yet: say so rather than drop it silently.
        let panel = matches!(out.ui, Some(UiChange::Show(_))).then_some("VectorCraft doesn't show plug-in panels yet");
        Ok(json!({"output": output, "logs": out.logs, "calls": out.calls, "fuel": out.fuel, "panel": panel}))
    })?;
    let mut v = json!({"plugin": m.id, "command": c.id, "notes": notes});
    if let (Value::Object(v), Value::Object(r)) = (&mut v, ran) {
        v.extend(r);
    }
    Ok(v)
}

// ---------- installing ----------

/// The keys of `plugin.install`'s parameters that name a bundle, as [`Candidate::request`] keeps
/// them: a later install with the same ones takes the bundle reviewed or handed in.
pub fn request_key(p: &Value) -> Value {
    let mut out = Map::new();
    for k in ["path", "data", "name", "repo", "catalog", "id", "version"] {
        if let Some(v) = p.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
            out.insert(k.into(), json!(v));
        }
    }
    Value::Object(out)
}

/// Whether `plugin.install`'s parameters `p` name a shared bundle (`.craftplugin` file, `data`,
/// `repo` or `catalog`) rather than one of VectorCraft's own `.wasm` modules.
pub fn names_bundle(p: &Value) -> bool {
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    text("data").is_some()
        || text("repo").is_some()
        || text("catalog").is_some()
        || text("path").is_some_and(|path| path.to_ascii_lowercase().ends_with(&format!(".{}", bundle::BUNDLE_EXTENSION)))
}

/// Read the bundle `p` names: `path` (a `.craftplugin` file), `data` (base64), `repo`
/// (`owner/repo[@tag]`) or `catalog` + `id` (and optionally `version`), over `fetch`. `catalogs` are
/// the catalogs in use, to tell whether one lists a repository. Pure apart from reading the file or
/// the network, so the app can run it away from the session ([`Session::hand_in_plugin_bundle`]).
pub fn obtain(fetch: &mut dyn Fetch, catalogs: &[String], cmd: &str, p: &Value) -> Result<Candidate> {
    let bad = |msg: String| bad(cmd, msg);
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    let request = request_key(p);
    if let Some(data) = text("data") {
        if data.len() > bundle::MAX_BUNDLE_BYTES / 3 * 4 + 4 {
            return Err(bad(format!("the bundle is larger than {} MB", bundle::MAX_BUNDLE_BYTES >> 20)));
        }
        let bytes = vectorcraft_format::base64_decode(data).ok_or_else(|| bad("`data` is not valid base64".into()))?;
        let name = text("name").unwrap_or("data");
        return Ok(Candidate { bytes, source: json!({"type": "file", "path": name}), in_catalog: None, request });
    }
    if let Some(path) = text("path") {
        let bytes = read_bundle_file(path).map_err(|e| bad(format!("{}: {e}", super::fileio::shown_path(path))))?;
        return Ok(Candidate { bytes, source: json!({"type": "file", "path": path}), in_catalog: None, request });
    }
    if let Some(spec) = text("repo") {
        let repo = github::parse_repo_spec(spec).map_err(bad)?;
        let plan = fetch::resolve_repo(fetch, &repo).map_err(bad)?;
        let bytes = fetch::download_repo(fetch, &plan).map_err(bad)?;
        let source = json!({"type": "github", "repo": plan.repo.slug(), "tag": plan.repo.tag.clone().unwrap_or_default(), "asset": plan.asset});
        let in_catalog = catalogs.iter().filter(|url| local_refused(url).is_ok()).any(|url| {
            fetch::fetch_catalog(fetch, url)
                .is_ok_and(|(cat, _)| cat.packages.iter().any(|pkg| pkg.repo_ref().is_some_and(|r| r.same_repo(&repo.owner, &repo.name))))
        });
        return Ok(Candidate { bytes, source, in_catalog: Some(in_catalog), request });
    }
    if let Some(url) = text("catalog") {
        let id = text("id").ok_or_else(|| bad("`id` required with `catalog` (a package of the catalog)".into()))?;
        local_refused(url).map_err(bad)?;
        let (cat, _) = fetch::fetch_catalog(fetch, url).map_err(bad)?;
        let package = cat.package(id).ok_or_else(|| bad(format!("{url} has no package `{id}`")))?;
        let app = if package.kind == KIND_COMMAND { &APP } else { &OWN_KINDS };
        let release = match text("version") {
            Some(v) => package.release(v).ok_or_else(|| bad(format!("{id} has no version {v} in {url}")))?,
            None => package.latest_for(app).ok_or_else(|| bad(format!("{id} has no release for VectorCraft {}", APP.version)))?,
        };
        release.target(&package.kind, app).map_err(bad)?;
        let bytes = fetch::download_release(fetch, url, package, release).map_err(bad)?;
        let (_, manifest) = open(&bytes).map_err(bad)?;
        package.check_manifest(release, &manifest).map_err(bad)?;
        return Ok(Candidate { bytes, source: json!({"type": "catalog", "catalog": url, "package": id}), in_catalog: Some(true), request });
    }
    Err(bad("pass `path` (a .craftplugin file), `data` (a bundle in base64), `repo` (owner/repo or owner/repo@tag) or `catalog` with `id`".into()))
}

/// The bytes of the bundle file `path`, read through the checked file access.
#[cfg(not(target_arch = "wasm32"))]
fn read_bundle_file(path: &str) -> std::result::Result<Vec<u8>, String> {
    crate::file_access::check_read(path)?;
    crate::store::read_capped(std::path::Path::new(path), bundle::MAX_BUNDLE_BYTES as u64)
}

#[cfg(target_arch = "wasm32")]
fn read_bundle_file(_: &str) -> std::result::Result<Vec<u8>, String> {
    Err("installing from a path is not available on the web; pass the bundle as `data`".into())
}

/// The module check of a bundle: a command plug-in's against the shared sandbox, one of
/// VectorCraft's own kinds loaded as `vectorcraft-plugins` loads it (never run), its embedded
/// manifest naming the same id and kind as the bundle's.
fn check_module(m: &Manifest, wasm: &[u8]) -> std::result::Result<(), String> {
    if m.kind == KIND_COMMAND {
        return bundle::check_command_module(m, wasm);
    }
    let own = vectorcraft_plugins::Plugin::load(wasm, vectorcraft_plugins::Limits::default()).map_err(|e| e.to_string())?;
    if own.id() != m.id {
        return Err(format!("its module is the plug-in `{}`, not `{}`", own.id(), m.id));
    }
    let kind = own.manifest().kind.name();
    if kind != m.kind {
        return Err(format!("its module is a `{kind}` plug-in, and its manifest says `{}`", m.kind));
    }
    Ok(())
}

/// [`bundle::open_with`] with [`check_module`].
pub fn open(bytes: &[u8]) -> std::result::Result<(bundle::Bundle, Manifest), String> {
    bundle::open_with(bytes, &check_module)
}

/// Whether VectorCraft can run the plug-in `m` describes, or why not.
pub fn target(m: &Manifest) -> std::result::Result<u32, String> {
    let app = if m.kind == KIND_COMMAND { &APP } else { &OWN_KINDS };
    m.target(app).map(|t| t.api())
}

/// What an install confirmation shows about `c`: the manifest's permissions, kind and commands,
/// the source, the SHA-256, whether a catalog lists it and what it replaces. An error when the
/// bundle isn't a plug-in VectorCraft can run.
pub fn review(s: &Session, cmd: &str, c: &Candidate) -> Result<(Manifest, Value)> {
    let (b, manifest) = open(&c.bytes).map_err(|e| bad(cmd, e))?;
    target(&manifest).map_err(|e| bad(cmd, format!("{} can't be installed: {e}", manifest.name)))?;
    let mut v = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({"id": manifest.id}));
    v["kind"] = json!(manifest.kind);
    v["sha256"] = json!(bundle::sha256_hex(&c.bytes));
    v["source"] = c.source.clone();
    v["inCatalog"] = json!(c.in_catalog);
    if manifest.kind == KIND_COMMAND {
        let old = s.craft_plugins.get(&manifest.id);
        v["replaces"] = json!(old.map(|o| &o.manifest.version));
        v["permissionsChanged"] = json!(old.is_some_and(|o| craft_plugin::permissions::changed(&o.manifest, &manifest)));
        v["stored"] = json!(s.craft_plugins.has_store());
    } else {
        // One of VectorCraft's own kinds: what its module asks for, as `plugin.inspect` says it.
        if let Ok(own) = vectorcraft_plugins::Plugin::load(&b.wasm, vectorcraft_plugins::Limits::default()) {
            let mut inspected = super::plugin::describe(&own);
            inspected["needsApproval"] = json!(!s.plugin_approved(&own));
            v["module"] = inspected;
        }
        v["replaces"] = json!(vectorcraft_plugins::registry::get(&manifest.id).map(|p| p.manifest().version.clone()));
        v["stored"] = json!(s.plugin_dir.is_some());
    }
    Ok((manifest, v))
}

/// `plugin.install` for a bundle ([`names_bundle`]): obtain it (the bundle reviewed or handed in
/// for these parameters, else read or downloaded now), check it, and with `review: true` only show
/// what it is (and keep it for the install that follows); otherwise install it, refused when
/// `sha256` is given and the bundle has another. A command plug-in goes to the shared store (or the
/// session, without one); a bundle of one of VectorCraft's own kinds installs its module as
/// `plugin.install {dataBase64}` would, with the same parameters (`grant`, `allowHeavy`,
/// `replace`, `hash`).
pub(crate) fn install_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.install";
    let review_only = super::switch_or(p, "review", false, C)?;
    let wanted = p.get("sha256").and_then(Value::as_str).map(str::to_ascii_lowercase);
    let request = request_key(p);
    let held = s.craft_plugins.pending.take().filter(|h| h.request == request && wanted.as_ref().is_none_or(|w| *w == bundle::sha256_hex(&h.bytes)));
    let c = match held {
        Some(c) => c,
        None => {
            let catalogs = s.craft_plugins.catalogs.clone();
            with_fetch(s, |f| obtain(f, &catalogs, C, p))?
        }
    };
    let sha256 = bundle::sha256_hex(&c.bytes);
    if wanted.as_ref().is_some_and(|w| *w != sha256) {
        return Err(bad(C, "the bundle doesn't have the SHA-256 `sha256` gives: nothing was installed"));
    }
    let (manifest, shown) = review(s, C, &c)?;
    if review_only {
        s.craft_plugins.pending = Some(c);
        return Ok(shown);
    }
    if manifest.kind != KIND_COMMAND {
        let (b, _) = open(&c.bytes).map_err(|e| bad(C, e))?;
        let own = vectorcraft_plugins::Plugin::load(&b.wasm, vectorcraft_plugins::Limits::default())
            .map_err(|e| EngineError::Other(e.to_string()))?
            .with_source(source_text(&c.source));
        let mut v = super::plugin::install_given(s, p, Some((own, b.wasm)))?;
        v["sha256"] = json!(sha256);
        v["bundleSource"] = c.source;
        return Ok(v);
    }
    if vectorcraft_plugins::registry::get(&manifest.id).is_some() {
        return Err(bad(C, format!("a VectorCraft plug-in with id {:?} is installed: remove it first", manifest.id)));
    }
    let plugin = install(s, C, c, manifest)?;
    Ok(plugin.describe())
}

/// How a VectorCraft plug-in installed from a bundle shows where it came from (`plugin.list`'s
/// `source`).
fn source_text(source: &Value) -> String {
    let t = |k: &str| source.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match source.get("type").and_then(Value::as_str) {
        Some("github") => format!("github:{}@{}", t("repo"), t("tag")),
        Some("catalog") => format!("catalog:{} {}", t("catalog"), t("package")),
        _ => t("path"),
    }
}

/// Install command plug-in `c` (reviewed, its manifest `manifest`): into the user's folder when the
/// session has one, else for the session. Replaces an installed plug-in with the same id; its
/// settings stay.
fn install(s: &mut Session, cmd: &str, c: Candidate, manifest: Manifest) -> Result<Plugin> {
    let sha256 = bundle::sha256_hex(&c.bytes);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(store) = &s.craft_plugins.store {
        let source: craft_plugin::store::Source = serde_json::from_value(c.source.clone()).map_err(|e| bad(cmd, e.to_string()))?;
        let i = store.install(&c.bytes, source).map_err(|e| bad(cmd, e))?;
        let plugin = Plugin { manifest: i.manifest, sha256: i.record.sha256, source: c.source, stored: true, wasm: Arc::new(i.bundle.wasm) };
        let id = plugin.manifest.id.clone();
        let p = keep(s, plugin);
        s.craft_plugins.save_settings(&id);
        return Ok(p);
    }
    let (b, _) = bundle::open(&c.bytes).map_err(|e| bad(cmd, e))?;
    Ok(keep(s, Plugin { manifest, sha256, source: c.source, stored: false, wasm: Arc::new(b.wasm) }))
}

/// Put `plugin` in the session's list, in place of one with the same id.
fn keep(s: &mut Session, plugin: Plugin) -> Plugin {
    let list = &mut s.craft_plugins.plugins;
    match list.iter_mut().find(|p| p.manifest.id.eq_ignore_ascii_case(&plugin.manifest.id)) {
        Some(slot) => *slot = plugin.clone(),
        None => list.push(plugin.clone()),
    }
    publish(list);
    plugin
}

/// Remove command plug-in `id` and its settings; whether it was installed.
pub(crate) fn remove(s: &mut Session, cmd: &str, id: &str) -> Result<bool> {
    let Some(p) = s.craft_plugins.get(id).cloned() else { return Ok(false) };
    #[cfg(not(target_arch = "wasm32"))]
    if p.stored
        && let Some(store) = &s.craft_plugins.store
    {
        store.remove(&p.manifest.id).map_err(|e| bad(cmd, e))?;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = cmd;
    s.craft_plugins.plugins.retain(|x| !x.manifest.id.eq_ignore_ascii_case(id));
    publish(&s.craft_plugins.plugins);
    s.craft_plugins.settings.remove(&p.manifest.id);
    Ok(true)
}

/// Why the catalog `url` can't be read here: a `file://` catalog (and the files it names) is read
/// straight from the disk, past the automation roots, so it is refused while they are in force.
fn local_refused(url: &str) -> std::result::Result<(), String> {
    if url.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("file://")) && crate::file_access::current().is_some() {
        return Err("file:// catalogs can't be read while automation is confined to folders".into());
    }
    Ok(())
}

/// `plugin.catalog.list`: the packages of catalog `url` (default: every catalog added) VectorCraft
/// can run, with their newest release for it.
pub(crate) fn catalog_list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.catalog.list";
    let urls: Vec<String> = match super::str_param(p, "url").map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => vec![u.to_string()],
        None => s.craft_plugins.catalogs.clone(),
    };
    let (mut packages, mut warnings) = (Vec::new(), Vec::new());
    for url in &urls {
        local_refused(url).map_err(|e| bad(C, e))?;
        let (cat, warned) = with_fetch(s, |f| fetch::fetch_catalog(f, url)).map_err(|e| bad(C, e))?;
        warnings.extend(warned);
        for pkg in &cat.packages {
            let app = if pkg.kind == KIND_COMMAND { &APP } else { &OWN_KINDS };
            let Some(release) = pkg.latest_for(app) else { continue };
            let installed = match s.craft_plugins.get(&pkg.id) {
                Some(p) => Some(p.manifest.version.clone()),
                None => vectorcraft_plugins::registry::get(&pkg.id).map(|p| p.manifest().version.clone()),
            };
            packages.push(json!({
                "catalog": url, "id": pkg.id, "name": pkg.name, "version": release.version, "author": pkg.author,
                "description": pkg.description, "license": pkg.license, "repo": pkg.repo, "kind": pkg.kind,
                "sha256": release.sha256, "installed": installed,
            }));
        }
    }
    Ok(json!({"catalogs": urls, "packages": packages, "warnings": warnings}))
}

/// `plugin.catalog.add {url, remove?}`: add the catalog `url` (`https://` on GitHub's hosts, or
/// `file://`) to the ones in use, or take it out.
pub(crate) fn catalog_add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.catalog.add";
    let url = super::str_param(p, "url").map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| bad(C, "missing `url`"))?.to_string();
    let take_out = super::switch_or(p, "remove", false, C)?;
    let list = &mut s.craft_plugins.catalogs;
    if take_out {
        list.retain(|u| *u != url);
    } else {
        local_refused(&url).map_err(|e| bad(C, e))?;
        catalog::check_url(&url, None).map_err(|e| bad(C, e))?;
        if !list.contains(&url) {
            if list.len() >= MAX_CATALOGS {
                return Err(bad(C, format!("at most {MAX_CATALOGS} catalogs can be added")));
            }
            list.push(url);
        }
    }
    s.craft_plugins.save_catalogs();
    Ok(json!({"catalogs": s.craft_plugins.catalogs}))
}

/// Where command plug-in `p` has a newer release: the install parameters that get it and its
/// version (a tag, for a repository). `None` when it is up to date or came from a file.
fn update_for(s: &mut Session, p: &Plugin) -> std::result::Result<Option<(Value, String)>, String> {
    let text = |k: &str| p.source.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match p.source.get("type").and_then(Value::as_str) {
        Some("catalog") => {
            let (url, id) = (text("catalog"), text("package"));
            local_refused(&url)?;
            let (cat, _) = with_fetch(s, |f| fetch::fetch_catalog(f, &url))?;
            let release = cat.package(&id).and_then(|pkg| pkg.latest_for(&APP));
            Ok(release
                .filter(|r| bundle::newer(&r.version, &p.manifest.version))
                .map(|r| (json!({"catalog": url, "id": id, "version": r.version}), r.version.clone())))
        }
        Some("github") => {
            let mut repo = github::parse_repo_spec(&text("repo"))?;
            repo.tag = None;
            let plan = with_fetch(s, |f| fetch::resolve_repo(f, &repo))?;
            let tag = plan.repo.tag.clone().unwrap_or_default();
            Ok((tag != text("tag")).then(|| (json!({"repo": format!("{}@{tag}", plan.repo.slug())}), tag)))
        }
        _ => Ok(None),
    }
}

/// `plugin.update {id?, install?}`: checks command plug-in `id` (default: every one installed from
/// a catalog or a repository) for a newer release; with `install: true`, installs each whose
/// permissions and kind stay the same, and leaves the others for a reviewed `plugin.install`.
pub(crate) fn update(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.update";
    let apply = super::switch_or(p, "install", false, C)?;
    let plugins: Vec<Plugin> = match super::str_param(p, "id").map(str::trim).filter(|t| !t.is_empty()) {
        Some(id) => {
            vec![s.craft_plugins.get(id).cloned().ok_or_else(|| bad(C, format!("no command plug-in `{id}` is installed (see plugin.list)")))?]
        }
        None => s.craft_plugins.plugins.clone(),
    };
    let mut out = Vec::new();
    for plugin in plugins {
        let id = plugin.manifest.id.clone();
        let mut row = json!({"id": id, "version": plugin.manifest.version});
        match update_for(s, &plugin) {
            Err(e) => row["error"] = json!(e),
            Ok(None) => row["upToDate"] = json!(true),
            Ok(Some((install, to))) => {
                row["available"] = json!(to);
                row["install"] = install.clone();
                if apply {
                    let catalogs = s.craft_plugins.catalogs.clone();
                    let done = with_fetch(s, |f| obtain(f, &catalogs, C, &install)).and_then(|c| {
                        let (manifest, _) = review(s, C, &c)?;
                        if manifest.kind != KIND_COMMAND || craft_plugin::permissions::changed(&plugin.manifest, &manifest) {
                            return Ok(false);
                        }
                        install_reviewed(s, c, manifest).map(|_| true)
                    });
                    match done {
                        Ok(true) => row["updated"] = json!(true),
                        Ok(false) => row["needsReview"] = json!(true),
                        Err(e) => row["error"] = json!(e.to_string()),
                    }
                }
            }
        }
        out.push(row);
    }
    Ok(json!({"plugins": out}))
}

/// Install command plug-in `c` once checked ([`review`]).
fn install_reviewed(s: &mut Session, c: Candidate, manifest: Manifest) -> Result<Plugin> {
    install(s, "plugin.update", c, manifest)
}

#[cfg(test)]
mod tests;
