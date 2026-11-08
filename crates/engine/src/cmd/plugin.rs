//! `plugin.*` commands: sandboxed WebAssembly plug-ins (`vectorcraft-plugins`).
//!
//! Plug-ins are installed with `plugin.install` (kept across restarts in the apps' plug-in folder,
//! [`Session::attach_plugin_store`]) or from every `*.wasm` in the Additional Plug-ins Folder
//! preference, and listed with `plugin.list` / `plugin.info`. Object filters run
//! with `plugin.run` on the selected paths and compound paths (one undo step, transactional: a
//! plug-in that fails leaves the document untouched), and read and write the data they keep with
//! the document ([`vectorcraft_doc::plugin_data`], theirs alone); live-effect plug-ins are applied
//! like any effect (`effect.apply {effect: "plugin.<id>"}`) and drawn by `vectorcraft-effects`;
//! exporters add a file format that `document.export` writes, and importers one that
//! `document.open` reads (`fileio::plugin_formats`).
//!
//! A filter may ask for a heavy budget (`Budget::Heavy`: minutes rather than a minute, for routing,
//! packing or simulation). It installs only once the user agrees (`plugin.install {allowHeavy}`),
//! which loading a folder doesn't ask, so folders skip it; the app runs it in the background with
//! Cancel ([`FilterRun`]).
//!
//! A plug-in whose manifest lists permissions installs once the user grants them
//! (`plugin.install {grant}`, recorded with the module's hash and kept in the plug-in store), and
//! a filter does only what they allow: without `doc.read` it is given no objects, without
//! `doc.write` an output that changes the document is refused, and without `state` it neither
//! reads nor writes the data it keeps with the document.
//!
//! A plug-in of API level 2 that lists `progress` may import the host's `vc_progress`,
//! `vc_cancelled` and `vc_log` ([`vectorcraft_plugins::host`]): run inside a
//! [`vectorcraft_doc::cancel::scope`] (the app's heavy filters and background exports), its
//! progress goes to the scope's sink and it sees the scope's Cancel; its log lines go to the log
//! under its id.

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node};
use vectorcraft_plugins::{Budget, Kind, Plugin, effect, export, objects, registry};

use super::{CommandSpec, EngineError, NodeId, Result, Session, always, bad, cmd, has_doc, ids_param, str_param, switch_or};

use vectorcraft_plugins::permissions::{self, Grant, Permission};

/// What a plug-in error looks like to the user. A run the caller cancelled changed nothing.
fn err(e: vectorcraft_plugins::Error) -> EngineError {
    match e {
        vectorcraft_plugins::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

/// JSON description of an installed plug-in.
pub fn describe(p: &Plugin) -> Value {
    let m = p.manifest();
    let mut v = json!({
        "id": m.id, "name": m.name, "version": m.version, "kind": m.kind.name(), "author": m.author, "description": m.description,
        "params": m.params_doc(), "paramsSchema": serde_json::to_value(m).ok().and_then(|v| v.get("params").cloned()),
        "size": p.size(), "source": p.source(),
        "permissions": permissions::names(&m.permissions()), "hash": p.hash(), "api": m.api,
    });
    match m.kind {
        Kind::Effect => v["effect"] = json!(effect::effect_id(&m.id)),
        Kind::Exporter | Kind::Importer => {
            v["format"] = json!(export::format_id(&m.id));
            v["extensions"] = json!(m.extensions);
        }
        Kind::Filter => {}
    }
    if m.budget == Budget::Heavy {
        v["budget"] = json!("heavy");
    }
    v
}

/// The plug-in's own parameters from command params: `params` if given, else the top-level keys
/// minus the ones the command itself uses.
fn plugin_params(p: &Value) -> Value {
    if let Some(v) = p.get("params") {
        return v.clone();
    }
    match p {
        Value::Object(m) => {
            Value::Object(m.iter().filter(|(k, _)| !matches!(k.as_str(), "id" | "ids")).map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        _ => Value::Object(Map::new()),
    }
}

fn plugin_by_id(cmd: &str, p: &Value) -> Result<Arc<Plugin>> {
    let id = str_param(p, "id").ok_or_else(|| bad(cmd, "missing `id` (see plugin.list)"))?;
    registry::get(id).ok_or_else(|| err(vectorcraft_plugins::Error::NotFound(id.into())))
}

/// One run of an object filter, split so the plug-in can compute away from the session: made from
/// the active document ([`FilterRun::prepare`]), run on any thread ([`FilterRun::run`] touches no
/// document), then handed in ([`FilterRun::hand_in`]) for `plugin.run` to apply instead of running
/// the plug-in again. The app runs a heavy filter ([`Budget::Heavy`]) this way, in the background
/// with Cancel, and the edit itself stays a `plugin.run`: journaled, guarded and undone as one step
/// like any other command.
pub struct FilterRun {
    plugin: Arc<Plugin>,
    input: Vec<u8>,
    params: Vec<u8>,
    /// The objects the plug-in is given, and the container new objects go in.
    inputs: Vec<NodeId>,
    parent: Option<NodeId>,
    /// The document it was made from, and that document's revision then.
    document: (u64, u64),
}

/// A filter's output handed in by a [`FilterRun`] made away from `plugin.run`, with what the run
/// was made of: only a run of the same installation of the plug-in on the same input with the same
/// parameters takes it (a plug-in's output depends on nothing else).
pub(crate) struct HandedIn {
    serial: u64,
    input: Vec<u8>,
    params: Vec<u8>,
    output: Vec<u8>,
}

impl FilterRun {
    /// The run `plugin.run`'s params `p` ask for, on the active document: the plug-in (a filter),
    /// its checked parameters and `_context`, and the selection's objects.
    pub fn prepare(s: &Session, p: &Value) -> Result<FilterRun> {
        const C: &str = "plugin.run";
        let plugin = plugin_by_id(C, p)?;
        let m = plugin.manifest();
        match m.kind {
            Kind::Filter => {}
            Kind::Effect => {
                return Err(bad(
                    C,
                    format!("`{}` is a live effect: apply it with effect.apply {{\"effect\": \"{}\"}}", m.id, effect::effect_id(&m.id)),
                ));
            }
            Kind::Exporter => {
                return Err(bad(
                    C,
                    format!("`{}` is an exporter: export with document.export {{\"format\": \"{}\"}}", m.id, export::format_id(&m.id)),
                ));
            }
            Kind::Importer => {
                return Err(bad(
                    C,
                    format!("`{}` is an importer: open a file with document.open {{\"importer\": \"{}\"}}", m.id, export::format_id(&m.id)),
                ));
            }
        }
        // Bad parameters fail before anything runs.
        let mut params = Value::Object(m.resolve_params(&plugin_params(p)).map_err(|e| bad(C, e.to_string()))?);
        let may = m.permissions();
        let st = s.doc()?;
        let selected = ids_param(p, "ids").unwrap_or_else(|| st.selection.objects.clone());
        // Without `doc.read` a filter is given no objects: it can only add new ones.
        let inputs = if may.contains(&Permission::DocRead) { objects::filter_targets(&st.doc, &selected) } else { Vec::new() };
        let parent = st.insertion_parent();
        let bounds = st.doc.bounds_of(&inputs, false);
        let board = bounds.and_then(|b| st.doc.artboard_at(b.center())).and_then(|i| st.doc.artboards.get(i)).or(st.doc.artboards.first());
        let rect = |r: vectorcraft_geom::Rect| json!([r.x0, r.y0, r.x1, r.y1]);
        // What the plug-in keeps with the document, as it last wrote it.
        let data = st.doc.plugin_data(&m.id).filter(|_| may.contains(&Permission::State)).cloned().unwrap_or(Value::Null);
        params["_context"] = json!({"mode": "filter", "bounds": bounds.map(rect), "artboard": board.map(|a| rect(a.rect)), "documentData": data});
        let input = objects::encode_input(&st.doc, &inputs);
        let params = serde_json::to_vec(&params).map_err(|e| bad(C, e.to_string()))?;
        let document = (st.uid, st.revision);
        Ok(FilterRun { plugin, input, params, inputs, parent, document })
    }

    /// The plug-in, for its name and budget.
    pub fn plugin(&self) -> &Arc<Plugin> {
        &self.plugin
    }

    /// The objects it gives the plug-in: `plugin.run`'s `ids` that make this same run again.
    pub fn inputs(&self) -> &[NodeId] {
        &self.inputs
    }

    /// The document it was made from (`DocState::uid`).
    pub fn document(&self) -> u64 {
        self.document.0
    }

    /// Does it still apply: is the active document the one it was made from, unchanged since, and
    /// the plug-in still installed as it was?
    pub fn current(&self, s: &Session) -> bool {
        let installed = registry::get(self.plugin.id()).is_some_and(|p| p.serial() == self.plugin.serial());
        installed && s.active().is_some_and(|st| (st.uid, st.revision) == self.document)
    }

    /// Runs the plug-in → its output, or why it failed or stopped. It touches no document, so it
    /// can run on any thread; inside a [`vectorcraft_doc::cancel::scope`] Cancel stops it between
    /// two slices of its budget.
    pub fn run(&self) -> Result<Vec<u8>> {
        self.plugin.run(&self.input, &self.params).map_err(err)
    }

    /// Hands in `output`, which [`FilterRun::run`] returned: the next `plugin.run` that makes this
    /// same run (same plug-in, `ids` and parameters, the document unchanged) applies it instead of
    /// running the plug-in. Any other run drops it.
    pub fn hand_in(self, s: &mut Session, output: Vec<u8>) {
        s.plugin_output = Some(HandedIn { serial: self.plugin.serial(), input: self.input, params: self.params, output });
    }

    /// Applies the plug-in's `output` to the active document as one undo step: its objects, and what
    /// it keeps with the document from now on. An output its permissions don't allow is refused,
    /// and the document stays as it was.
    fn finish(self, s: &mut Session, output: &[u8]) -> Result<Value> {
        let m = self.plugin.manifest();
        let may = m.permissions();
        let out = objects::decode_output(output).map_err(err)?;
        if out.document_data.is_some() && !may.contains(&Permission::State) {
            return Err(not_granted(&m.name, Permission::State));
        }
        let may_write = may.contains(&Permission::DocWrite);
        let (inputs, parent) = (self.inputs, self.parent);
        let ids = s.edit(m.name.trim_end_matches('…'), |d, sel| {
            let before = (!may_write).then(|| d.layers.clone());
            let ids = objects::apply_output(d, &inputs, out.objects, parent, &admit_effects).map_err(err)?;
            if before.is_some_and(|b| b != d.layers) {
                return Err(not_granted(&m.name, Permission::DocWrite));
            }
            if let Some(data) = out.document_data {
                d.set_plugin_data(&m.id, data);
            }
            sel.set(ids.iter().copied().filter(|&id| d.node(id).is_some_and(objects::selectable)));
            Ok(ids)
        })?;
        Ok(json!({"plugin": m.id, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
    }
}

/// The error of a filter `name` whose output needs `permission`, which it wasn't granted.
fn not_granted(name: &str, permission: Permission) -> EngineError {
    let (name, p) = (permissions::display_text(name, permissions::MAX_SHOWN_NAME), permission.name());
    err(vectorcraft_plugins::Error::Output(format!("“{name}” wasn't granted `{p}`, which its output needs: nothing was changed")))
}

/// `plugin.run`: runs filter `id` on `ids` (else the selected paths and compound paths) with its
/// parameters and `_context` (what it keeps with the document included), and applies its output as
/// one undo step ([`objects::apply_output`]; effect records as [`admit_effects`] admits them), with
/// what it keeps with the document from now on: the output handed in for this very run when there
/// is one ([`FilterRun::hand_in`]), else the plug-in's, run now. → `{plugin, ids}`: what it made or
/// changed, which is then selected.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    // An output handed in is for this run only (the app hands it in just before), whatever it does.
    let handed = s.plugin_output.take();
    let r = FilterRun::prepare(s, p)?;
    // Handed in for this very run (the app ran it in the background), it is what running the
    // plug-in again would return.
    let handed = handed.filter(|h| (h.serial, &h.input, &h.params) == (r.plugin.serial(), &r.input, &r.params));
    let output = match handed {
        Some(h) => h.output,
        None => r.run()?,
    };
    r.finish(s, &output)
}

/// Admits the effect records a filter returned for object `n` (written, its effects still its own)
/// as `effect.apply` would apply them: each new record a known effect (one of ours, or an installed
/// live-effect plug-in) with parameters it takes, filled in to their defaults. A record the object
/// already had stays as it was. An object takes one 3D effect (its fills' and strokes' own
/// counted), which must fit it, and the `textureImage` of an effect with Load Texture (Glass,
/// Texturizer) comes as the effect stores it, never as a file to read: a plug-in can't have the
/// host read one.
fn admit_effects(fx: &[Effect], n: &Node) -> std::result::Result<Vec<Effect>, String> {
    admit_effects_for("plugin.run", fx, n)
}

/// [`admit_effects`] for command `cmd`, which bad parameters name: `plugin.run`, or `document.open`
/// for the objects an importer plug-in returns.
pub(crate) fn admit_effects_for(cmd: &str, fx: &[Effect], n: &Node) -> std::result::Result<Vec<Effect>, String> {
    use vectorcraft_render::effects;
    let mut out = Vec::with_capacity(fx.len());
    for (j, e) in fx.iter().enumerate() {
        if n.appearance.effects.contains(e) {
            out.push(e.clone());
            continue;
        }
        let refuse = |why: String| format!("[{j}] `{}`: {why}", e.id);
        let params = if e.params.is_null() { json!({}) } else { e.params.clone() };
        match effect::installed(&e.id) {
            Some(plugin) => {
                plugin.manifest().resolve_params(&params).map_err(|why| refuse(why.to_string()))?;
            }
            None => super::params::check_effect_params(cmd, &e.id, &params).map_err(|why| refuse(why.to_string()))?,
        }
        if super::effectcmd::texture_load_effect(&e.id)
            && let Some(t) = params.get(effects::pixel::TEXTURE_IMAGE)
            && t.as_str().is_none_or(|t| effects::pixel::texture(t).is_none())
        {
            return Err(refuse("`textureImage` must be a texture as effect.list gives it".into()));
        }
        let mut admitted = effects::new_effect(&e.id, &params).ok_or_else(|| refuse("no such effect".into()))?;
        if effects::is_three_d(&e.id) {
            effects::validate_three_d(n, &e.id, &admitted.params).map_err(refuse)?;
        }
        admitted.visible = e.visible;
        out.push(admitted);
    }
    let items = n.appearance.items.iter().flat_map(|i| i.effects());
    if out.iter().chain(items).filter(|e| effects::is_three_d(&e.id)).count() > 1 {
        return Err(super::effectcmd::ONE_3D.into());
    }
    Ok(out)
}

/// Largest base64 payload `plugin.install` accepts (the module limit, encoded).
const MAX_DATA_CHARS: usize = (32 << 20) / 3 * 4 + 4;

/// The `.wasm` extension plug-in files have (File › Open installs them).
pub const EXTS: &[&str] = &["wasm"];

/// The plug-in module command `c`'s params give, loaded and checked, and its bytes: `path` (a
/// `.wasm` file) or `dataBase64` (the module, with `name` shown as its source).
fn load_given(c: &str, p: &Value) -> Result<(Plugin, Vec<u8>)> {
    match (str_param(p, "path").filter(|s| !s.is_empty()), str_param(p, "dataBase64")) {
        (_, Some(data)) => {
            if data.len() > MAX_DATA_CHARS {
                return Err(bad(c, "`dataBase64` is larger than the module limit"));
            }
            let bytes = vectorcraft_format::base64_decode(data).ok_or_else(|| bad(c, "`dataBase64` is not valid base64"))?;
            let plugin = Plugin::load(&bytes, vectorcraft_plugins::Limits::default()).map_err(err)?;
            let plugin = match str_param(p, "name") {
                Some(name) => plugin.with_source(name),
                None => plugin,
            };
            Ok((plugin, bytes))
        }
        (Some(path), None) => load_path(path),
        (None, None) => Err(bad(c, "give `path` (a .wasm file) or `dataBase64` (the module)")),
    }
}

/// The permissions `grant` lists in command `c`'s params: `None` when it is left out (or null).
/// Anything but a list of permission names is an error.
fn grant_param(c: &str, p: &Value) -> Result<Option<Vec<Permission>>> {
    let Some(v) = p.get("grant").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let names = v.as_array().ok_or_else(|| bad(c, "`grant` must be a list of permission names"))?;
    let mut out = Vec::with_capacity(names.len());
    for n in names {
        let name = n.as_str().ok_or_else(|| bad(c, "`grant` must be a list of permission names"))?;
        let shown = permissions::display_text(name, permissions::MAX_SHOWN_FIELD);
        out.push(Permission::parse(name).ok_or_else(|| bad(c, format!("`grant` names an unknown permission `{shown}`")))?);
    }
    Ok(Some(out))
}

/// `plugin.install`: loads and checks the module, refuses one that asks for a heavy budget unless
/// `allowHeavy` says the user agreed, and one that lists permissions the user hasn't granted its id
/// unless `grant` lists them all, and one whose SHA-256 isn't the `hash` given; records the grant
/// with the module's hash, keeps it for the next start and installs it → its `plugin.list` entry.
fn install(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.install";
    let (replace, allow_heavy) = (switch_or(p, "replace", true, C)?, switch_or(p, "allowHeavy", false, C)?);
    let grant = grant_param(C, p)?;
    let (plugin, bytes) = load_given(C, p)?;
    // `hash` pins the module the user was shown: one with other bytes is refused.
    if let Some(h) = p.get("hash").filter(|v| !v.is_null())
        && h.as_str() != Some(plugin.hash())
    {
        return Err(bad(C, "the module doesn't have the SHA-256 `hash` gives: nothing was installed"));
    }
    if !replace && registry::get(plugin.id()).is_some() {
        return Err(bad(C, format!("a plug-in with id {:?} is already installed", plugin.id())));
    }
    // A heavy plug-in is installed once the user agrees: whoever installs it asks them first.
    if plugin.manifest().budget == Budget::Heavy && !allow_heavy {
        let (minutes, times) = vectorcraft_plugins::heavy_terms();
        return Err(bad(
            C,
            format!(
                "“{}” asks for a heavy budget (each run up to {minutes} minutes and {times} times the usual instructions): install it with `allowHeavy: true` once the user agrees",
                plugin.manifest().name
            ),
        ));
    }
    // A plug-in that lists permissions installs once the user grants them: whoever installs it
    // asks them first, unless they already granted its id all of them.
    let m = plugin.manifest();
    let wanted = m.permissions();
    if !s.plugin_approved(&plugin) && !grant.as_ref().is_some_and(|g| wanted.iter().all(|w| g.contains(w))) {
        let name = permissions::display_text(&m.name, permissions::MAX_SHOWN_NAME);
        let asked = permissions::names(&wanted).join(", ");
        return Err(bad(C, format!("“{name}” asks for the permissions {asked}: install it with `grant` listing them once the user agrees")));
    }
    if m.asks_permissions() {
        let g = Grant { id: m.id.clone(), hash: plugin.hash().to_string(), permissions: wanted };
        registry::grant(g.clone());
        s.store_grant(&m.id, Some(g));
    }
    // Installed plug-ins stay installed after a restart (#2541).
    s.store_plugin(plugin.id(), &bytes);
    Ok(describe(&registry::install(plugin)))
}

/// `plugin.inspect`: what installing the module would install, without installing it, and whether
/// that needs the user to grant its permissions first (`needsApproval`).
fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    load_given("plugin.inspect", p).map(|(plugin, _)| {
        let mut v = describe(&plugin);
        v["needsApproval"] = json!(!s.plugin_approved(&plugin));
        v
    })
}

/// The plug-in in the `.wasm` file `path`, and its module.
#[cfg(not(target_arch = "wasm32"))]
fn load_path(path: &str) -> Result<(Plugin, Vec<u8>)> {
    crate::file_access::check_read(path).map_err(EngineError::Other)?;
    let limits = vectorcraft_plugins::Limits::default();
    let bytes = crate::store::read_capped(std::path::Path::new(path), limits.max_module_bytes as u64).map_err(EngineError::Other)?;
    Ok((Plugin::load(&bytes, limits).map_err(err)?.with_source(path), bytes))
}

#[cfg(target_arch = "wasm32")]
fn load_path(_: &str) -> Result<(Plugin, Vec<u8>)> {
    Err(EngineError::Other("installing from a path is not available on the web; pass the module as `dataBase64`".into()))
}

/// The plug-in folder last loaded, the plug-in store whose grants it was loaded with, and what
/// happened.
static FOLDER: Mutex<Option<(String, Option<std::path::PathBuf>, Value)>> = Mutex::new(None);

/// Loads the Additional Plug-ins Folder preference (`*.wasm`) when it is set and it, or the
/// session's plug-in store, has changed since the last load. Runs whenever preferences are applied
/// (startup, `prefs.set`). Native only.
pub(crate) fn sync_prefs(s: &Session) {
    let folder = s.prefs.plugins_folder.trim();
    if folder.is_empty() {
        return;
    }
    let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref().is_some_and(|(f, store, _)| f == folder && *store == s.plugin_dir) {
        return;
    }
    *last = Some((folder.to_string(), s.plugin_dir.clone(), load_folder(s, folder)));
}

/// Loads the plug-ins in `folder`, those that list permissions only when `s` has them granted
/// ([`Session::plugin_approved`], with the store's grants read once).
#[cfg(not(target_arch = "wasm32"))]
fn load_folder(s: &Session, folder: &str) -> Value {
    if let Err(e) = crate::file_access::check_read(folder) {
        return json!({"folder": folder, "error": e});
    }
    // A file in it may be a link to one elsewhere.
    let allow = |f: &std::path::Path| match f.to_str() {
        Some(f) => crate::file_access::check_read(f),
        // A lossy name would check another file than the one loaded.
        None if crate::file_access::current().is_none() => Ok(()),
        None => Err(format!("automation path rejected: {} isn't valid text", f.display())),
    };
    match registry::load_folder_where(std::path::Path::new(folder), allow, s.plugin_approver()) {
        Ok(r) => {
            json!({"folder": folder, "loaded": r.loaded, "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>()})
        }
        Err(e) => json!({"folder": folder, "error": e.to_string()}),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_folder(_: &Session, folder: &str) -> Value {
    json!({"folder": folder, "error": "plug-in folders are not available on the web"})
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.reload";
    let folder = match p.get("path") {
        Some(v) => v.as_str().map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "`path` must be a folder"))?.to_string(),
        None => s.prefs.plugins_folder.trim().to_string(),
    };
    if folder.is_empty() {
        return Err(bad(C, "no folder: pass `path` or set Preferences › Plug-ins & Scratch Disks › Additional Plug-ins Folder"));
    }
    let report = load_folder(s, &folder);
    if let Some(e) = report.get("error").and_then(Value::as_str) {
        return Err(EngineError::Other(e.to_string()));
    }
    *FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder, s.plugin_dir.clone(), report.clone()));
    Ok(report)
}

fn list(_: &mut Session, _: &Value) -> Result<Value> {
    let folder = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, _, r)| r.clone());
    Ok(json!({"plugins": registry::list().iter().map(|p| describe(p)).collect::<Vec<_>>(), "folder": folder}))
}

fn info(_: &mut Session, p: &Value) -> Result<Value> {
    let plugin = plugin_by_id("plugin.info", p)?;
    let mut v = describe(&plugin);
    v["manifest"] = serde_json::to_value(plugin.manifest()).unwrap_or(Value::Null);
    v["defaults"] = Value::Object(plugin.manifest().defaults());
    v["lastError"] = json!(effect::last_error(plugin.id()));
    Ok(v)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "id").ok_or_else(|| bad("plugin.remove", "missing `id`"))?;
    // Installed again, it asks again.
    registry::revoke(id);
    s.store_grant(id, None);
    s.unstore_plugin(id);
    if registry::remove(id) { Ok(json!({"removed": id})) } else { Err(err(vectorcraft_plugins::Error::NotFound(id.into()))) }
}

/// This module's commands: `plugin.list`, `.info`, `.run`, `.install`, `.inspect`, `.remove` and
/// `.reload`.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "plugin.list",
            "List Plug-ins",
            [],
            None,
            "{} → {plugins: [{id, name, version, kind: \"filter\"|\"effect\"|\"exporter\"|\"importer\", author, description, params (doc), paramsSchema, effect? (the effect id of a live effect: plugin.<id>), format? and extensions? (an exporter's or importer's format id, plugin.<id>, for document.export or document.open's importer, and its file extensions), budget?: \"heavy\" (a filter that may compute for longer, see plugin.install's allowHeavy), size, source, permissions: [name] (what it may do: the ones its manifest lists, else the ones its kind implies; progress: it may report its progress, see Cancel and write log lines), hash (the module's SHA-256), api (the plug-in API level it was built for, 1 or 2)}], folder: the Additional Plug-ins Folder load report {folder, loaded, failed: [{file, error}]} | null}",
            always,
            list
        ),
        cmd!(
            query "plugin.info",
            "Plug-in Info",
            [],
            None,
            "{id} → plugin.list's entry plus {manifest, defaults, lastError (a live effect's last failure, null)}",
            always,
            info
        ),
        cmd!(
            "plugin.run",
            "Run Plug-in",
            [],
            None,
            "{id, params?: {…} (the plug-in's parameters; or give them as top-level keys; missing ones take their defaults, see plugin.info), ids?: [..] (default: the selection), …the plug-in's parameters as top-level keys} run an object filter on the selected paths and compound paths (inside selected groups and layers too; hidden, locked, guide and clipping paths are left out): it may change, delete or add objects, as one undo step; the result is selected, except the objects it locked or made guides. With nothing selected it gets no objects (generators add theirs to the current layer) → {plugin, ids}",
            has_doc,
            run
        ),
        cmd!(
            query "plugin.install",
            "Install Plug-in",
            [],
            None,
            "{path: a .wasm file (desktop) | dataBase64: the module (+ name?: shown as its source), replace?: bool (true: a plug-in with the same id is replaced), name?: file name (with dataBase64), allowHeavy?: bool (true once the user agreed to a heavy budget: a filter whose manifest asks for one, budget \"heavy\" in plugin.inspect, installs only with it), grant?: [permission] (the permissions the user granted: a plug-in whose manifest lists permissions, needsApproval in plugin.inspect, installs only when grant holds every one of them; the grant is recorded with the module's hash, and replacing the plug-in with one that lists no new permission needs none; with a plug-in store attached, only the grants kept there count), hash?: string (the module's SHA-256 as plugin.inspect gave it: a module with other bytes is refused)} validate and install a WebAssembly plug-in for this session → its plugin.list entry",
            always,
            install
        ),
        cmd!(
            query "plugin.inspect",
            "Inspect Plug-in",
            [],
            None,
            "{path: a .wasm file (desktop) | dataBase64: the module, name?: the file name shown as its source (with dataBase64)} validate a WebAssembly plug-in without installing it → the plugin.list entry it would have, plus needsApproval: bool (true when it lists permissions the user hasn't granted its id: plugin.install then needs grant)",
            always,
            inspect
        ),
        cmd!(query "plugin.remove", "Remove Plug-in", [], None, "{id} uninstall a plug-in (its live effects then leave the geometry as it is) → {removed}", always, remove),
        cmd!(
            query "plugin.reload",
            "Reload Plug-ins",
            ["Object", "Plug-ins"],
            None,
            "{path?: a folder (default: the Additional Plug-ins Folder preference)} install every *.wasm in it (desktop; a bad module is reported and skipped, and so are a heavy filter and a plug-in that lists permissions the user hasn't granted its id) → {folder, loaded: [id], failed: [{file, error}]}",
            always,
            reload
        ),
    ]
}
