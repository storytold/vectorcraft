//! `plugin.*` commands: sandboxed WebAssembly plug-ins (`vectorcraft-plugins`).
//!
//! Plug-ins are installed per process (`plugin.install`, or every `*.wasm` in the Additional
//! Plug-ins Folder preference) and listed with `plugin.list` / `plugin.info`. Object filters run
//! with `plugin.run` on the selected paths and compound paths (one undo step, transactional: a
//! plug-in that fails leaves the document untouched); live-effect plug-ins are applied like any
//! effect (`effect.apply {effect: "plugin.<id>"}`) and drawn by `vectorcraft-effects`.

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};
use vectorcraft_plugins::{Kind, Plugin, effect, objects, registry};

use super::*;

/// What a plug-in error looks like to the user.
fn err(e: vectorcraft_plugins::Error) -> EngineError {
    EngineError::Other(e.to_string())
}

/// JSON description of an installed plug-in.
pub fn describe(p: &Plugin) -> Value {
    let m = p.manifest();
    let mut v = json!({
        "id": m.id, "name": m.name, "version": m.version, "kind": m.kind.name(), "author": m.author, "description": m.description,
        "params": m.params_doc(), "paramsSchema": serde_json::to_value(m).ok().and_then(|v| v.get("params").cloned()),
        "size": p.size(), "source": p.source(),
    });
    if m.kind == Kind::Effect {
        v["effect"] = json!(effect::effect_id(&m.id));
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

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.run";
    let plugin = plugin_by_id(C, p)?;
    let m = plugin.manifest();
    if m.kind != Kind::Filter {
        return Err(bad(C, format!("`{}` is a live effect: apply it with effect.apply {{\"effect\": \"{}\"}}", m.id, effect::effect_id(&m.id))));
    }
    // Bad parameters fail before anything runs.
    let mut params = Value::Object(m.resolve_params(&plugin_params(p)).map_err(|e| bad(C, e.to_string()))?);
    let st = s.doc()?;
    let selected = ids_param(p, "ids").unwrap_or_else(|| st.selection.objects.clone());
    let inputs = objects::filter_targets(&st.doc, &selected);
    let parent = st.insertion_parent();
    let bounds = st.doc.bounds_of(&inputs, false);
    let board = bounds.and_then(|b| st.doc.artboard_at(b.center())).and_then(|i| st.doc.artboards.get(i)).or(st.doc.artboards.first());
    let rect = |r: vectorcraft_geom::Rect| json!([r.x0, r.y0, r.x1, r.y1]);
    params["_context"] = json!({"mode": "filter", "bounds": bounds.map(rect), "artboard": board.map(|a| rect(a.rect))});
    let input = objects::encode_input(&st.doc, &inputs);
    let params = serde_json::to_vec(&params).map_err(|e| bad(C, e.to_string()))?;
    let out = objects::decode(&plugin.run(&input, &params).map_err(err)?).map_err(err)?;
    let ids = s.edit(m.name.trim_end_matches('…'), |d, sel| {
        let ids = objects::apply_output(d, &inputs, out, parent).map_err(err)?;
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    Ok(json!({"plugin": m.id, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// Largest base64 payload `plugin.install` accepts (the module limit, encoded).
const MAX_DATA_CHARS: usize = (32 << 20) / 3 * 4 + 4;

/// The `.wasm` extension plug-in files have (File › Open installs them).
pub const EXTS: &[&str] = &["wasm"];

fn install(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.install";
    let plugin = match (str_param(p, "path").filter(|s| !s.is_empty()), str_param(p, "dataBase64")) {
        (_, Some(data)) => {
            if data.len() > MAX_DATA_CHARS {
                return Err(bad(C, "`dataBase64` is larger than the module limit"));
            }
            let bytes = vectorcraft_format::base64_decode(data).ok_or_else(|| bad(C, "`dataBase64` is not valid base64"))?;
            let plugin = Plugin::load(&bytes, vectorcraft_plugins::Limits::default()).map_err(err)?;
            match str_param(p, "name") {
                Some(name) => plugin.with_source(name),
                None => plugin,
            }
        }
        (Some(path), None) => load_path(path)?,
        (None, None) => return Err(bad(C, "give `path` (a .wasm file) or `dataBase64` (the module)")),
    };
    if !p.get("replace").and_then(Value::as_bool).unwrap_or(true) && registry::get(plugin.id()).is_some() {
        return Err(bad(C, format!("a plug-in with id {:?} is already installed", plugin.id())));
    }
    Ok(describe(&registry::install(plugin)))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_path(path: &str) -> Result<Plugin> {
    crate::file_access::check_read(path).map_err(EngineError::Other)?;
    registry::load_file(std::path::Path::new(path), vectorcraft_plugins::Limits::default()).map_err(err)
}

#[cfg(target_arch = "wasm32")]
fn load_path(_: &str) -> Result<Plugin> {
    Err(EngineError::Other("installing from a path is not available on the web; pass the module as `dataBase64`".into()))
}

/// The plug-in folder last loaded from the preferences, and what happened.
static FOLDER: Mutex<Option<(String, Value)>> = Mutex::new(None);

/// Loads the Additional Plug-ins Folder preference (`*.wasm`) when it is set and has changed since
/// the last load. Runs whenever preferences are applied (startup, `prefs.set`). Native only.
pub(crate) fn sync_prefs(s: &Session) {
    let folder = s.prefs.plugins_folder.trim();
    if folder.is_empty() {
        return;
    }
    let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref().is_some_and(|(f, _)| f == folder) {
        return;
    }
    *last = Some((folder.to_string(), load_folder(folder)));
}

#[cfg(not(target_arch = "wasm32"))]
fn load_folder(folder: &str) -> Value {
    if let Err(e) = crate::file_access::check_read(folder) {
        return json!({"folder": folder, "error": e});
    }
    // A file in it may be a link to one elsewhere.
    match registry::load_folder_where(std::path::Path::new(folder), |f| crate::file_access::check_read(&f.to_string_lossy())) {
        Ok(r) => {
            json!({"folder": folder, "loaded": r.loaded, "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>()})
        }
        Err(e) => json!({"folder": folder, "error": e.to_string()}),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_folder(folder: &str) -> Value {
    json!({"folder": folder, "error": "plug-in folders are not available on the web"})
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.reload";
    let folder = match p.get("path") {
        Some(v) => v.as_str().map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "`path` must be a folder"))?.to_string(),
        None => s.prefs.plugins_folder.trim().to_string(),
    };
    if folder.is_empty() {
        return Err(bad(C, "no folder: pass `path` or set Preferences › Performance & Storage › Additional Plug-ins Folder"));
    }
    let report = load_folder(&folder);
    if let Some(e) = report.get("error").and_then(Value::as_str) {
        return Err(EngineError::Other(e.to_string()));
    }
    *FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder, report.clone()));
    Ok(report)
}

fn list(_: &mut Session, _: &Value) -> Result<Value> {
    let folder = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, r)| r.clone());
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

fn remove(_: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "id").ok_or_else(|| bad("plugin.remove", "missing `id`"))?;
    if registry::remove(id) { Ok(json!({"removed": id})) } else { Err(err(vectorcraft_plugins::Error::NotFound(id.into()))) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "plugin.list",
            "List Plug-ins",
            [],
            None,
            "{} → {plugins: [{id, name, version, kind: \"filter\"|\"effect\", author, description, params (doc), paramsSchema, effect? (the effect id of a live effect: plugin.<id>), size, source}], folder: the Additional Plug-ins Folder load report {folder, loaded, failed: [{file, error}]} | null}",
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
            "{id, params?: {…} (the plug-in's parameters; or give them as top-level keys; missing ones take their defaults, see plugin.info), ids?: [..] (default: the selection)} run an object filter on the selected paths and compound paths (inside selected groups and layers too; hidden, locked, guide and clipping paths are left out): it may change, delete or add objects, as one undo step; the result is selected. With nothing selected it gets no objects (generators add theirs to the current layer) → {plugin, ids}",
            has_doc,
            run
        ),
        cmd!(
            query "plugin.install",
            "Install Plug-in",
            [],
            None,
            "{path: a .wasm file (desktop) | dataBase64: the module (+ name?: shown as its source), replace?: bool (true: a plug-in with the same id is replaced)} validate and install a WebAssembly plug-in for this session → its plugin.list entry",
            always,
            install
        ),
        cmd!(query "plugin.remove", "Remove Plug-in", [], None, "{id} uninstall a plug-in (its live effects then leave the geometry as it is) → {removed}", always, remove),
        cmd!(
            query "plugin.reload",
            "Reload Plug-ins",
            ["Object", "Plug-ins"],
            None,
            "{path?: a folder (default: the Additional Plug-ins Folder preference)} install every *.wasm in it (desktop; a bad module is reported and skipped) → {folder, loaded: [id], failed: [{file, error}]}",
            always,
            reload
        ),
    ]
}
