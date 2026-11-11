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

use std::sync::{Arc, Mutex, RwLock};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node};
use vectorcraft_plugins::{Budget, Command, Kind, Plugin, effect, export, objects, registry, safe_mode};

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
    if !m.inputs.is_empty() {
        v["inputs"] = json!(m.inputs);
    }
    if m.paint_output {
        v["paintOutput"] = json!(true);
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
                json!({"id": c.id, "command": command_id(&m.id, &c.id), "label": c.shown_label(), "params": run.params_doc(),
                       "paramsSchema": serde_json::to_value(&run).ok().and_then(|v| v.get("params").cloned())})
            })
            .collect();
        v["commands"] = json!(commands);
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
        let params = serde_json::to_vec(&params).map_err(|e| bad(c, e.to_string()))?;
        // The input and the parameters may use half the plug-in's memory together.
        let capacity = (plugin.limits().max_memory_bytes / 2).min(i32::MAX as usize).saturating_sub(params.len());
        let (input, read_only) = objects::encode_input_with(&st.doc, &inputs, &kinds, capacity).map_err(err)?;
        let document = (st.uid, st.revision);
        Ok(FilterRun { plugin, command, input, params, inputs, read_only, parent, document })
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
        self.plugin.run_calling(&self.input, &self.params, self.plugin.limits(), &mut bridge).0.map_err(err)
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
    /// it was.
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
        let ids = s.edit(&label, |d, sel| {
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
        let mut out = json!({"plugin": m.id, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()});
        if let Some(cmd) = &self.command {
            out["command"] = json!(cmd.id);
        }
        Ok(out)
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

/// The command spec of plug-in `p`'s command `c`: its id, its label as menus show it (escaped and
/// capped), its menu (Object › Plug-ins › the plug-in's name, or Effect › Plug-ins › it for a live
/// effect: never a menu the manifest names) and its params description.
fn spec_of(p: &Plugin, c: &Command) -> &'static CommandSpec {
    let m = p.manifest();
    let id = command_id(&m.id, &c.id);
    let label = c.shown_label();
    let top = if m.kind == Kind::Effect { "Effect" } else { "Object" };
    let name = permissions::display_text(m.name.trim(), permissions::MAX_SHOWN_NAME);
    let mut fields =
        String::from("ids?: [object id] (left out or null: the selection), params?: {…} (its parameters; or give them as top-level keys)");
    let doc = m.for_command(c).params_doc();
    if let Some(own) = doc.strip_prefix('{').and_then(|d| d.strip_suffix('}')).filter(|d| !d.is_empty()) {
        fields.push_str(", ");
        fields.push_str(own);
    }
    let params = format!(
        "{{{fields}}} run “{label}”, a command of plug-in {}, on the selected paths and compound paths as plugin.run runs a filter, with _context.command {:?}: one undo step → {{plugin, command, ids}}",
        m.id, c.id
    );
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    let same = |s: &&&'static CommandSpec| s.id == id && s.label == label && s.menu == [top, "Plug-ins", name.as_str()] && s.params == params;
    if let Some(s) = made.iter().find(same) {
        return s;
    }
    let menu: &'static [&'static str] = Box::leak(vec![top, "Plug-ins", registry::intern(&name)].into_boxed_slice());
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
/// after the command → `{plugin, command, ids}`. A run that fails (a trap, an exhausted budget, an
/// output it may not make) leaves the document as it was, and the error names the plug-in and the
/// command.
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

/// `plugin.install`: loads and checks the module, refuses `grant` and `allowHeavy` that neither an
/// [`InstallApproval`] for this module nor the session allows, one that asks for a heavy budget
/// unless `allowHeavy` says a person agreed, one that lists permissions no person has granted this
/// very module unless `grant` lists them all, one that would take another module's place
/// ([`takes_over`]) unless `replace` is true, and one whose SHA-256 isn't the `hash` given; records
/// the grant with the module's hash, keeps it for the next start and installs it → its
/// `plugin.list` entry. A module that lists no permissions ends any grant held for its id.
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
    if replace == Some(false) && registry::get(plugin.id()).is_some() {
        return Err(bad(C, format!("a plug-in with id {:?} is already installed", plugin.id())));
    }
    // Another module under the id gets the settings and cache kept for it: whoever installs it asks
    // the user first.
    if replace != Some(true) && takes_over(s, &plugin) {
        let name = permissions::display_text(&plugin.manifest().name, permissions::MAX_SHOWN_NAME);
        return Err(bad(
            C,
            format!(
                "“{name}” is another module than the plug-in with id {:?} and would get the settings and cache kept for it: install it with `replace: true` once the user agrees",
                plugin.id()
            ),
        ));
    }
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
    if !s.plugin_approved(&plugin) && !grant.as_ref().is_some_and(|g| wanted.iter().all(|w| g.contains(w))) {
        let name = permissions::display_text(&m.name, permissions::MAX_SHOWN_NAME);
        let asked = permissions::names(&wanted).join(", ");
        return Err(bad(
            C,
            format!(
                "“{name}” asks for the permissions {asked}: a person has to approve them in the desktop app, or pass `grant` in a session started with --allow-plugin-grants"
            ),
        ));
    }
    // Installed plug-ins stay installed after a restart (#2541). The module is written first and its
    // grant only once it is there; when the grant can't be written, the store gets the module it
    // held back. A failed write of that module is only logged: the store then holds the new module,
    // which the grant doesn't cover. A stored module that can't be read back isn't replaced by one
    // that needs a grant.
    let previous = s.stored_plugin_bytes(plugin.id());
    let mut stored = (previous.is_ok() || !m.asks_permissions()) && s.store_plugin(plugin.id(), &bytes);
    if m.asks_permissions() {
        let g = Grant { id: m.id.clone(), hash: plugin.hash().to_string(), permissions: wanted };
        registry::grant(g.clone());
        if stored && !s.store_grant(&m.id, Some(g)) {
            stored = !s.put_back_plugin(plugin.id(), previous.ok().flatten().as_deref());
        }
    } else {
        // This module takes the id over, so nothing the user approved for the id applies any more.
        registry::revoke(&m.id);
        // When the write failed, the old stored module stays in the folder and needs its grant at
        // the next launch. A grant that can't be removed still approves the old bytes, so the store
        // gets the module it held back. When that module can't be read or written back, or there
        // was none, no module goes with the grant, and the grant stops counting for this session. A
        // grants file that is too large or doesn't hold a list approves nothing, and the new module
        // stays; one that can't be opened or read may approve the old bytes once it reads again.
        if stored && !s.store_grant(&m.id, None) && s.holds_plugin_grant(&m.id) {
            let back = previous.as_ref().is_ok_and(|held| s.put_back_plugin(plugin.id(), held.as_deref()));
            stored = !back;
            if !back || matches!(previous, Ok(None)) {
                s.ignore_stored_grant(&m.id);
            }
        }
    }
    // It gets the settings and cache kept for its id.
    s.attach_plugin_storage(plugin.id());
    // Installing a disabled plug-in again (an updated version) turns it on for the next launch too,
    // also when it is disabled by its stored file alone (it stopped a launch while loading). When
    // the new version could not replace the stored file, the old one stays disabled.
    let file = s.stored_plugin_file(plugin.id());
    let mut changed = false;
    if file.is_none() || stored {
        changed = safe_mode::set_enabled(&safe_mode::Item::of(&plugin), true);
        if let Some(file) = file {
            changed |= safe_mode::set_enabled(&safe_mode::Item::file(file), true);
        }
    }
    if changed {
        s.store_disabled_plugins();
    }
    let mut v = describe_installed(&registry::install(plugin));
    if !unknown.is_empty() {
        v["unknownCommands"] = json!(unknown);
    }
    Ok(v)
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
    // In Safe Mode nothing loads, and the folder is loaded once it ends ([`load_prefs_folder`]).
    if folder.is_empty() || safe_mode::active() {
        return;
    }
    let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref().is_some_and(|(f, store, _)| f == folder && *store == s.plugin_dir) {
        return;
    }
    *last = Some((folder.to_string(), s.plugin_dir.clone(), load_folder(s, folder)));
}

/// The Additional Plug-ins Folder preference was cleared from `old`: when `old` is the folder last
/// loaded, it is forgotten, so setting the preference to it again loads it again.
pub(crate) fn forget_prefs_folder(old: &str) {
    let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref().is_some_and(|(f, _, _)| f == old.trim()) {
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

/// Loads the plug-ins in `folder`, those that list permissions only when `s` has them granted
/// ([`Session::plugin_approved`], with the store's grants read once) → its report; saves the
/// disabled list when loading changed it.
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
            for id in &r.loaded {
                s.attach_plugin_storage(id);
            }
            if r.disabled_changed {
                s.store_disabled_plugins();
            }
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

/// [`describe`] for an installed plug-in, with the bytes its cache holds (`cacheBytes`).
fn describe_installed(p: &Plugin) -> Value {
    let mut v = describe(p);
    v["cacheBytes"] = json!(registry::storage(p.id()).map_or(0, |st| st.cache_bytes()));
    v
}

fn list(_: &mut Session, _: &Value) -> Result<Value> {
    let folder = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, _, r)| r.clone());
    let disabled: Vec<Value> = safe_mode::disabled().iter().map(safe_mode::Item::to_json).collect();
    Ok(
        json!({"plugins": registry::list().iter().map(|p| describe_installed(p)).collect::<Vec<_>>(), "folder": folder, "safeMode": safe_mode::active(), "disabled": disabled}),
    )
}

/// `plugin.safeMode`: whether Safe Mode is on and why, the plug-ins disabled for the next launch,
/// and the plug-in the last launch was loading when it stopped while the user hasn't chosen yet.
fn safe_mode_state(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({
        "active": safe_mode::active(),
        "source": safe_mode::source().map(safe_mode::Source::name),
        "disabled": safe_mode::disabled().iter().map(safe_mode::Item::to_json).collect::<Vec<_>>(),
        "pendingItem": safe_mode::previous().map(|i| i.to_json()),
    }))
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
        (None, Some(file)) => (safe_mode::Item::file(file), None),
        (None, None) => return Err(bad(C, "give `id` or `file` (see plugin.safeMode)")),
    };
    let items: Vec<&safe_mode::Item> = std::iter::once(&item).chain(stored.as_ref()).collect();
    if enabled && !safe_mode::disabled().iter().any(|i| items.iter().any(|item| i.same(item))) {
        return Err(bad(C, "that plug-in isn't disabled (see plugin.safeMode)"));
    }
    let mut changed = false;
    for item in items {
        changed |= safe_mode::set_enabled(item, enabled);
    }
    // Without a plug-in folder (a session confined to automation roots), or when the list there
    // can't take the change, the change stays in this process and goes with the next change that saves.
    let saved = !changed || s.store_disabled_plugins();
    Ok(json!({"changed": changed, "saved": saved, "disabled": safe_mode::disabled().iter().map(safe_mode::Item::to_json).collect::<Vec<_>>()}))
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
    let installed = registry::get(id).is_some();
    // Settings and cache the store keeps for an id that isn't installed are deleted too.
    let stored = !installed && vectorcraft_plugins::manifest::valid_id(id) && s.has_plugin_storage(id);
    // Installed again, it asks again. A grant that can't be removed still approves the module, and
    // so may a grants file that can't be opened or read. One that is too large or doesn't hold a
    // list approves nothing and doesn't stop the removal.
    let forget = || s.store_grant(id, None) || !s.holds_plugin_grant(id);
    // Its settings and cache are moved aside and its grant removed before anything is deleted: when
    // either can't be done, the plug-in stays installed, as it was, and a second `plugin.remove`
    // tries again.
    let failed = if installed || stored {
        s.remove_plugin_storage(id, forget).err().map(|e| e.to_string())
    } else {
        (!forget()).then(|| crate::store::GRANT_NOT_REMOVED.to_string())
    };
    if let Some(e) = failed {
        if installed {
            s.attach_plugin_storage(id);
        }
        return Err(EngineError::Other(e));
    }
    registry::revoke(id);
    s.unstore_plugin(id);
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
            "{} → {plugins: [{id, name, version, kind: \"filter\"|\"effect\"|\"exporter\"|\"importer\", author, description, params (doc), paramsSchema, effect? (the effect id of a live effect: plugin.<id>), format? and extensions? (an exporter's or importer's format id, plugin.<id>, for document.export or document.open's importer, and its file extensions), budget?: \"heavy\" (a filter that may compute for longer, see plugin.install's allowHeavy), inputs?: [\"type\"|\"image\"|\"symbolInstance\"] (the objects besides paths a filter reads, see plugin.run), size, source, permissions: [name] (what it may do: the ones its manifest lists, else the ones its kind implies; progress: it may report its progress, see Cancel and write log lines; settings: it may keep settings of its own; cache: it may keep a cache of its own; commands: a filter may run the commands its commandAllowlist names, see docs/plugins.md), commandAllowlist?: [command id] (the commands a filter with the commands permission may run while it runs, with vc_call), hash (the module's SHA-256), api (the plug-in API level it was built for, 1 or 2), paintOutput?: true (a live effect whose pieces have paints of their own, see docs/plugins.md), cacheBytes (the bytes its cache holds; see plugin.clearCache), commands?: [{id, command (its command id, plugin.<id>.<command id>, which runs it), label, params (doc), paramsSchema}] (the menu commands a filter or live effect adds)}], folder: the Additional Plug-ins Folder load report {folder, loaded, failed: [{file, error}]} | null, safeMode: bool (no plug-in loads or runs, see plugin.safeMode), disabled: [{id, file, name, label}] (turned off for the next launch, see plugin.setEnabled)}",
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
            "{id, params?: {…} (the plug-in's parameters; or give them as top-level keys; missing ones take their defaults, see plugin.info), ids?: [object id] (left out or null: the selection), …the plug-in's parameters as top-level keys} run an object filter on the selected paths and compound paths (inside selected groups and layers too; hidden, locked, guide and clipping paths, compound paths that clip a clip group and paths inside a group that clips one are left out): it may change, delete or add objects, as one undo step; the result is selected, except the objects it locked or made guides. A filter whose manifest lists inputs (type, image, symbolInstance; it lists doc.read in permissions, so plugin.install needs grant) is also given the selected objects of those kinds to read, in paint order from the bottom, with at most 16 MiB of pixels and 16 MiB of type runs per run (an object that doesn't fit what the objects before it left comes with pixelsOmitted or truncated; an image that reaches the decoder counts toward the pixels even when decoding fails); every object counts toward the input's half of the plug-in's memory cap, ids, bounds, transforms and names included, and an object whose fields besides its pixels and runs don't fit fails the run with a memory-budget error naming it and its size (a path or compound path fails it with the memory-budget error for a selection too large to hand over), and more than 100,000 of these objects fail it with an object-budget error: its output returns each as {id} or exactly as given, or the run is refused and nothing changes. With nothing selected it gets no objects (generators add theirs to the current layer). A filter with a commandAllowlist runs the commands it names while it runs (vc_call, see docs/plugins.md): its calls and its output are one undo step, and a run that fails or makes a call that fails changes nothing → {plugin, ids}",
            has_doc,
            run
        ),
        cmd!(
            query "plugin.install",
            "Install Plug-in",
            [],
            None,
            "{path: a .wasm file (desktop) | dataBase64: the module (+ name?: shown as its source), replace?: bool (true once the user agreed: a module with other bytes than the plug-in installed with its id, or with none installed, one whose id has settings or a cache in the plug-in store, installs only with it, takes that plug-in's place and gets its settings and cache; false: one whose id is installed is refused; left out, the same module installs again), name?: file name (with dataBase64), allowHeavy?: bool (a filter whose manifest asks for a heavy budget, budget \"heavy\" in plugin.inspect, installs only with it), grant?: [permission] (a plug-in whose manifest lists permissions, needsApproval in plugin.inspect, installs only when grant holds every one of them; the grant is recorded with the module's hash and covers only that module, so a module with other bytes needs a new grant even when it lists the same permissions; installing a module that lists no permissions forgets the grant held for its id; a refused install keeps the grant, a failed write of the module to the plug-in store leaves the grant in the store, and a stored module that is disabled or not read in Safe Mode keeps the grant for its id; with a plug-in store attached, only the grants kept there count; a request's allowHeavy and grant are refused unless the session was started with vectorcraft-cli --allow-plugin-grants: in the desktop app a person approves the install, see ui.installPlugin, and a module whose hash is already recorded, by a person's approval or by a grant accepted under that flag, needs no grant), hash?: string (the module's SHA-256 as plugin.inspect gave it: a module with other bytes is refused)} validate and install a WebAssembly plug-in for this session (a module whose commandAllowlist names a command no plug-in may call, such as one that reads or writes files, is refused) → its plugin.list entry, plus unknownCommands?: [id] (commandAllowlist ids no command has; a call of one fails)",
            always,
            install
        ),
        cmd!(
            query "plugin.inspect",
            "Inspect Plug-in",
            [],
            None,
            "{path: a .wasm file (desktop) | dataBase64: the module, name?: the file name shown as its source (with dataBase64)} validate a WebAssembly plug-in without installing it → the plugin.list entry it would have, plus needsApproval: bool (true when it lists permissions no person has granted this very module, its bytes included: it then needs a person's approval, see plugin.install) and needsReplace: bool (true when it would take the place of another module with its id: plugin.install then needs replace: true), unknownCommands?: [id] (commandAllowlist ids no command has) and refusedCommands?: [id] (commandAllowlist ids no plug-in may call: plugin.install refuses the module)",
            always,
            inspect
        ),
        cmd!(
            query "plugin.remove",
            "Remove Plug-in",
            [],
            None,
            "{id} uninstall a plug-in (its live effects then leave the geometry as it is) and delete the settings and cache it kept (when their folder can't be moved aside, or its grant can't be removed from the plug-in store, the command fails and the plug-in stays installed); for an id that isn't installed, deletes the settings and cache the plug-in store still keeps for it → {removed}",
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
            "{path?: a folder (default: the Additional Plug-ins Folder preference)} install every *.wasm in it (desktop; a bad module, and one disabled with plugin.setEnabled, is reported and skipped, and so are a heavy filter and a plug-in that lists permissions the user hasn't granted this very module, its bytes included; an error in Safe Mode) → {folder, loaded: [id], failed: [{file, error}]}",
            always,
            reload
        ),
        cmd!(
            query "plugin.safeMode",
            "Safe Mode",
            [],
            None,
            "{} → {active: bool (no plug-in loads or runs: plugin.install, .run, .reload and exporters fail, live effects draw their object's geometry unchanged and the objects keep their effects), source: \"flag\" (started with --safe-mode) | \"startup\" (the user chose Safe Mode, or hasn't chosen yet, after a launch that stopped while a plug-in was loading) | null, disabled: [{id, file, name, label}] (turned off for the next launch; id is null for one known only by its file), pendingItem: {id, file, name, label} | null (the plug-in the last launch was loading when it stopped, until the user chooses; it may not be what stopped it)}. Safe Mode is chosen at launch, never by a command",
            always,
            safe_mode_state
        ),
        cmd!(
            query "plugin.setEnabled",
            "Enable on Re-launch",
            [],
            None,
            "{id | file: a plug-in as plugin.safeMode's disabled lists it, enabled: bool} turn a plug-in on or off for the next launch (Help › Safe Mode's Enable on Re-launch); nothing loads, unloads or is uninstalled now. Turning on by id also turns on the file the plug-in is stored in. Turning on one that isn't disabled is an error. plugin.install of a disabled plug-in turns it on → {changed, saved: false when the change isn't in the plug-in folder for the next launch (a session without one, or the list there can't be read or written or is full; it is saved with the next change that saves), disabled}",
            always,
            set_enabled
        ),
    ]
}
