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
//! packing or simulation). It installs only once a person agrees, which loading a folder doesn't
//! ask, so folders skip it; the app runs it in the background with Cancel ([`FilterRun`]).
//!
//! A plug-in whose manifest lists permissions installs once a person grants them (recorded with
//! the module's hash and kept in the plug-in store), and a filter does only what they allow:
//! without `doc.read` it is given no objects, without `doc.write` an output that changes the
//! document is refused, and without `state` it neither reads nor writes the data it keeps with
//! the document.
//!
//! `plugin.install {grant, allowHeavy}` takes effect only with an [`InstallApproval`], which no
//! command can set, or in a session started to accept them from requests
//! ([`Session::allow_plugin_grants_from_requests`]).
//!
//! A filter or a live effect may add menu commands of its own (its manifest's `commands`). Each
//! is the command `plugin.<plug-in id>.<command id>` while the plug-in is installed
//! ([`command_specs`], which [`super::find_command`] and the command listings read): under
//! Object › Plug-ins › the plug-in's name (Effect › Plug-ins for a live effect's), journaled so
//! actions record it, and run as a filter run of the plug-in ([`run_command`]) with the command's
//! id in `_context`, its own checked parameters and the plug-in's permissions, as one undo step.
//!
//! A plug-in of API level 2 that lists `progress` may import the host's `vc_progress`,
//! `vc_cancelled` and `vc_log` ([`vectorcraft_plugins::host`]): run inside a
//! [`vectorcraft_doc::cancel::scope`] (the app's heavy filters and background exports), its
//! progress goes to the scope's sink and it sees the scope's Cancel; its log lines go to the log
//! under its id. One that lists `settings` or `cache` may keep settings and a cache of its own
//! (`vc_settings_*`, `vc_cache_*`), in the plug-in store under its id ([`crate::store`]):
//! `plugin.clearCache` empties the cache, `plugin.list` and `plugin.info` give its size
//! (`cacheBytes`), replacing the plug-in keeps both and `plugin.remove` deletes them.
//!
//! A filter whose manifest lists `inputs` (plug-in API level 2) is also given the selected type,
//! images or symbol instances of those kinds, to read only: its output must return each of them
//! unchanged ([`objects::decode_output_with`]).
//!
//! A filter, or a live effect's command, whose manifest lists `context` (plug-in API level 3) is
//! also given the parts of the active document it names in `_context.document`, whether or not
//! anything is selected: the settings, artboards, layers and swatches, as plain values
//! ([`context`]).
//!
//! A filter may be written in JavaScript ([`vectorcraft_plugins::js`]): `plugin.install {path}`
//! takes its `.vcplugin` manifest and reads the `.js` file the manifest names beside it, and the
//! plug-in store keeps the two as one module.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, Effect, Node};
use vectorcraft_plugins::{Budget, Command, Kind, Plugin, effect, export, objects, registry, safe_mode};

use super::{CommandSpec, EngineError, NodeId, Result, Session, always, bad, cmd, has_doc, ids_param, str_param, switch_or};
use crate::store::{PluginLoad, StoreTurn};

use vectorcraft_plugins::permissions::{self, Grant, GrantToken, Permission};

mod context;
pub use context::{CONTEXT_FUEL_PER_ITEM, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS};

/// What a plug-in error looks like to the user. A run the caller cancelled changed nothing.
fn err(e: vectorcraft_plugins::Error) -> EngineError {
    match e {
        vectorcraft_plugins::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

/// Under automation read roots, a reply names a file or folder outside them by its name alone.
fn shown_file(path: &str) -> String {
    super::fileio::shown_path(path).into_owned()
}

/// A plug-in turned off for the next launch, or the one a launch stopped at, as a reply shows it: a
/// file outside the automation read roots is named by its file name.
fn shown_item(item: &safe_mode::Item) -> Value {
    let mut v = item.to_json();
    if let Some(file) = item.file.as_deref() {
        v["file"] = json!(shown_file(file));
    }
    v
}

/// JSON description of an installed plug-in.
pub fn describe(p: &Plugin) -> Value {
    let m = p.manifest();
    let mut v = json!({
        "id": m.id, "name": m.name, "version": m.version, "kind": m.kind.name(), "author": m.author, "description": m.description,
        "params": m.params_doc(), "paramsSchema": serde_json::to_value(m).ok().and_then(|v| v.get("params").cloned()),
        "size": p.size(), "source": p.source().map(shown_file),
        "permissions": permissions::names(&m.permissions()), "hash": p.hash(), "api": m.api,
    });
    match m.kind {
        Kind::Effect => v["effect"] = json!(effect::effect_id(&m.id)),
        Kind::Exporter | Kind::Importer => {
            v["format"] = json!(export::format_id(&m.id));
            v["extensions"] = json!(m.extensions);
            if m.place {
                v["place"] = json!(true);
            }
        }
        Kind::Filter => {}
    }
    if m.budget == Budget::Heavy {
        v["budget"] = json!("heavy");
    }
    if !m.inputs.is_empty() {
        v["inputs"] = json!(m.inputs);
    }
    if !m.context.is_empty() {
        v["context"] = json!(m.context);
    }
    if m.paint_output {
        v["paintOutput"] = json!(true);
    }
    if let Some(script) = p.script() {
        v["runtime"] = json!(m.runtime.name());
        v["script"] = json!(script);
    }
    if !m.command_allowlist.is_empty() {
        v["commandAllowlist"] = json!(m.command_allowlist);
    }
    if !m.commands.is_empty() {
        let commands: Vec<Value> = m
            .commands
            .iter()
            .map(|c| {
                let run = m.for_command(c);
                let mut entry = json!({"id": c.id, "command": command_id(&m.id, &c.id), "label": c.shown_label(), "params": run.params_doc(),
                       "paramsSchema": serde_json::to_value(&run).ok().and_then(|v| v.get("params").cloned())});
                if let Some(place) = &c.place {
                    let mut menu = json!({"path": place.path});
                    if let Some(after) = &place.after {
                        menu["after"] = json!(permissions::display_text(after, permissions::MAX_SHOWN_NAME));
                    }
                    if placed_menu(p, c).is_none() {
                        menu["placed"] = json!(false);
                    }
                    entry["menu"] = menu;
                }
                entry
            })
            .collect();
        v["commands"] = json!(commands);
    }
    v
}

/// The plug-in's own parameters from command params: `params` if given, else the top-level keys
/// minus the ones the command itself uses.
pub fn plugin_params(p: &Value) -> Value {
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
    /// The plug-in command it runs (`plugin.<id>.<command>`), if it isn't `plugin.run`'s run.
    command: Option<Command>,
    input: Vec<u8>,
    params: Vec<u8>,
    /// The objects the plug-in is given, and the container new objects go in.
    inputs: Vec<NodeId>,
    /// The type, images and symbol instances among them (a filter that asks for them): its output
    /// must return each one as `input` gives it. These are their places in `input`.
    read_only: objects::ReadOnlySpans,
    parent: Option<NodeId>,
    /// The document it was made from, and that document's revision then.
    document: (u64, u64),
    /// The plug-in's limits with the Plug-in Instruction Budget in force when it was made.
    limits: vectorcraft_plugins::Limits,
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
        Self::prepare_with(s, plugin, None, C, p)
    }

    /// The run of `plugin`, or of its `command`, that command `c`'s params `p` ask for, on the
    /// active document. Bad parameters fail before anything runs.
    fn prepare_with(s: &Session, plugin: Arc<Plugin>, command: Option<Command>, c: &str, p: &Value) -> Result<FilterRun> {
        let m = plugin.manifest();
        let resolved = match &command {
            Some(cmd) => m.for_command(cmd).resolve_params(&plugin_params(p)),
            None => m.resolve_params(&plugin_params(p)),
        };
        let mut params = Value::Object(resolved.map_err(|e| bad(c, e.to_string()))?);
        let may = m.permissions();
        let st = s.doc()?;
        // While an object of a dynamic symbol instance's art is selected, the selection is refused,
        // as `selected_roots` refuses it.
        let selected = match ids_param(p, "ids", c)? {
            Some(ids) => ids,
            None => {
                super::edit::refuse_symbol_part(s)?;
                st.selection.objects.clone()
            }
        };
        // Without `doc.read` a filter is given no objects: it can only add new ones. Type, images
        // and symbol instances only when its manifest asks for them (`inputs`).
        let kinds = m.input_kinds();
        let inputs = if may.contains(&Permission::DocRead) { objects::filter_targets_with(&st.doc, &selected, &kinds) } else { Vec::new() };
        let parent = st.insertion_parent();
        let bounds = st.doc.bounds_of(&inputs, false);
        let board = bounds.and_then(|b| st.doc.artboard_at(b.center())).and_then(|i| st.doc.artboards.get(i)).or(st.doc.artboards.first());
        let rect = |r: vectorcraft_geom::Rect| json!([r.x0, r.y0, r.x1, r.y1]);
        // What the plug-in keeps with the document, as it last wrote it.
        let data = st.doc.plugin_data(&m.id).filter(|_| may.contains(&Permission::State)).cloned().unwrap_or(Value::Null);
        params["_context"] = json!({"mode": "filter", "bounds": bounds.map(rect), "artboard": board.map(|a| rect(a.rect)), "documentData": data});
        // A command run says which command it is.
        if let Some(cmd) = &command {
            params["_context"]["mode"] = json!("command");
            params["_context"]["command"] = json!(cmd.id);
        }
        // The parts of the document its manifest asks for (`context`), selection or not. Each row
        // costs the run [`context::CONTEXT_FUEL_PER_ITEM`] instructions.
        let sections = m.context_sections();
        let mut charged = 0;
        if !sections.is_empty() && may.contains(&Permission::DocRead) {
            let given = context::document_context(st, &sections, c)?;
            params["_context"]["document"] = given.value;
            charged = (given.items as u64).saturating_mul(context::CONTEXT_FUEL_PER_ITEM);
        }
        let params = serde_json::to_vec(&params).map_err(|e| bad(c, e.to_string()))?;
        // The input and the parameters may use half the plug-in's memory together.
        let capacity = (plugin.limits().max_memory_bytes / 2).min(i32::MAX as usize).saturating_sub(params.len());
        let (input, read_only) = objects::encode_input_with(&st.doc, &inputs, &kinds, capacity).map_err(err)?;
        let document = (st.uid, st.revision);
        let mut limits = match s.prefs.plugin_instruction_budget {
            crate::PluginBudget::Normal => plugin.limits().clone(),
            crate::PluginBudget::Large => plugin.large_limits(),
        };
        limits.fuel_base = limits.fuel_base.saturating_sub(charged);
        Ok(FilterRun { plugin, command, input, params, inputs, read_only, parent, document, limits })
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
        self.plugin.run_limited(&self.input, &self.params, &self.limits).map_err(err)
    }

    /// The limits it runs under: the plug-in's, with the Plug-in Instruction Budget in force when
    /// it was made, less what its document context costs.
    pub fn limits(&self) -> &vectorcraft_plugins::Limits {
        &self.limits
    }

    /// The parameters it hands the plug-in, `_context` included.
    #[cfg(test)]
    pub(crate) fn params_json(&self) -> Value {
        serde_json::from_slice(&self.params).unwrap_or(Value::Null)
    }

    /// Does the plug-in call commands (its manifest's `commandAllowlist`)? Then it runs only in the
    /// session ([`FilterRun::run_with_calls`]).
    pub fn calls(&self) -> bool {
        !self.plugin.manifest().command_allowlist.is_empty()
    }

    /// Runs the plug-in in `s`, making the command calls it asks for on the active document
    /// ([`super::plugin_call::call`]) → its output.
    fn run_calling(&self, s: &mut Session) -> Result<Vec<u8>> {
        let may = self.plugin.manifest().permissions();
        let calls = super::plugin_call::Calls::of(s, may.contains(&Permission::DocRead), may.contains(&Permission::DocWrite));
        let mut bridge = |id: &str, args: &Value| super::plugin_call::call(s, &calls, id, args);
        self.plugin.run_calling(&self.input, &self.params, &self.limits, &mut bridge).0.map_err(err)
    }

    /// Runs a plug-in that calls commands and applies its output: the calls and the output are one
    /// undo step, and a run that fails changes nothing ([`super::plugin_call::transaction`]).
    fn run_with_calls(self, s: &mut Session) -> Result<Value> {
        let label = self.label();
        super::plugin_call::transaction(s, &label, |s| {
            let output = self.run_calling(s)?;
            self.finish(s, &output)
        })
    }

    /// The name of its undo step: the plug-in's, or its command's.
    fn label(&self) -> String {
        let m = self.plugin.manifest();
        self.command.as_ref().map_or_else(|| m.name.trim_end_matches('…').to_string(), Command::shown_label)
    }

    /// Hands in `output`, which [`FilterRun::run`] returned: the next `plugin.run` that makes this
    /// same run (same plug-in, `ids` and parameters, the document unchanged) applies it instead of
    /// running the plug-in. Any other run drops it.
    pub fn hand_in(self, s: &mut Session, output: Vec<u8>) {
        s.plugin_output = Some(HandedIn { serial: self.plugin.serial(), input: self.input, params: self.params, output });
    }

    /// Applies the plug-in's `output` to the active document as one undo step: its objects, and what
    /// it keeps with the document from now on. An output its permissions don't allow, and one that
    /// changes or leaves out a read-only object it was given, is refused, and the document stays as
    /// it was. An output that changes neither the objects nor the kept data records no undo step and
    /// leaves the document unmodified; its objects are still selected.
    fn finish(self, s: &mut Session, output: &[u8]) -> Result<Value> {
        let m = self.plugin.manifest();
        let may = m.permissions();
        let out = objects::decode_output_with(output, &self.input, &self.read_only).map_err(err)?;
        if out.document_data.is_some() && !may.contains(&Permission::State) {
            return Err(not_granted(&m.name, Permission::State));
        }
        let may_write = may.contains(&Permission::DocWrite);
        // A command's undo step is named after the command.
        let label = self.label();
        let (inputs, parent) = (self.inputs, self.parent);
        let notice = out.notice;
        // The output is applied to a copy of the document, as `colorcmds::edit_if_changed` applies
        // its pass: only a copy whose objects or kept data differ becomes the document, as one undo
        // step. Otherwise the document, its undo and redo steps and its saved state stay as they
        // were, and only the selection is set.
        if s.script_running() {
            return Err(EngineError::Busy);
        }
        if !s.previewing && super::typing_in_progress(s) {
            super::settle_interaction(s)?;
        }
        let now = s.doc()?.doc.clone();
        let mut d = (*now).clone();
        let before = (!may_write).then(|| d.layers.clone());
        let ids = objects::apply_output(&mut d, &inputs, out.objects, parent, &admit_effects).map_err(err)?;
        if before.is_some_and(|b| b != d.layers) {
            return Err(not_granted(&m.name, Permission::DocWrite));
        }
        if let Some(data) = out.document_data {
            d.set_plugin_data(&m.id, data);
        }
        let selectable = |d: &Document| ids.iter().copied().filter(|&id| d.node(id).is_some_and(objects::selectable)).collect::<Vec<_>>();
        if same_art(&d.layers, &now.layers) && d.plugin_data(&m.id) == now.plugin_data(&m.id) {
            let st = s.doc()?;
            let mut sel = st.selection.clone();
            sel.set(selectable(&st.doc));
            if sel != st.selection {
                s.select(|_, selection| *selection = sel)?;
            }
        } else {
            // The document is shared with `now` no longer, so `s.edit` does not copy it again.
            drop(now);
            s.edit(&label, |doc, sel| {
                *doc = d;
                sel.set(selectable(doc));
                Ok(())
            })?;
        }
        let mut out = json!({"plugin": m.id, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()});
        if let Some(cmd) = &self.command {
            out["command"] = json!(cmd.id);
        }
        // Only an output that was applied reports its notice, with the name of the plug-in that ran.
        if let Some(notice) = notice {
            out["notice"] = json!(notice);
            out["pluginName"] = json!(m.name);
        }
        Ok(out)
    }
}

/// Whether layer lists `a` and `b` hold the same art: a layer that is shared is not compared again.
fn same_art(a: &[Arc<Node>], b: &[Arc<Node>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y) || x == y)
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
/// is one ([`FilterRun::hand_in`]), else the plug-in's, run now. → `{plugin, ids, notice?,
/// pluginName?}`: what it made or changed, which is then selected, and the output's `notice` once it
/// was applied, with the plug-in's name.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    // An output handed in is for this run only (the app hands it in just before), whatever it does.
    let handed = s.plugin_output.take();
    let r = FilterRun::prepare(s, p)?;
    // One that calls commands runs here, where it can make them, on the document with any edit in
    // progress committed.
    if r.calls() {
        let r = if super::plugin_call::settle(s)? { FilterRun::prepare(s, p)? } else { r };
        return r.run_with_calls(s);
    }
    // Handed in for this very run (the app ran it in the background), it is what running the
    // plug-in again would return.
    let handed = handed.filter(|h| (h.serial, &h.input, &h.params) == (r.plugin.serial(), &r.input, &r.params));
    let output = match handed {
        Some(h) => h.output,
        None => r.run()?,
    };
    r.finish(s, &output)
}

/// The command id of plug-in `plugin`'s command `command`: `plugin.<plugin>.<command>`.
pub fn command_id(plugin: &str, command: &str) -> String {
    format!("{}{plugin}.{command}", vectorcraft_plugins::PREFIX)
}

/// Is `id` shaped as a plug-in command's id (`plugin.<plug-in id>.<command id>`)? This module's
/// own `plugin.*` commands have no dot after the prefix, and a command id has none.
pub fn is_command_id(id: &str) -> bool {
    id.strip_prefix(vectorcraft_plugins::PREFIX).is_some_and(|rest| rest.contains('.'))
}

/// The installed plug-in and its command that command id `id` names.
pub fn command_of(id: &str) -> Option<(Arc<Plugin>, Command)> {
    let (plugin, command) = id.strip_prefix(vectorcraft_plugins::PREFIX)?.rsplit_once('.')?;
    let p = registry::get(plugin)?;
    let c = p.manifest().command(command)?.clone();
    Some((p, c))
}

/// The plug-in commands of the installed plug-ins, the registry revision they were listed at, and
/// a count that changes whenever the list does ([`commands_generation`]).
struct Listed {
    revision: Option<u64>,
    generation: u64,
    specs: Vec<&'static CommandSpec>,
}

static LISTED: RwLock<Listed> = RwLock::new(Listed { revision: None, generation: 0, specs: Vec::new() });

/// Every command spec made for a plug-in command, made once: specs hold static strings, so memory
/// stays bounded by the plug-in commands ever installed.
static MADE: Mutex<Vec<&'static CommandSpec>> = Mutex::new(Vec::new());

/// The commands the installed plug-ins add, in plug-in id order and then in each manifest's order.
pub fn command_specs() -> Vec<&'static CommandSpec> {
    let revision = registry::revision();
    if let Ok(l) = LISTED.read()
        && l.revision == Some(revision)
    {
        return l.specs.clone();
    }
    let specs: Vec<&'static CommandSpec> =
        registry::list().iter().flat_map(|p| p.manifest().commands.iter().map(|c| spec_of(p, c)).collect::<Vec<_>>()).collect();
    // When a plug-in was installed or removed during the listing, the list may not match
    // `revision`. It is returned without being cached, and the next call lists again.
    if registry::revision() != revision {
        return specs;
    }
    let mut l = LISTED.write().unwrap_or_else(|e| e.into_inner());
    if l.specs.len() != specs.len() || l.specs.iter().zip(&specs).any(|(a, b)| !std::ptr::eq(*a, *b)) {
        l.generation += 1;
    }
    l.revision = Some(revision);
    l.specs.clone_from(&specs);
    specs
}

/// Changes whenever the plug-in commands [`command_specs`] lists change, for caches built from
/// them (the keyboard shortcuts' catalogue).
pub fn commands_generation() -> u64 {
    command_specs();
    LISTED.read().map(|l| l.generation).unwrap_or_else(|e| e.into_inner().generation)
}

/// The spec of installed plug-in command `id`.
pub fn find_spec(id: &str) -> Option<&'static CommandSpec> {
    if !is_command_id(id) {
        return None;
    }
    command_specs().into_iter().find(|c| c.id == id)
}

/// The letters and digits of `label`, lowercased. Two labels with the same key read alike in a menu
/// (`Paste (in Place)` and `Paste in Place`).
fn label_key(label: &str) -> String {
    label.chars().filter(|ch| ch.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The labels of engine commands whose menu item switches to the label of [`other_state`] (the
/// desktop app's dynamic labels).
pub const SWITCHING_LABELS: &[&str] = &[
    "Preview",
    "Outline",
    "Show Rulers",
    "Change to Global Rulers",
    "Show Transparency Grid",
    "Lock Guides",
    "Hide Slices",
    "Show Print Tiling",
    "Lock Grid",
    "Edit Envelope",
    "Edit Contents",
    "Start Global Edit",
];

/// The label of `label`'s other state, for a menu item that switches between two labels: `Show
/// Edges` for `Hide Edges`, `Unlock Guides` for `Lock Guides`, `Stop Global Edit` for `Start Global
/// Edit`, and the reverse.
pub fn other_state(label: &str) -> Option<String> {
    const PAIRS: &[(&str, &str)] = &[
        ("Preview", "Outline"),
        ("Change to Global Rulers", "Change to Artboard Rulers"),
        ("Edit Envelope", "Edit Contents"),
        ("Start Global Edit", "Stop Global Edit"),
    ];
    let paired = |&(a, b): &(&'static str, &'static str)| {
        if label == a {
            Some(b)
        } else if label == b {
            Some(a)
        } else {
            None
        }
    };
    if let Some(other) = PAIRS.iter().find_map(paired) {
        return Some(other.to_string());
    }
    [("Hide ", "Show "), ("Show ", "Hide "), ("Lock ", "Unlock "), ("Unlock ", "Lock ")]
        .iter()
        .find_map(|(from, to)| label.strip_prefix(from).map(|rest| format!("{to}{rest}")))
}

/// The menu that plug-in `p`'s command `c` is listed in when its manifest's `menu` places it in
/// another menu: that menu's path, or `None` when `c` stays in its plug-in's submenu. It stays
/// there when its menu label ([`Command::menu_label`]) has the same letters and digits, ignoring
/// case, as an engine command listed in that menu (either label of one of [`SWITCHING_LABELS`]),
/// an item of [`DESKTOP_MENU_ITEMS`] there, or a command that a plug-in with a lower id, or an
/// earlier command of `p`, places in that menu. The result is the same with and without the
/// desktop app. Results for installed plug-ins are kept until the registry changes.
pub fn placed_menu(p: &Plugin, c: &Command) -> Option<&'static [&'static str]> {
    type Placed = HashMap<String, Option<&'static [&'static str]>>;
    static PLACED: Mutex<Option<(u64, Placed)>> = Mutex::new(None);
    c.place.as_ref()?;
    let m = p.manifest();
    // Read before the registry, so a result found while the registry changes is kept under the
    // revision before the change.
    let revision = registry::revision();
    // `plugin.inspect` describes a module that is not installed, or not yet in place of the
    // installed one: never kept.
    if !registry::get(&m.id).is_some_and(|q| std::ptr::eq(&*q, p)) {
        return find_place(p, c);
    }
    let id = command_id(&m.id, &c.id);
    if let Ok(placed) = PLACED.lock()
        && let Some((at, known)) = placed.as_ref()
        && *at == revision
        && let Some(found) = known.get(&id)
    {
        return *found;
    }
    let found = find_place(p, c);
    if let Ok(mut placed) = PLACED.lock() {
        let (at, known) = placed.get_or_insert_with(|| (revision, HashMap::new()));
        if *at != revision {
            *at = revision;
            known.clear();
        }
        known.insert(id, found);
    }
    found
}

/// [`placed_menu`], found again.
fn find_place(p: &Plugin, c: &Command) -> Option<&'static [&'static str]> {
    let place = c.place.as_ref()?;
    let m = p.manifest();
    let key = label_key(&c.menu_label(&m.name));
    let reads_like =
        |label: &str| label_key(label) == key || SWITCHING_LABELS.contains(&label) && other_state(label).is_some_and(|o| label_key(&o) == key);
    if super::command_specs().iter().any(|s| s.menu == place.path && reads_like(s.label))
        || DESKTOP_MENU_ITEMS.iter().any(|(path, label)| *path == place.path && label_key(label) == key)
    {
        return None;
    }
    let taken = |name: &str, d: &Command| d.place.as_ref().is_some_and(|e| e.path == place.path) && label_key(&d.menu_label(name)) == key;
    // The registry lists plug-ins in id order. `p` itself may not be installed yet (`plugin.inspect`),
    // or be replacing the installed module of its id, so its own commands come from `m`.
    for q in registry::list() {
        let qm = q.manifest();
        if qm.id >= m.id {
            break;
        }
        if qm.commands.iter().any(|d| taken(&qm.name, d)) {
            return None;
        }
    }
    for d in &m.commands {
        if d.id == c.id {
            return Some(place.path);
        }
        if taken(&m.name, d) {
            return None;
        }
    }
    Some(place.path)
}

/// The labels of the desktop app's own items (not engine commands) in the menus of
/// [`vectorcraft_plugins::manifest::MENU_PLACES`], with both labels of an item that switches
/// between two. [`placed_menu`] reads them, so its result is the same with and without the desktop
/// app.
pub const DESKTOP_MENU_ITEMS: &[(&[&str], &str)] = &[
    (&["Edit"], "Spelling"),
    (&["Edit"], "Edit Custom Dictionary…"),
    (&["Edit"], "Edit Colors"),
    (&["Edit"], "Edit Original"),
    (&["Edit"], "Transparency Flattener Presets…"),
    (&["Edit"], "Print Presets…"),
    (&["Edit"], "PDF Presets…"),
    (&["Edit"], "Tracing Presets…"),
    (&["Edit"], "Perspective Grid Presets…"),
    (&["Edit"], "Refresh Monitor Profile"),
    (&["Edit"], "Keyboard Shortcuts…"),
    (&["Edit"], "Preferences…"),
    (&["Edit", "Edit Colors"], "Recolor with Preset"),
    (&["Object"], "Transform"),
    (&["Object"], "Arrange"),
    (&["Object"], "Align"),
    (&["Object"], "Distribute"),
    (&["Object"], "Lock"),
    (&["Object"], "Hide"),
    (&["Object"], "Slice"),
    (&["Object"], "Path"),
    (&["Object"], "Shape"),
    (&["Object"], "Pattern"),
    (&["Object"], "Repeat"),
    (&["Object"], "Intertwine"),
    (&["Object"], "Objects on Path"),
    (&["Object"], "Blend"),
    (&["Object"], "Envelope Distort"),
    (&["Object"], "Perspective"),
    (&["Object"], "Live Paint"),
    (&["Object"], "Image Trace"),
    (&["Object"], "Text Wrap"),
    (&["Object"], "Clipping Mask"),
    (&["Object"], "Compound Path"),
    (&["Object"], "Artboards"),
    (&["Object"], "Graph"),
    (&["Object"], "Collect for Export"),
    (&["Object"], "Plug-ins"),
    (&["Type"], "Font"),
    (&["Type"], "Recent Fonts"),
    (&["Type"], "Size"),
    (&["Type"], "Bold"),
    (&["Type"], "Italic"),
    (&["Type"], "Glyphs"),
    (&["Type"], "Insert Special Character"),
    (&["Type"], "Insert Whitespace Character"),
    (&["Type"], "Insert Break Character"),
    (&["Type"], "Type on a Path"),
    (&["Type"], "Dynamic Text Presets"),
    (&["Type"], "Threaded Text"),
    (&["Type"], "Find Font…"),
    (&["Type"], "Change Case"),
    (&["Type"], "Bullets and Numbering"),
    (&["Type"], "Show Hidden Characters"),
    (&["Type"], "Hide Hidden Characters"),
    (&["Type"], "Type Orientation"),
    (&["Select"], "Same"),
    (&["Select"], "Object"),
    (&["Select"], "Saved Selection"),
    (&["Effect"], "Apply Last Effect"),
    (&["Effect"], "Last Effect…"),
    (&["Effect"], "3D and Materials"),
    (&["Effect"], "Color Adjustments"),
    (&["Effect"], "Color Effects"),
    (&["Effect"], "Convert to Shape"),
    (&["Effect"], "Crop Marks"),
    (&["Effect"], "Distort & Transform"),
    (&["Effect"], "Path"),
    (&["Effect"], "Path Effects"),
    (&["Effect"], "Pathfinder"),
    (&["Effect"], "Rasterize…"),
    (&["Effect"], "Stylize"),
    (&["Effect"], "SVG Filters"),
    (&["Effect"], "Warp"),
    (&["Effect"], "Effect Gallery…"),
    (&["Effect"], "Artistic"),
    (&["Effect"], "Blur"),
    (&["Effect"], "Brush Strokes"),
    (&["Effect"], "Distort"),
    (&["Effect"], "Pixelate"),
    (&["Effect"], "Sharpen"),
    (&["Effect"], "Sketch"),
    (&["Effect"], "Texture"),
    (&["Effect"], "Video"),
    (&["Effect"], "Plug-ins"),
    (&["View"], "Outline"),
    (&["View"], "Preview"),
    (&["View"], "Pixel Preview"),
    (&["View"], "Trim View"),
    (&["View"], "Presentation Mode"),
    (&["View"], "Screen Mode"),
    (&["View"], "Proof Setup"),
    (&["View"], "Zoom In"),
    (&["View"], "Zoom Out"),
    (&["View"], "Fit Artboard in Window"),
    (&["View"], "Fit All in Window"),
    (&["View"], "Actual Size"),
    (&["View"], "Rotate View"),
    (&["View"], "Rotate View to Selection"),
    (&["View"], "Reset Rotate View"),
    (&["View"], "Hide Edges"),
    (&["View"], "Show Edges"),
    (&["View"], "Hide Artboards"),
    (&["View"], "Show Artboards"),
    (&["View"], "Hide Template"),
    (&["View"], "Show Template"),
    (&["View"], "Rulers"),
    (&["View"], "Hide Bounding Box"),
    (&["View"], "Show Bounding Box"),
    (&["View"], "Hide Text Threads"),
    (&["View"], "Show Text Threads"),
    (&["View"], "Hide Gradient Annotator"),
    (&["View"], "Show Gradient Annotator"),
    (&["View"], "Hide Corner Widget"),
    (&["View"], "Show Corner Widget"),
    (&["View"], "Show Live Paint Gaps"),
    (&["View"], "Hide Live Paint Gaps"),
    (&["View"], "Guides"),
    (&["View"], "Smart Guides"),
    (&["View"], "Perspective Grid"),
    (&["View"], "Show Grid"),
    (&["View"], "Hide Grid"),
    (&["View"], "Snap to Grid"),
    (&["View"], "Snap to Pixel"),
    (&["View"], "Snap to Point"),
    (&["View"], "Snap to Glyph"),
    (&["View"], "Saved View"),
];

/// The command spec of plug-in `p`'s command `c`: its id, its label as menus show it (escaped and
/// capped, and followed by the plug-in's name when it is listed in another menu), its menu
/// ([`placed_menu`], else Object › Plug-ins › the plug-in's name, or Effect › Plug-ins › it for a
/// live effect) and its params description.
fn spec_of(p: &Plugin, c: &Command) -> &'static CommandSpec {
    let m = p.manifest();
    let id = command_id(&m.id, &c.id);
    let placed = placed_menu(p, c);
    let label = if placed.is_some() { c.menu_label(&m.name) } else { c.shown_label() };
    let top = if m.kind == Kind::Effect { "Effect" } else { "Object" };
    let name = permissions::display_text(m.name.trim(), permissions::MAX_SHOWN_NAME);
    let submenu = [top, "Plug-ins", name.as_str()];
    let path: &[&str] = placed.unwrap_or(&submenu);
    let mut fields =
        String::from("ids?: [object id] (left out or null: the selection), params?: {…} (its parameters; or give them as top-level keys)");
    let doc = m.for_command(c).params_doc();
    if let Some(own) = doc.strip_prefix('{').and_then(|d| d.strip_suffix('}')).filter(|d| !d.is_empty()) {
        fields.push_str(", ");
        fields.push_str(own);
    }
    let params = format!(
        "{{{fields}}} run “{label}”, a command of plug-in {}, on the selected paths and compound paths as plugin.run runs a filter, with _context.command {:?}: one undo step, none when the run changes nothing → {{plugin, command, ids, notice?, pluginName?}} (pluginName, the plug-in's name, comes with notice)",
        m.id, c.id
    );
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    let same = |s: &&&'static CommandSpec| s.id == id && s.label == label && s.menu == path && s.params == params;
    if let Some(s) = made.iter().find(same) {
        return s;
    }
    let menu: &'static [&'static str] = match placed {
        Some(path) => path,
        None => Box::leak(vec![top, "Plug-ins", registry::intern(&name)].into_boxed_slice()),
    };
    let spec: &'static CommandSpec = Box::leak(Box::new(CommandSpec {
        id: registry::intern(&id),
        label: registry::intern(&label),
        menu,
        shortcut: None,
        params: registry::intern(&params),
        enabled: has_doc,
        run: unrouted,
        journal: true,
        schema: None,
    }));
    made.push(spec);
    spec
}

/// A plug-in command's `run`: plug-in commands run through [`super::run_spec`], which passes
/// [`run_command`] the command id.
fn unrouted(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Internal { cmd: "plugin".into(), msg: "a plug-in command runs through Session::execute".into() })
}

/// Runs plug-in command `id` (`plugin.<plug-in id>.<command id>`): a filter run of the plug-in on
/// `ids` (else the selection) with the command's own parameters, checked before it runs, and
/// `_context.command`, under the plug-in's permissions and budget, applied as one undo step named
/// after the command → `{plugin, command, ids, notice?, pluginName?}`. A run that fails (a trap, an exhausted
/// budget, an output it may not make) leaves the document as it was, and the error names the
/// plug-in and the command.
pub(crate) fn run_command(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    // An output handed in is for `plugin.run` alone.
    s.plugin_output = None;
    let (plugin, command) = command_of(id).ok_or_else(|| EngineError::UnknownCommand(id.into()))?;
    let failed = {
        let (name, label) = (permissions::display_text(&plugin.manifest().name, permissions::MAX_SHOWN_NAME), command.shown_label());
        move |e: EngineError| match e {
            EngineError::Cancelled => EngineError::Cancelled,
            e => EngineError::Other(format!("“{name}” › {label}: {e}")),
        }
    };
    let r = FilterRun::prepare_with(s, plugin, Some(command), id, p)?;
    if r.calls() {
        let r = match super::plugin_call::settle(s)? {
            true => {
                let (plugin, command) = command_of(id).ok_or_else(|| EngineError::UnknownCommand(id.into()))?;
                FilterRun::prepare_with(s, plugin, Some(command), id, p)?
            }
            false => r,
        };
        return r.run_with_calls(s).map_err(failed);
    }
    let output = r.run().map_err(&failed)?;
    r.finish(s, &output).map_err(failed)
}

/// Admits the effect records a filter returned for object `n` (written, its effects still its own)
/// as `effect.apply` would apply them: each new record a known effect (one of ours, or an installed
/// live-effect plug-in) with parameters it takes, filled in to their defaults. A record the object
/// already had stays as it was. An object takes one 3D effect (its fills' and strokes' own
/// counted), which must fit it, and the `textureImage` of an effect with Load Texture (Glass,
/// Texturizer, Rough Pastels) comes as the effect stores it, never as a file to read: a plug-in
/// can't have the host read one.
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

/// The extensions plug-in files have (File › Open installs them): a WebAssembly module, and a
/// JavaScript plug-in's manifest.
pub const EXTS: &[&str] = &["wasm", vectorcraft_plugins::js::PACKAGE_EXT];

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

/// What a person approved for one install: the module they were shown (its id, SHA-256 and the
/// permissions it lists) and whether they agreed to its heavy budget. Only the code that asked the
/// person sets it ([`Session::approve_plugin_install`]); no command can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallApproval {
    pub id: String,
    pub hash: String,
    pub permissions: Vec<String>,
    pub heavy: bool,
}

impl InstallApproval {
    /// Is `plugin` the module this approval was given for?
    fn covers(&self, plugin: &Plugin) -> bool {
        let m = plugin.manifest();
        self.id == m.id && self.hash == plugin.hash() && self.permissions.iter().map(String::as_str).eq(permissions::names(&m.permissions()))
    }
}

impl Session {
    /// Lets the next `plugin.install` take its `grant` and `allowHeavy`, for the module `approval`
    /// describes. Call it only with what a person approved, just before that install.
    pub fn approve_plugin_install(&mut self, approval: InstallApproval) {
        self.plugin_install_approval = Some(approval);
    }

    /// Drops an approval that no install has spent.
    pub fn clear_plugin_install_approval(&mut self) {
        self.plugin_install_approval = None;
    }

    /// Makes `plugin.install` accept `grant` and `allowHeavy` from any request in this session.
    /// For the person starting the process (`vectorcraft-cli --allow-plugin-grants`), never for
    /// a request.
    pub fn allow_plugin_grants_from_requests(&mut self) {
        self.plugin_grants_from_requests = true;
    }
}

/// Would installing `plugin` take another module's place: one installed with its id and other
/// bytes, or, with none installed, the settings and cache the plug-in store keeps for its id?
fn takes_over(s: &Session, plugin: &Plugin) -> bool {
    match registry::get(plugin.id()) {
        Some(installed) => installed.hash() != plugin.hash(),
        None => s.has_plugin_storage(plugin.id()),
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: runs once in the next `plugin.install` on this thread after its checks and before it
    /// installs, as another session changing the plug-ins in between would.
    pub(crate) static BEFORE_INSTALL: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once in the next `plugin.install` on this thread right after it reads the grant
    /// held for the id.
    pub(crate) static AFTER_GRANT_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once in the next `plugin.install` on this thread after it installs and before it
    /// writes the store.
    pub(crate) static BEFORE_STORE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once on this thread when [`keep_installed`] has written the module to the store
    /// and before it writes the grant.
    pub(crate) static BEFORE_GRANT_STORED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once on this thread when [`hold_plugin_changes`] finds the lock held and waits.
    pub(crate) static WHILE_WAITING: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: runs once in the next `plugin.reload` on this thread after its load started and before
    /// it installs, as another session loading a folder in between would.
    pub(crate) static BEFORE_RELOAD_INSTALLS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// Held by `plugin.install` from its first read of the grant until the store holds what it
/// installed, throughout by `plugin.remove`, and while the loads of the stored plug-ins
/// ([`Session::load_stored_plugins`]) and of a plug-ins folder install what they loaded: these run
/// one after the other across sessions. It is not held while the modules load, so a session's
/// plug-in commands don't wait for another session's loads.
static PLUGIN_CHANGES: Mutex<()> = Mutex::new(());

/// Takes [`PLUGIN_CHANGES`], waiting while another session holds it.
pub(crate) fn hold_plugin_changes() -> std::sync::MutexGuard<'static, ()> {
    match PLUGIN_CHANGES.try_lock() {
        Ok(held) => held,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            #[cfg(test)]
            if let Some(waiting) = WHILE_WAITING.take() {
                waiting();
            }
            PLUGIN_CHANGES.lock().unwrap_or_else(|e| e.into_inner())
        }
    }
}

/// Takes [`PLUGIN_CHANGES`] when no other session holds it, without waiting.
pub(crate) fn try_hold_plugin_changes() -> Option<std::sync::MutexGuard<'static, ()>> {
    match PLUGIN_CHANGES.try_lock() {
        Ok(held) => Some(held),
        Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Tests: whether [`PLUGIN_CHANGES`] could be taken now.
#[cfg(test)]
pub(crate) fn plugin_changes_free() -> bool {
    !matches!(PLUGIN_CHANGES.try_lock(), Err(std::sync::TryLockError::WouldBlock))
}

/// `plugin.install`: loads and checks the module, refuses `grant` and `allowHeavy` that neither an
/// [`InstallApproval`] for this module nor the session allows, one that asks for a heavy budget
/// unless `allowHeavy` says a person agreed, one that lists permissions no person has granted this
/// very module unless `grant` lists them all, one that would take another module's place
/// ([`takes_over`]) unless `replace` is true, and one whose SHA-256 isn't the `hash` given. It then
/// checks the plug-in installed under the id and its grant again while it records the grant with
/// the module's hash and installs it ([`registry::install_checked`]), and only then keeps both for
/// the next start → its `plugin.list` entry. A refused install changes neither the plug-in store
/// nor the plug-ins and grants of the process. A module that lists no permissions ends any grant
/// held for its id. A `plugin.remove` in another session waits until the install has written the
/// store. With a plug-in store, all of it happens in a turn on the store ([`StoreTurn`]), so another
/// process using the same folder changes it before or after; when the store can't be locked, nothing
/// is installed.
fn install(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.install";
    // One use: spent whatever this install does.
    let approval = s.plugin_install_approval.take();
    let replace = match p.get("replace") {
        None | Some(Value::Null) => None,
        Some(_) => Some(switch_or(p, "replace", false, C)?),
    };
    let allow_heavy = switch_or(p, "allowHeavy", false, C)?;
    let grant = grant_param(C, p)?;
    let (plugin, bytes) = load_given(C, p)?;
    // `hash` pins the module the user was shown: one with other bytes is refused.
    if let Some(h) = p.get("hash").filter(|v| !v.is_null())
        && h.as_str() != Some(plugin.hash())
    {
        return Err(bad(C, "the module doesn't have the SHA-256 `hash` gives: nothing was installed"));
    }
    // A removal in another session waits until the store holds what this install wrote, and so
    // does a change of the store by another process.
    let _held = hold_plugin_changes();
    let turn =
        s.plugin_store_turn().map_err(|e| EngineError::Other(format!("{C}: {}: {e}: nothing was installed", crate::store::STORE_NOT_LOCKED)))?;
    // The grant the checks below count on, the approval included. The install checks it again,
    // with the plug-in installed under the id, before anything is stored: another session may
    // change either in between.
    let seen_grant = registry::granted(plugin.id());
    #[cfg(test)]
    if let Some(between) = AFTER_GRANT_READ.take() {
        between();
    }
    let installed_already = || bad(C, format!("a plug-in with id {:?} is already installed", plugin.id()));
    let taken_over = || {
        let name = permissions::display_text(&plugin.manifest().name, permissions::MAX_SHOWN_NAME);
        bad(
            C,
            format!(
                "“{name}” is another module than the plug-in with id {:?} and would get the settings and cache kept for it: install it with `replace: true` once the user agrees",
                plugin.id()
            ),
        )
    };
    if replace == Some(false) && registry::get(plugin.id()).is_some() {
        return Err(installed_already());
    }
    // Another module under the id gets the settings and cache kept for it: whoever installs it asks
    // the user first.
    if replace != Some(true) && takes_over(s, &plugin) {
        return Err(taken_over());
    }
    let (installed_already, taken_over) = (installed_already(), taken_over());
    // `grant` and `allowHeavy` say that a person agreed, and a request alone can't say that.
    let approval = approval.filter(|a| a.covers(&plugin));
    if !s.plugin_grants_from_requests && ((grant.is_some() && approval.is_none()) || (allow_heavy && !approval.is_some_and(|a| a.heavy))) {
        return Err(bad(
            C,
            "`grant` and `allowHeavy` take effect only in a session started with --allow-plugin-grants: approve the plug-in in the desktop app",
        ));
    }
    // A plug-in never calls a command no plug-in may call, whatever its manifest lists.
    let (unknown, refused) = super::plugin_call::check_allowlist(plugin.manifest());
    if let Some(id) = refused.first() {
        let (name, id) = (
            permissions::display_text(&plugin.manifest().name, permissions::MAX_SHOWN_NAME),
            permissions::display_text(id, permissions::MAX_SHOWN_FIELD),
        );
        return Err(bad(C, format!("“{name}” lists `{id}` in `commandAllowlist`, which a plug-in may not call")));
    }
    let unknown: Vec<String> = unknown.into_iter().map(str::to_string).collect();
    // A heavy plug-in is installed once a person agrees.
    if plugin.manifest().budget == Budget::Heavy && !allow_heavy {
        let (minutes, times) = vectorcraft_plugins::heavy_terms();
        return Err(bad(
            C,
            format!(
                "“{}” asks for a heavy budget (each run up to {minutes} minutes and {times} times the usual instructions): a person has to approve it in the desktop app, or pass `allowHeavy: true` in a session started with --allow-plugin-grants",
                plugin.manifest().name
            ),
        ));
    }
    // A plug-in that lists permissions installs once they are granted, unless a grant recorded for
    // this very module already covers all of them.
    let m = plugin.manifest();
    let wanted = m.permissions();
    let needs_approval = || {
        let name = permissions::display_text(&m.name, permissions::MAX_SHOWN_NAME);
        let asked = permissions::names(&wanted).join(", ");
        bad(
            C,
            format!(
                "“{name}” asks for the permissions {asked}: a person has to approve them in the desktop app, or pass `grant` in a session started with --allow-plugin-grants"
            ),
        )
    };
    let granted_here = grant.as_ref().is_some_and(|g| wanted.iter().all(|w| g.contains(w)));
    // Without a plug-in store, the grant read above approves the module or doesn't.
    let approved = match s.plugin_dir {
        None => !m.asks_permissions() || seen_grant.as_ref().is_some_and(|g| g.covers(&m.id, plugin.hash(), &wanted)),
        Some(_) => s.plugin_approved(&plugin),
    };
    if !approved && !granted_here {
        return Err(needs_approval());
    }
    let needs_approval = needs_approval();
    let (id, hash, asks) = (m.id.clone(), plugin.hash().to_string(), m.asks_permissions());
    let g = Grant { id: id.clone(), hash: hash.clone(), permissions: wanted };
    let update = if asks { registry::GrantUpdate::Record(GrantToken::after_engine_approval(g.clone())) } else { registry::GrantUpdate::Revoke };
    // Another module installed under the id since the checks above takes the place they found free,
    // or held by these bytes. A grant that changed since no longer approves what they approved,
    // unless `grant` approves this module by itself or the new grant covers it.
    let check = |installed: Option<&Plugin>, held: Option<&Grant>| {
        if let Some(installed) = installed {
            match replace {
                Some(false) => return Err(installed_already),
                None if installed.hash() != hash => return Err(taken_over),
                _ => {}
            }
        }
        if asks && !granted_here && held != seen_grant.as_ref() && !held.is_some_and(|h| h.covers(&id, &hash, &g.permissions)) {
            return Err(needs_approval);
        }
        Ok(())
    };
    #[cfg(test)]
    if let Some(between) = BEFORE_INSTALL.take() {
        between();
    }
    // The plug-in gets the settings and cache kept for its id. A module without permissions takes
    // the id over, so nothing the user approved for the id applies any more.
    let installed = registry::install_checked(plugin, update, check, || s.attach_plugin_storage(&turn, &id), err)?;
    #[cfg(test)]
    if let Some(between) = BEFORE_STORE.take() {
        between();
    }
    let stored = keep_installed(s, &turn, &id, &bytes, asks.then_some(g));
    // Installing a disabled plug-in again (an updated version) turns it on for the next launch too,
    // also when it is disabled by its stored file alone (it stopped a launch while loading). When
    // the new version could not replace the stored file, the old one stays disabled.
    let file = s.stored_plugin_file(&id);
    let mut changed = false;
    if file.is_none() || stored {
        changed = safe_mode::set_enabled(&safe_mode::Item::of(&installed), true);
        if let Some(file) = file {
            changed |= safe_mode::set_enabled(&safe_mode::Item::file(file), true);
        }
    }
    if changed {
        s.store_disabled_plugins_in(&turn);
    }
    drop(turn);
    let mut v = describe_installed(&installed);
    if !unknown.is_empty() {
        v["unknownCommands"] = json!(unknown);
    }
    Ok(v)
}

/// Keeps the plug-in `id` that `plugin.install` installed, its module `bytes`, in the plug-in store,
/// in `turn`, with `grant` when its module lists permissions (`None`: it lists none, and the grant
/// held for the id is removed) → whether the store now holds the new module (`false` without a
/// store).
///
/// Installed plug-ins stay installed after a restart (#2541). The module is written first and its
/// grant only once it is there; when the grant can't be written, the store gets the module it held
/// back. A failed write of that module is only logged: the store then holds the new module, which
/// the grant doesn't cover. A write that reports a failure may still have put the old module back,
/// so the store is read again to tell which module it holds. A stored module that can't be read back
/// isn't replaced by one that needs a grant.
pub(crate) fn keep_installed(s: &Session, turn: &StoreTurn, id: &str, bytes: &[u8], grant: Option<Grant>) -> bool {
    let asks = grant.is_some();
    let previous = s.stored_plugin_bytes(id);
    let mut stored = (previous.is_ok() || !asks) && s.store_plugin(turn, id, bytes);
    #[cfg(test)]
    if let Some(between) = BEFORE_GRANT_STORED.take() {
        between();
    }
    let holds_new = |s: &Session| s.stored_plugin_bytes(id).is_ok_and(|held| held.as_deref() == Some(bytes));
    match grant {
        Some(g) => {
            if stored && !s.store_grant_in(turn, id, Some(g)) {
                stored = !s.put_back_plugin(turn, id, previous.ok().flatten().as_deref()) && holds_new(s);
            }
        }
        None => {
            // When the write failed, the old stored module stays in the folder and needs its grant
            // at the next launch. A grant that can't be removed still approves the old bytes, so the
            // store gets the module it held back. When that module can't be read or written back, or
            // there was none, no module goes with the grant, and the grant stops counting for every
            // session in the process attached to the folder. A grants file that is too large or
            // doesn't hold a list approves nothing, and the new module stays; one that can't be
            // opened or read may approve the old bytes once it reads again.
            let writes = s.grant_writes();
            if stored && !s.store_grant_in(turn, id, None) && s.holds_plugin_grant(id) {
                let back = previous.as_ref().is_ok_and(|held| s.put_back_plugin(turn, id, held.as_deref()));
                stored = !back && holds_new(s);
                if !back || matches!(previous, Ok(None)) {
                    s.ignore_stored_grant(id, writes);
                }
            }
        }
    }
    stored
}

/// `plugin.inspect`: what installing the module would install, without installing it, whether that
/// needs the user to grant its permissions first (`needsApproval`) and whether it would take
/// another module's place (`needsReplace`).
fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    load_given("plugin.inspect", p).map(|(plugin, _)| {
        let mut v = describe(&plugin);
        v["needsApproval"] = json!(!s.plugin_approved(&plugin));
        v["needsReplace"] = json!(takes_over(s, &plugin));
        let (unknown, refused) = super::plugin_call::check_allowlist(plugin.manifest());
        if !unknown.is_empty() {
            v["unknownCommands"] = json!(unknown);
        }
        if !refused.is_empty() {
            v["refusedCommands"] = json!(refused);
        }
        v
    })
}

/// The plug-in in the `.wasm` file `path`, or in the `.vcplugin` manifest `path` and the `.js` file
/// it names, and its module: for a JavaScript plug-in, the manifest with the script's text
/// ([`registry::read_js_package`]). Under automation read roots the script must be inside them too.
#[cfg(not(target_arch = "wasm32"))]
fn load_path(path: &str) -> Result<(Plugin, Vec<u8>)> {
    crate::file_access::check_read(path).map_err(EngineError::Other)?;
    let limits = vectorcraft_plugins::Limits::default();
    let file = std::path::Path::new(path);
    let bytes = if registry::is_js_package(file) {
        let allow = |script: &std::path::Path| match script.to_str() {
            Some(script) => crate::file_access::check_read(script),
            None => Err(format!("automation path rejected: {} isn't valid text", script.display())),
        };
        registry::read_js_package(file, &allow).map_err(err)?
    } else {
        crate::store::read_capped(file, limits.max_module_bytes as u64).map_err(EngineError::Other)?
    };
    Ok((Plugin::load(&bytes, limits).map_err(err)?.with_source(path), bytes))
}

#[cfg(target_arch = "wasm32")]
fn load_path(_: &str) -> Result<(Plugin, Vec<u8>)> {
    Err(EngineError::Other("installing from a path is not available on the web; pass the module as `dataBase64`".into()))
}

/// The plug-in folder last loaded, the plug-in store whose grants it was loaded with, what
/// happened, the load whose report it is ([`FOLDER_LOADS`]), and whether a file failed because no
/// loader thread was left to load it ([`vectorcraft_plugins::loader::NO_THREAD`]), so that the next
/// [`sync_prefs`] loads the folder again.
type FolderRecord = (String, Option<std::path::PathBuf>, Value, u64, bool);

/// The [`FolderRecord`] of the plug-in folder last loaded.
static FOLDER: Mutex<Option<FolderRecord>> = Mutex::new(None);

/// The number of the last folder load [`sync_prefs`] or `plugin.reload` started. [`sync_prefs`]
/// keeps it in [`FOLDER`] while its load runs, so only that load replaces or clears the record, and
/// a `plugin.reload` keeps its report only when no load started after it.
static FOLDER_LOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Loads the Additional Plug-ins Folder preference (`*.wasm`) when it is set and it, or the
/// session's plug-in store, has changed since the last load. Runs whenever preferences are applied
/// (startup, `prefs.set`). With [`Session::load_plugins_in_background`] on, it returns while the
/// modules load, and `plugin.list` reports the folder as loading until they are installed. Off, it
/// returns once the folder's plug-ins are installed, also while another session is still loading
/// the same folder. Native only.
pub(crate) fn sync_prefs(s: &mut Session) {
    let folder = s.prefs.plugins_folder.trim().to_string();
    // In Safe Mode nothing loads, and the folder is loaded once it ends ([`load_prefs_folder`]).
    if folder.is_empty() || safe_mode::active() {
        return;
    }
    // This session's own load of the folder, still running, is waited for unless loads go on in the
    // background.
    if s.plugin_loads.iter().any(|l| matches!(l, PluginLoad::Folder(f) if f.loads(&folder, &s.plugin_dir))) {
        if !s.plugin_loads_in_background {
            s.finish_plugin_loads();
        }
        return;
    }
    let token = {
        let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
        // A folder another session is still loading is loaded here too: that load may end with its
        // session, and this one returns once its own plug-ins are installed.
        // A load in which a file found no thread to load it is tried again.
        if last.as_ref().is_some_and(|(f, store, r, _, retry)| *f == folder && *store == s.plugin_dir && r["loading"] != true && !retry) {
            return;
        }
        let token = FOLDER_LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *last = Some((folder.clone(), s.plugin_dir.clone(), json!({"folder": folder, "loading": true}), token, false));
        token
    };
    let load = start_folder(s, &folder, Some(token));
    s.queue_plugin_load(PluginLoad::Folder(load));
}

/// The Additional Plug-ins Folder preference was cleared from `old`: when `old` is the folder last
/// loaded, it is forgotten, so setting the preference to it again loads it again.
pub(crate) fn forget_prefs_folder(old: &str) {
    let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref().is_some_and(|(f, _, _, _, _)| f == old.trim()) {
        *last = None;
    }
}

/// Turns Safe Mode on for `--safe-mode`, before the plug-in folder is attached: no plug-in loads or
/// runs in this process from then on.
pub fn start_in_safe_mode() {
    safe_mode::set_active(Some(safe_mode::Source::Flag));
}

/// Loads the stored plug-ins and the Additional Plug-ins Folder preference: the app calls it when
/// the user leaves the choice after a launch that stopped with Disable or Start Normally, which
/// turns Safe Mode off first.
pub fn load_prefs_folder(s: &mut Session) {
    s.load_stored_plugins();
    *FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    sync_prefs(s);
}

/// The modules of a folder loading ([`registry::FolderLoad`]); the web build has none.
#[cfg(not(target_arch = "wasm32"))]
type FolderModules = registry::FolderLoad;
#[cfg(target_arch = "wasm32")]
type FolderModules = std::convert::Infallible;

/// A load of the plug-ins in a folder ([`start_folder`]), which [`install_folder`] installs as the
/// modules load and [`end_folder`] ends.
pub(crate) struct FolderPending {
    folder: String,
    /// The plug-in store whose grants it loads with.
    store: Option<std::path::PathBuf>,
    /// The load's number in [`FOLDER`] when [`end_folder`] keeps its report as the folder last
    /// loaded ([`sync_prefs`]).
    record: Option<u64>,
    /// A `plugin.reload`'s load: [`end_folder`] keeps its report as the folder last loaded unless a
    /// load started after it.
    reload: bool,
    /// The modules loading, or the report of a folder that can't be loaded; `None` once ended.
    load: Option<std::result::Result<FolderModules, Value>>,
    /// Whether the modules not installed yet were dropped ([`FolderPending::cancel`]).
    canceled: bool,
}

impl FolderPending {
    /// Whether every module has been installed or reported.
    pub(crate) fn done(&self) -> bool {
        match &self.load {
            #[cfg(not(target_arch = "wasm32"))]
            Some(Ok(load)) => load.done(),
            #[cfg(target_arch = "wasm32")]
            Some(Ok(never)) => match *never {},
            Some(Err(_)) | None => true,
        }
    }

    /// Starts the modules loading, unless they started already.
    pub(crate) fn start(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(Ok(load)) = &mut self.load {
            load.start();
        }
    }

    /// The installs and removals its modules are checked against (`None`: the folder couldn't be
    /// listed, or the load ended).
    pub(crate) fn changes(&self) -> Option<&registry::LoadChanges> {
        match &self.load {
            #[cfg(not(target_arch = "wasm32"))]
            Some(Ok(load)) => Some(load.changes()),
            #[cfg(target_arch = "wasm32")]
            Some(Ok(never)) => match *never {},
            Some(Err(_)) | None => None,
        }
    }

    /// [`FolderPending::changes`], to adopt an earlier load's.
    pub(crate) fn changes_mut(&mut self) -> Option<&mut registry::LoadChanges> {
        match &mut self.load {
            #[cfg(not(target_arch = "wasm32"))]
            Some(Ok(load)) => Some(load.changes_mut()),
            #[cfg(target_arch = "wasm32")]
            Some(Ok(never)) => match *never {},
            Some(Err(_)) | None => None,
        }
    }

    /// Whether something can be installed or reported now, without waiting.
    pub(crate) fn ready(&mut self) -> bool {
        match &mut self.load {
            #[cfg(not(target_arch = "wasm32"))]
            Some(Ok(load)) => load.ready(),
            #[cfg(target_arch = "wasm32")]
            Some(Ok(never)) => match *never {},
            Some(Err(_)) | None => true,
        }
    }

    /// Waits until the next module in file-name order has loaded or failed.
    pub(crate) fn wait(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(Ok(load)) = &mut self.load {
            load.wait();
        }
    }

    /// Drops the modules not installed yet, leaving it [`FolderPending::done`]; [`end_folder`]
    /// then keeps no report as the folder last loaded.
    pub(crate) fn cancel(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(Ok(load)) = &mut self.load {
            load.cancel();
        }
        self.canceled = true;
    }

    /// Whether it is a load of `folder` with the plug-in store `store`.
    pub(crate) fn loads(&self, folder: &str, store: &Option<std::path::PathBuf>) -> bool {
        self.folder == folder && self.store == *store
    }
}

impl Drop for FolderPending {
    /// A load dropped before it ended (its session closed) leaves no folder reported as loading, so
    /// the next session that applies the preference loads it.
    fn drop(&mut self) {
        if let Some(token) = self.record.filter(|_| self.load.is_some()) {
            let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref().is_some_and(|(_, _, _, t, _)| *t == token) {
                *last = None;
            }
        }
    }
}

/// Starts loading the plug-ins in `folder` on worker threads ([`registry::FolderLoad`]); `record`:
/// the load's number in [`FOLDER`], whose record [`end_folder`] replaces with the report.
#[cfg(not(target_arch = "wasm32"))]
fn start_folder(s: &Session, folder: &str, record: Option<u64>) -> FolderPending {
    let pending =
        |load| FolderPending { folder: folder.to_string(), store: s.plugin_dir.clone(), record, reload: false, load: Some(load), canceled: false };
    if let Err(e) = crate::file_access::check_read(folder) {
        return pending(Err(json!({"folder": folder, "error": e})));
    }
    // A file in it may be a link to one elsewhere. Each file is checked again on its worker
    // thread, in the scope in force here (the automation roots and the script gate), as the
    // folder itself was.
    let scope = crate::file_access::ScopeHandle::capture_now();
    let allow = move |f: &std::path::Path| {
        scope.clone().run(|| match f.to_str() {
            Some(f) => crate::file_access::check_read(f),
            // A lossy name would check another file than the one loaded, so such a file loads only
            // where no read is checked: without roots and outside a script's command.
            None if crate::file_access::current().is_some() => Err(format!("automation path rejected: {} isn't valid text", f.display())),
            None if crate::file_access::in_script_command() => Err(format!("Script file access denied: {} isn't valid text", f.display())),
            None => Ok(()),
        })
    };
    // The modules start loading once the loads queued before them are done.
    pending(registry::FolderLoad::list(std::path::Path::new(folder), allow).map_err(|e| json!({"folder": folder, "error": e.to_string()})))
}

#[cfg(target_arch = "wasm32")]
fn start_folder(s: &Session, folder: &str, record: Option<u64>) -> FolderPending {
    let load = Err(json!({"folder": folder, "error": "plug-in folders are not available on the web"}));
    FolderPending { folder: folder.to_string(), store: s.plugin_dir.clone(), record, reload: false, load: Some(load), canceled: false }
}

/// Installs what `load` has loaded so far, in file-name order up to the first module still
/// loading, without waiting: those that list permissions only when `s` has them granted
/// ([`Session::plugin_approved`], with the store's grants read once). Runs while
/// [`hold_plugin_changes`] and `turn` are held, so a `plugin.install` in another session or process
/// finishes writing the store first; when the store can't be locked, the folder still installs,
/// and the store isn't written.
pub(crate) fn install_folder(s: &mut Session, load: &mut FolderPending, turn: &StoreTurn) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(Ok(load)) = &mut load.load
        && !load.done()
    {
        for id in load.install_ready(s.plugin_folder_approver()) {
            s.attach_plugin_storage(turn, &id);
        }
        // A disabled plug-in's file matched by its id is saved in the same turn.
        if load.disabled_changed() && s.store_disabled_plugins_in(turn) {
            load.disabled_saved();
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (s, load, turn);
}

/// Ends `load` once [`FolderPending::done`], while [`hold_plugin_changes`] and `turn` are held, so
/// the grants of ids whose module lists no permissions are forgotten while no `plugin.install` can
/// record one for another module → its report; saves the disabled list when loading changed it.
pub(crate) fn end_folder(s: &mut Session, mut load: FolderPending, turn: &StoreTurn) -> Value {
    let (report, retry) = match load.load.take() {
        #[cfg(not(target_arch = "wasm32"))]
        Some(Ok(modules)) => {
            let r = modules.finish();
            if r.disabled_changed {
                s.store_disabled_plugins_in(turn);
            }
            let failed = r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>();
            (json!({"folder": load.folder, "loaded": r.loaded, "failed": failed, "leftOut": r.left_out}), r.no_thread)
        }
        #[cfg(target_arch = "wasm32")]
        Some(Ok(never)) => match never {},
        Some(Err(report)) => (report, false),
        None => (Value::Null, false),
    };
    if let Some(token) = load.record {
        let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
        // A later load, or a `plugin.reload`, replaced it meanwhile. A canceled load leaves no
        // folder reported, so the next session that applies the preference loads it. A
        // `plugin.reload`'s report replaces the record unless a load started after it, or the
        // folder couldn't be loaded.
        let current = if load.reload {
            FOLDER_LOADS.load(std::sync::atomic::Ordering::Relaxed) == token && report.get("error").is_none()
        } else {
            last.as_ref().is_some_and(|(_, _, _, t, _)| *t == token)
        };
        if current {
            *last = if load.canceled { None } else { Some((load.folder.clone(), load.store.clone(), report.clone(), token, retry)) };
        }
    }
    report
}

/// Loads the plug-ins in `folder` and installs them, waiting for every module → the report, which is
/// kept as the folder last loaded unless a load started after this one.
fn load_folder_now(s: &mut Session, folder: &str) -> Value {
    let token = FOLDER_LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let mut load = start_folder(s, folder, Some(token));
    load.reload = true;
    #[cfg(test)]
    if let Some(between) = BEFORE_RELOAD_INSTALLS.take() {
        between();
    }
    s.finish_plugin_load(PluginLoad::Folder(load))
}

/// Tests: the next [`sync_prefs`] treats the folder last loaded as one in which a file found no
/// loader thread to load it.
#[cfg(test)]
pub(crate) fn folder_found_no_thread() {
    if let Some(record) = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        record.4 = true;
    }
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
    // Loads started earlier install first, so this one's plug-ins replace theirs.
    s.finish_plugin_loads();
    let report = load_folder_now(s, &folder);
    if let Some(e) = report.get("error").and_then(Value::as_str) {
        return Err(EngineError::Other(e.to_string()));
    }
    Ok(report)
}

/// [`describe`] for an installed plug-in, with the bytes its cache holds (`cacheBytes`).
fn describe_installed(p: &Plugin) -> Value {
    let mut v = describe(p);
    v["cacheBytes"] = json!(registry::storage(p.id()).map_or(0, |st| st.cache_bytes()));
    v
}

/// The folder load `report` as a reply shows it: under automation read roots, a folder outside them
/// is `null` and a failed file outside them is named by its file name, in the errors too.
fn shown_report(mut report: Value) -> Value {
    let mut hidden: Vec<(String, String)> = vec![];
    if let Some(failed) = report.get_mut("failed").and_then(Value::as_array_mut) {
        for row in failed {
            let Some(file) = row.get("file").and_then(Value::as_str).map(str::to_string) else { continue };
            let shown = shown_file(&file);
            if shown != file {
                row["file"] = json!(shown);
                hidden.push((file, shown));
            }
        }
    }
    if let Some(folder) = report.get("folder").and_then(Value::as_str).map(str::to_string) {
        let shown = shown_file(&folder);
        if shown != folder {
            report["folder"] = Value::Null;
            hidden.push((folder, shown));
        }
    }
    // A failed file's path holds its folder's: the longer paths are put back first.
    hidden.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
    let unnamed = |text: &str| hidden.iter().fold(text.to_string(), |t, (path, name)| t.replace(path.as_str(), name));
    if let Some(e) = report.get("error").and_then(Value::as_str).map(unnamed) {
        report["error"] = json!(e);
    }
    if let Some(failed) = report.get_mut("failed").and_then(Value::as_array_mut) {
        for row in failed {
            if let Some(e) = row.get("error").and_then(Value::as_str).map(unnamed) {
                row["error"] = json!(e);
            }
        }
    }
    report
}

fn list(_: &mut Session, _: &Value) -> Result<Value> {
    let folder = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, _, r, _, _)| shown_report(r.clone()));
    let disabled: Vec<Value> = safe_mode::disabled().iter().map(shown_item).collect();
    Ok(
        json!({"plugins": registry::list().iter().map(|p| describe_installed(p)).collect::<Vec<_>>(), "folder": folder, "safeMode": safe_mode::active(), "disabled": disabled}),
    )
}

/// `plugin.safeMode`: whether Safe Mode is on and why, the plug-ins disabled for the next launch,
/// and the plug-ins the last launch was loading when it stopped while the user hasn't chosen yet.
fn safe_mode_state(_: &mut Session, _: &Value) -> Result<Value> {
    let pending: Vec<Value> = safe_mode::previous().iter().map(shown_item).collect();
    Ok(json!({
        "active": safe_mode::active(),
        "source": safe_mode::source().map(safe_mode::Source::name),
        "disabled": safe_mode::disabled().iter().map(shown_item).collect::<Vec<_>>(),
        "pendingItem": pending.first(),
        "pendingItems": pending,
    }))
}

/// The file of a plug-in `plugin.safeMode` lists as `file`, or `plugin.list` as `source`: one they name
/// by its file name (outside the automation read roots) is found by that name, when no other listed
/// file has it.
fn listed_file(file: String) -> String {
    let mut listed: Vec<String> = safe_mode::disabled()
        .into_iter()
        .chain(safe_mode::previous())
        .filter_map(|i| i.file)
        .chain(registry::list().iter().filter_map(|p| p.source().map(str::to_string)))
        .collect();
    listed.sort();
    listed.dedup();
    if listed.contains(&file) {
        return file;
    }
    let mut named = listed.into_iter().filter(|f| shown_file(f) == file);
    match (named.next(), named.next()) {
        (Some(f), None) => f,
        _ => file,
    }
}

/// `plugin.setEnabled`: turns a plug-in on or off for the next launch, by `id` or by `file` (a
/// plug-in that stopped a launch while loading may be known by its file alone). It neither loads
/// nor unloads anything now, and uninstalls nothing.
fn set_enabled(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "plugin.setEnabled";
    let Some(enabled) = p.get("enabled").and_then(Value::as_bool) else {
        let key = "enabled";
        return Err(bad(C, format!("`{key}` must be true or false")));
    };
    let text = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.trim().to_string()).filter(|v| !v.is_empty())),
        Some(_) => Err(bad(C, format!("`{key}` must be a string"))),
    };
    // Turning on by id also matches the plug-in's stored file, which names it when it stopped a
    // launch while loading.
    let (item, stored) = match (text("id")?, text("file")?) {
        (Some(id), _) => {
            let stored = if enabled { s.stored_plugin_file(&id).map(safe_mode::Item::file) } else { None };
            (registry::get(&id).map(|p| safe_mode::Item::of(&p)).unwrap_or(safe_mode::Item { id: Some(id), ..Default::default() }), stored)
        }
        (None, Some(file)) => (safe_mode::Item::file(listed_file(file)), None),
        (None, None) => return Err(bad(C, "give `id` or `file` (see plugin.safeMode)")),
    };
    let items: Vec<&safe_mode::Item> = std::iter::once(&item).chain(stored.as_ref()).collect();
    // The list is changed and saved in one turn on the plug-in store, so that an install in another
    // process doesn't change the store in between.
    let turn = s.plugin_store_turn().ok();
    if enabled && !safe_mode::disabled().iter().any(|i| items.iter().any(|item| i.same(item))) {
        return Err(bad(C, "that plug-in isn't disabled (see plugin.safeMode)"));
    }
    let mut changed = false;
    for item in items {
        changed |= safe_mode::set_enabled(item, enabled);
    }
    // Without a plug-in folder (a session confined to automation roots), or when the list there
    // can't take the change, the change stays in this process and goes with the next change that saves.
    let saved = !changed || turn.as_ref().is_some_and(|turn| s.store_disabled_plugins_in(turn));
    Ok(json!({"changed": changed, "saved": saved, "disabled": safe_mode::disabled().iter().map(shown_item).collect::<Vec<_>>()}))
}

fn info(_: &mut Session, p: &Value) -> Result<Value> {
    let plugin = plugin_by_id("plugin.info", p)?;
    let mut v = describe_installed(&plugin);
    v["manifest"] = serde_json::to_value(plugin.manifest()).unwrap_or(Value::Null);
    v["defaults"] = Value::Object(plugin.manifest().defaults());
    v["lastError"] = json!(effect::last_error(plugin.id()));
    Ok(v)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "id").ok_or_else(|| bad("plugin.remove", "missing `id`"))?;
    // An install in another session or process finishes its store writes first. When the store
    // can't be locked, nothing is removed.
    let _held = hold_plugin_changes();
    let turn = s
        .plugin_store_turn()
        .map_err(|e| EngineError::Other(format!("plugin.remove: {}: {e}: nothing was removed", crate::store::STORE_NOT_LOCKED)))?;
    let installed = registry::get(id).is_some();
    // Settings and cache the store keeps for an id that isn't installed are deleted too.
    let stored = !installed && vectorcraft_plugins::manifest::valid_id(id) && s.has_plugin_storage(id);
    // Installed again, it asks again. A grant that can't be removed still approves the module, and
    // so may a grants file that can't be opened or read. One that is too large or doesn't hold a
    // list approves nothing and doesn't stop the removal.
    let forget = || s.store_grant_in(&turn, id, None) || !s.holds_plugin_grant(id);
    // Its settings and cache are moved aside and its grant removed before anything is deleted: when
    // either can't be done, the plug-in stays installed, as it was, and a second `plugin.remove`
    // tries again.
    let failed = if installed || stored {
        s.remove_plugin_storage(&turn, id, forget).err().map(|e| e.to_string())
    } else {
        (!forget()).then(|| crate::store::GRANT_NOT_REMOVED.to_string())
    };
    if let Some(e) = failed {
        if installed {
            s.attach_plugin_storage(&turn, id);
        }
        return Err(EngineError::Other(e));
    }
    registry::revoke(id);
    s.unstore_plugin(&turn, id);
    if !stored && (!installed || !registry::remove(id)) {
        return Err(err(vectorcraft_plugins::Error::NotFound(id.into())));
    }
    Ok(json!({"removed": id}))
}

/// `plugin.clearCache`: empties the cache of plug-in `id`, else of every installed plug-in →
/// `{cleared: [id], bytesFreed}`. Settings stay.
fn clear_cache(_: &mut Session, p: &Value) -> Result<Value> {
    let plugins = match p.get("id").filter(|v| !v.is_null()) {
        Some(_) => vec![plugin_by_id("plugin.clearCache", p)?],
        None => registry::list(),
    };
    let (mut cleared, mut freed) = (Vec::new(), 0_u64);
    for plugin in plugins {
        if let Some(st) = registry::storage(plugin.id()) {
            freed += st.clear_cache().map_err(|e| EngineError::Other(e.to_string()))?;
        }
        cleared.push(plugin.id().to_string());
    }
    Ok(json!({"cleared": cleared, "bytesFreed": freed}))
}

/// This module's commands: `plugin.list`, `.info`, `.run`, `.install`, `.inspect`, `.remove`,
/// `.reload`, `.clearCache`, `.safeMode` and `.setEnabled`.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "plugin.list",
            "List Plug-ins",
            [],
            None,
            "{} → {plugins: [{id, name, version, kind: \"filter\"|\"effect\"|\"exporter\"|\"importer\", author, description, params (doc), paramsSchema, effect? (the effect id of a live effect: plugin.<id>), format? and extensions? (an exporter's or importer's format id, plugin.<id>, for document.export or document.open's importer, and its file extensions), place?: true (an importer whose files file.place also reads, see file.place's importer), budget?: \"heavy\" (a filter that may compute for longer, see plugin.install's allowHeavy), inputs?: [\"type\"|\"image\"|\"symbolInstance\"] (the objects besides paths a filter reads, see plugin.run), context?: [\"settings\"|\"artboards\"|\"layers\"|\"swatches\"] (the parts of the active document a filter's runs and its or a live effect's commands are given in _context.document, with or without a selection, see docs/plugins.md), size, source, permissions: [name] (what it may do: the ones its manifest lists, else the ones its kind implies; progress: it may report its progress, see Cancel and write log lines; settings: it may keep settings of its own; cache: it may keep a cache of its own; commands: a filter may run the commands its commandAllowlist names, see docs/plugins.md), commandAllowlist?: [command id] (the commands a filter with the commands permission may run while it runs, with vc_call), hash (the module's SHA-256), api (the plug-in API level it was built for, 1, 2 or 3), runtime?: \"js\" and script?: its .js file name (a JavaScript filter, see docs/plugins.md), paintOutput?: true (a live effect whose pieces have paints of their own, see docs/plugins.md), cacheBytes (the bytes its cache holds; see plugin.clearCache), commands?: [{id, command (its command id, plugin.<id>.<command id>, which runs it), label, params (doc), paramsSchema, menu?: {path: [menu label], after?: item label (escaped), placed?: false (it stays in its plug-in's submenu: its label reads like a built-in item or submenu of that menu or another plug-in command placed there)} (the menu its manifest places it in, see docs/plugins.md)}] (the menu commands a filter or live effect adds)}], folder: the Additional Plug-ins Folder load report {folder, loaded, failed: [{file, error}], leftOut} | null (under automation read roots, a folder outside them is null, and a failed file and a plug-in's source outside them are named by their file name), safeMode: bool (no plug-in loads or runs, see plugin.safeMode), disabled: [{id, file, name, label}] (turned off for the next launch, see plugin.setEnabled)}",
            always,
            list
        ),
        cmd!(
            query "plugin.info",
            "Plug-in Info",
            [],
            None,
            "{id} → plugin.list's entry plus {manifest, defaults, lastError (a live effect's last failure, null; for a live effect with paintOutput, its name and the refused field, such as objects[1].strokes[0].width)}",
            always,
            info
        ),
        cmd!(
            "plugin.run",
            "Run Plug-in",
            [],
            None,
            "{id, params?: {…} (the plug-in's parameters; or give them as top-level keys; missing ones take their defaults, see plugin.info), ids?: [object id] (left out or null: the selection), …the plug-in's parameters as top-level keys} run an object filter on the selected paths and compound paths (inside selected groups and layers too; hidden, locked, guide and clipping paths, compound paths that clip a clip group and paths inside a group that clips one are left out): it may change, delete or add objects, as one undo step (an output that changes no object and no kept data records none and leaves the document unmodified); the result is selected, except the objects it locked or made guides. A filter whose manifest lists inputs (type, image, symbolInstance; it lists doc.read in permissions, so plugin.install needs grant) is also given the selected objects of those kinds to read, in paint order from the bottom, with at most 16 MiB of pixels and 16 MiB of type runs per run (an object that doesn't fit what the objects before it left comes with pixelsOmitted or truncated; an image that reaches the decoder counts toward the pixels even when decoding fails); every object counts toward the input's half of the plug-in's memory cap, ids, bounds, transforms and names included, and an object whose fields besides its pixels and runs don't fit fails the run with a memory-budget error naming it and its size (a path or compound path fails it with the memory-budget error for a selection too large to hand over), and more than 100,000 of these objects fail it with an object-budget error: its output returns each as {id} or exactly as given, or the run is refused and nothing changes. With nothing selected it gets no objects (generators add theirs to the current layer). A filter with a commandAllowlist runs the commands it names while it runs (vc_call, see docs/plugins.md): its calls and its output are one undo step, and a run that fails or makes a call that fails changes nothing → {plugin, ids, notice?, pluginName?} (notice: the text the output reports in its notice key once applied, escaped and cut at 200 characters; pluginName: the plug-in's name, with it)",
            has_doc,
            run
        ),
        cmd!(
            query "plugin.install",
            "Install Plug-in",
            [],
            None,
            "{path: a .wasm file or a JavaScript plug-in's .vcplugin manifest (read with the .js file its script names) (desktop) | dataBase64: the module or a JavaScript plug-in as one JSON object (its manifest, with the script's text in source) (+ name?: shown as its source), replace?: bool (true once the user agreed: a module with other bytes than the plug-in installed with its id, or with none installed, one whose id has settings or a cache in the plug-in store, installs only with it, takes that plug-in's place and gets its settings and cache; false: one whose id is installed is refused; left out, the same module installs again), name?: file name (with dataBase64), allowHeavy?: bool (a filter whose manifest asks for a heavy budget, budget \"heavy\" in plugin.inspect, installs only with it), grant?: [permission] (a plug-in whose manifest lists permissions, needsApproval in plugin.inspect, installs only when grant holds every one of them; the grant is recorded with the module's hash and covers only that module, so a module with other bytes needs a new grant even when it lists the same permissions; installing a module that lists no permissions forgets the grant held for its id; a refused install keeps the grant, a failed write of the module to the plug-in store leaves the grant in the store, and a stored module that is disabled or not read in Safe Mode keeps the grant for its id; with a plug-in store attached, only the grants kept there count; a request's allowHeavy and grant are refused unless the session was started with vectorcraft-cli --allow-plugin-grants: in the desktop app a person approves the install, see ui.installPlugin, and a module whose hash is already recorded, by a person's approval or by a grant accepted under that flag, needs no grant), hash?: string (the module's SHA-256 as plugin.inspect gave it: a module with other bytes is refused)} validate and install a plug-in for this session (with a plug-in store, the install waits while another window or process changes the store and writes the module and its grant before that window or process changes it again; when the store's disabled.lock can't be locked, nothing is installed) (a module whose commandAllowlist names a command no plug-in may call, such as one that reads or writes files, is refused) → its plugin.list entry, plus unknownCommands?: [id] (commandAllowlist ids no command has; a call of one fails)",
            always,
            install
        ),
        cmd!(
            query "plugin.inspect",
            "Inspect Plug-in",
            [],
            None,
            "{path: a .wasm file or a .vcplugin manifest (desktop) | dataBase64: the module or a JavaScript plug-in's JSON, name?: the file name shown as its source (with dataBase64)} validate a plug-in without installing it, running none of its code → the plugin.list entry it would have, plus needsApproval: bool (true when it lists permissions no person has granted this very module, its bytes included: it then needs a person's approval, see plugin.install) and needsReplace: bool (true when it would take the place of another module with its id: plugin.install then needs replace: true), unknownCommands?: [id] (commandAllowlist ids no command has) and refusedCommands?: [id] (commandAllowlist ids no plug-in may call: plugin.install refuses the module)",
            always,
            inspect
        ),
        cmd!(
            query "plugin.remove",
            "Remove Plug-in",
            [],
            None,
            "{id} uninstall a plug-in (its live effects then leave the geometry as it is) and delete the settings and cache it kept (when their folder can't be moved aside, or its grant can't be removed from the plug-in store, the command fails and the plug-in stays installed; when the settings can't be moved back either, the error names the folder they stay in, and the next load, install or removal of the plug-in moves them back); for an id that isn't installed, deletes the settings and cache the plug-in store still keeps for it; it waits while another window or process changes the plug-in store, and when the store's disabled.lock can't be locked, the command fails and nothing is removed → {removed}",
            always,
            remove
        ),
        cmd!(
            query "plugin.clearCache",
            "Clear Plug-in Cache",
            [],
            None,
            "{id?} delete every entry in the cache of plug-in id (default: of every installed plug-in); its settings stay → {cleared: [id], bytesFreed}",
            always,
            clear_cache
        ),
        cmd!(
            query "plugin.reload",
            "Reload Plug-ins",
            ["Object", "Plug-ins"],
            None,
            "{path?: a folder (default: the Additional Plug-ins Folder preference)} install every *.wasm and *.vcplugin in it, the first 1,000 by file name (desktop; a bad module, one that isn't a regular file, and one disabled with plugin.setEnabled, is reported and skipped, and so are a heavy filter and a plug-in that lists permissions the user hasn't granted this very module, its bytes included; an error in Safe Mode) → {folder, loaded: [id], failed: [{file, error}], leftOut: the count of *.wasm and *.vcplugin files past the first 1,000, not tried}",
            always,
            reload
        ),
        cmd!(
            query "plugin.safeMode",
            "Safe Mode",
            [],
            None,
            "{} → {active: bool (no plug-in loads or runs: plugin.install, .run, .reload and exporters fail, live effects draw their object's geometry unchanged and the objects keep their effects), source: \"flag\" (started with --safe-mode) | \"startup\" (the user chose Safe Mode, or hasn't chosen yet, after a launch that stopped while a plug-in was loading) | null, disabled: [{id, file, name, label}] (turned off for the next launch; id is null for one known only by its file; under automation read roots, a file outside them is named by its file name here and in pendingItem, and plugin.setEnabled takes that name), pendingItem: {id, file, name, label} | null (the first of pendingItems), pendingItems: [{id, file, name, label}] (the plug-ins the last launch was loading when it stopped, oldest first, until the user chooses; any of them, or none, may be what stopped it)}. Safe Mode is chosen at launch, never by a command",
            always,
            safe_mode_state
        ),
        cmd!(
            query "plugin.setEnabled",
            "Enable on Re-launch",
            [],
            None,
            "{id | file: a plug-in as plugin.safeMode's disabled lists it or as plugin.list's source (under automation read roots, a file outside them by the file name shown, when no other listed file has it), enabled: bool} turn a plug-in on or off for the next launch (Help › Safe Mode's Enable on Re-launch); nothing loads, unloads or is uninstalled now. Turning on by id also turns on the file the plug-in is stored in. Turning on one that isn't disabled is an error. plugin.install of a disabled plug-in turns it on → {changed, saved: false when the change isn't in the plug-in folder for the next launch (a session without one, or the list there can't be read or written or is full; it is saved with the next change that saves), disabled}",
            always,
            set_enabled
        ),
    ]
}
