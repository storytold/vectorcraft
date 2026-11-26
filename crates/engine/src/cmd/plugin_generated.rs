//! Generated art objects: art a generator plug-in draws from its parameters alone
//! ([`vectorcraft_plugins::generate`]), kept in a group with the parameters it was drawn with
//! ([`vectorcraft_doc::generated`]).
//!
//! `plugin.generate` runs a generator and adds its art as a new generated art object, one undo
//! step. `plugin.editGenerated` runs the object's generator again with changed parameters and
//! replaces the art, one undo step (none when nothing changes), placed as the object has been
//! moved, scaled, rotated, reflected or sheared since it was drawn. Object › Expand makes it an
//! ordinary group of its art. `plugin.generatedInfo` describes one.
//!
//! The plug-in runs only when one of these commands asks it to, with its own permissions and budget:
//! opening, placing, pasting, drawing and exporting a document never run it. A generated art object
//! whose plug-in isn't installed, whose saved fields are damaged, or whose art doesn't match the hash
//! its file gave keeps its art as it is, and opening the file says so ([`open_notes`]).

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::generated::{GeneratedArt, MAX_ART_BYTES, State, art_bytes};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_plugins::objects::{self, OutObject};
use vectorcraft_plugins::permissions::{self, display_text};
use vectorcraft_plugins::{Plugin, generate, registry};

use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_doc, str_param};

const GENERATE: &str = "plugin.generate";
const EDIT: &str = "plugin.editGenerated";
const INFO: &str = "plugin.generatedInfo";

/// The undo step of `plugin.editGenerated`.
pub const EDIT_LABEL: &str = "Edit Generated Art";

/// What a plug-in error looks like to the user. A run the caller cancelled changed nothing.
fn err(e: vectorcraft_plugins::Error) -> EngineError {
    match e {
        vectorcraft_plugins::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

/// The error of a plug-in's bad parameters for command `c`, others as [`err`] gives them.
fn params_err(c: &str) -> impl Fn(vectorcraft_plugins::Error) -> EngineError + '_ {
    move |e| match e {
        vectorcraft_plugins::Error::Params(why) => bad(c, why),
        e => err(e),
    }
}

/// The limits a generator runs under: its own, with the Plug-in Instruction Budget in force.
fn limits(s: &Session, plugin: &Plugin) -> vectorcraft_plugins::Limits {
    match s.prefs.plugin_instruction_budget {
        crate::PluginBudget::Normal => plugin.limits().clone(),
        crate::PluginBudget::Large => plugin.large_limits(),
    }
}

/// The plug-in of generated art `g` as a message shows it: its name (its id when it has none) and
/// its id, each as a prompt shows a manifest string.
fn shown(g: &GeneratedArt) -> (String, String) {
    let id = display_text(&g.plugin, permissions::MAX_SHOWN_FIELD);
    let name = if g.name.trim().is_empty() { id.clone() } else { display_text(g.name.trim(), permissions::MAX_SHOWN_NAME) };
    (name, id)
}

/// Generated art whose plug-in, by its saved id, isn't installed.
fn not_installed(g: &GeneratedArt) -> String {
    let (name, id) = shown(g);
    format!("“{name}” ({id}) isn't installed, so its generated art can't be drawn again: the art stays as it is")
}

/// Generated art whose saved fields can't be read.
const DAMAGED: &str = "a generated art object's plug-in id or parameters are damaged, so its art can't be drawn again: the art stays as it is";

/// Generated art that doesn't match the hash its file gave.
fn changed(g: &GeneratedArt) -> String {
    let (name, id) = shown(g);
    format!("the generated art of “{name}” ({id}) doesn't match the hash its file gave: the art stays as it is")
}

/// What opening `doc` says about its generated art objects, each note once: those whose plug-in
/// isn't installed or is installed as a plug-in of another kind, those whose saved fields are
/// damaged, and those whose art doesn't match its file's hash. Opening runs no plug-in.
pub fn open_notes(doc: &Document) -> Vec<String> {
    let mut notes: Vec<String> = vec![];
    let mut visit = |n: &Node| {
        let Some(g) = n.extras().generated.as_deref() else { return };
        let note = match g.state {
            State::Damaged => DAMAGED.to_string(),
            state => match generate::named(&g.plugin) {
                Err(vectorcraft_plugins::Error::NotFound(_)) => not_installed(g),
                Err(e) => e.to_string(),
                Ok(_) if state == State::Changed => changed(g),
                Ok(_) => return,
            },
        };
        if !notes.contains(&note) {
            notes.push(note);
        }
    };
    for layer in &doc.layers {
        vectorcraft_doc::swatches::walk_with_masks(layer, &mut visit);
    }
    for symbol in &doc.symbols {
        vectorcraft_doc::swatches::walk_with_masks(&symbol.art, &mut visit);
    }
    for pattern in &doc.patterns {
        for art in &pattern.art {
            vectorcraft_doc::swatches::walk_with_masks(art, &mut visit);
        }
    }
    for design in &doc.graph_designs {
        vectorcraft_doc::swatches::walk_with_masks(&design.art, &mut visit);
    }
    notes
}

/// Draws the art `out` in generated art object `gid`, in place of the art it holds, placed as
/// `settings` says its art has been transformed ([`GeneratedArt::place`]). The objects go through a
/// filter's checks; art past [`MAX_ART_BYTES`] is refused.
fn draw(d: &mut Document, gid: NodeId, out: Vec<OutObject>, settings: &GeneratedArt) -> Result<()> {
    if let Some(ch) = d.node_mut(gid).and_then(Node::children_mut) {
        ch.clear();
    }
    let admit = |fx: &[vectorcraft_doc::Effect], n: &Node| super::plugin::admit_effects_for(GENERATE, fx, n);
    let ids = objects::apply_output(d, &[], out, Some(gid), &admit).map_err(err)?;
    let g = d.node_mut(gid).ok_or(EngineError::NoNode(gid))?;
    if let Some(ch) = g.children_mut() {
        for c in ch.iter_mut() {
            settings.place(Arc::make_mut(c), vectorcraft_render::effects::scale_effect);
        }
    }
    if art_bytes(g.children().map_or(&[][..], Vec::as_slice)) > MAX_ART_BYTES {
        return Err(err(vectorcraft_plugins::Error::Output(format!(
            "the generator's art is larger than the {MAX_ART_BYTES} bytes a generated art object keeps"
        ))));
    }
    if !super::super::sanity::objects_in_range(d, &ids) {
        return Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()));
    }
    Ok(())
}

/// `plugin.generate {id, params?}`: runs generator `id` with its parameters (checked first: missing
/// ones take their defaults) and adds its art as a new generated art object in the current layer,
/// selected, as one undo step named after the plug-in → `{id, plugin}`.
fn generate_art(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "id").ok_or_else(|| bad(GENERATE, "missing `id` (see plugin.list)"))?;
    let plugin = generate::named(id).map_err(params_err(GENERATE))?;
    let given = p.get("params").cloned().unwrap_or(Value::Null);
    let params = generate::resolve(&plugin, &given).map_err(params_err(GENERATE))?;
    let parent = s.doc()?.target_parent()?;
    let out = generate::run(&plugin, &params, &limits(s, &plugin)).map_err(err)?;
    let m = plugin.manifest();
    let label = m.name.trim_end_matches('…').to_string();
    let gid = s.edit(&label, |d, sel| {
        let gid = d.alloc_id();
        let mut g = Node::group(gid, vec![]);
        g.name = Some(m.name.trim_end_matches('…').to_string());
        let settings = GeneratedArt::new(&m.id, &m.name, params);
        g.extras_mut().generated = Some(Box::new(settings.clone()));
        d.insert(parent, usize::MAX, g)?;
        draw(d, gid, out, &settings)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({"id": gid.0, "plugin": m.id}))
}

/// The generated art object `p` names (`id`), else the one selected → its id and settings. A
/// locked or hidden one, and a selection that holds none or more than one, are errors.
fn target(s: &Session, c: &str, p: &Value) -> Result<(NodeId, GeneratedArt)> {
    let st = s.doc()?;
    let one = |id: NodeId| st.doc.node(id).and_then(|n| n.extras().generated.as_deref().cloned()).map(|g| (id, g));
    let found = match super::given_objects(s, p, c)? {
        Some(ids) if ids.len() == 1 => one(ids[0]),
        Some(_) => None,
        None => match st.selection.objects.as_slice() {
            [id] => one(*id),
            _ => None,
        },
    };
    let select = || bad(c, "select one generated art object");
    let (id, g) = found.ok_or_else(select)?;
    if !st.doc.is_editable(id) {
        return Err(select());
    }
    Ok((id, g))
}

/// The installed generator of generated art `g`, which must be able to draw it again.
fn generator_of(c: &str, g: &GeneratedArt) -> Result<Arc<Plugin>> {
    if g.state == State::Damaged {
        return Err(bad(c, DAMAGED));
    }
    match registry::get(&g.plugin) {
        None => Err(bad(c, not_installed(g))),
        Some(_) => generate::named(&g.plugin).map_err(params_err(c)),
    }
}

/// `plugin.editGenerated {id?, params}`: runs the generator of generated art object `id` (else the
/// selected one) again with its saved parameters changed by `params` (checked against the plug-in's
/// manifest; saved ones it no longer declares are left out) and replaces its art, placed as the
/// object has been transformed since it was drawn, as one undo step. Parameters that change nothing
/// run nothing and make no undo step → `{id, plugin, changed}`.
fn edit_generated(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, g) = target(s, EDIT, p)?;
    let plugin = generator_of(EDIT, &g)?;
    let m = plugin.manifest();
    let given = match p.get("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err(bad(EDIT, "`params` must be an object")),
    };
    let params = generate::resolve_over(&plugin, &g.params, &given).map_err(params_err(EDIT))?;
    if params == g.params {
        return Ok(json!({"id": id.0, "plugin": g.plugin, "changed": false}));
    }
    let out = generate::run(&plugin, &params, &limits(s, &plugin)).map_err(err)?;
    let settings = GeneratedArt { params, state: State::Intact, name: m.name.chars().take(vectorcraft_doc::generated::MAX_NAME).collect(), ..g };
    s.edit(EDIT_LABEL, |d, _| {
        draw(d, id, out, &settings)?;
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        n.extras_mut().generated = Some(Box::new(settings));
        Ok(())
    })?;
    Ok(json!({"id": id.0, "plugin": m.id, "changed": true}))
}

/// `plugin.generatedInfo {id?}`: generated art object `id` (else the selected one) → `{id, plugin,
/// name, params, state, installed}`.
fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, g) = target(s, INFO, p)?;
    let state = match g.state {
        State::Intact => "intact",
        State::Damaged => "damaged",
        State::Changed => "changed",
    };
    let installed = g.state != State::Damaged && generate::named(&g.plugin).is_ok();
    Ok(json!({"id": id.0, "plugin": g.plugin, "name": g.name, "params": g.params, "state": state, "installed": installed}))
}

/// This module's commands: `plugin.generate`, `plugin.editGenerated` and `plugin.generatedInfo`.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "plugin.generate",
            "Generate Art",
            [],
            None,
            "{id: a generator plug-in (plugin.list kind \"generator\"), params?: {…} (its parameters; missing ones take their defaults, see plugin.info; a bad one is an error)} run the generator and add its art to the current layer as a new generated art object (a group that keeps the plug-in's id and name and the parameters, at most 64 KiB of JSON), selected, as one undo step named after the plug-in; its objects go through a filter's checks and the art may take at most 16 MiB of JSON. Edit it with plugin.editGenerated; Object › Expand makes it an ordinary group → {id, plugin}",
            has_doc,
            generate_art
        ),
        cmd!(
            "plugin.editGenerated",
            "Edit Generated Art",
            [],
            None,
            "{id?: a generated art object (default: the one selected), params: {…} (the parameters to change; the others keep their saved values)} run the object's generator again and replace its art, placed as the object has been moved, scaled, rotated, reflected or sheared since it was drawn, as one undo step; parameters that change nothing run nothing and make no undo step. An object whose plug-in isn't installed, or whose saved plug-in id, parameters, transforms or scales are damaged, is an error, and its art stays as it is (Object › Expand keeps it as ordinary art) → {id, plugin, changed}",
            has_doc,
            edit_generated
        ),
        cmd!(
            query "plugin.generatedInfo",
            "Generated Art Info",
            [],
            None,
            "{id?: a generated art object (default: the one selected)} → {id, plugin (its generator's id), name (the generator's name when it drew the art), params, state: \"intact\" | \"damaged\" (its saved plug-in id, parameters, transforms or scales can't be read: it can't be drawn again) | \"changed\" (its art doesn't match the hash its file gave), installed (its generator is installed)}",
            has_doc,
            info
        ),
    ]
}
