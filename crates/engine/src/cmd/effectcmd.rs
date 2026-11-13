//! Effect menu commands (live effects) and Object → Expand Appearance.
//!
//! Effects are stored on the object's appearance (`appearance.effects`) and evaluated at render
//! time by `vectorcraft-effects` (re-exported by the renderer). The commands edit the objects'
//! own stacks, groups and layers included (layers through `ids`), or with `target: "contents"`
//! the objects inside them (see `appearance`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::appearance::SharedStack;
use vectorcraft_doc::{Document, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_render::effects;

use super::appearance::{ItemTarget, aligned, appearance_targets, index_param, item_indices, item_target, item_target_at};
use super::expand::has_seeded;
use super::rasterfx;
use super::{CommandSpec, EngineError, NodeId, Result, Session, always, bad, cmd, fileio, has_doc, str_param, switch_or};

/// The name of the `svg.filter` effect drawing `xml`: its `<filter>` id. An effect names the
/// document's filter of that name, so `xml` that differs from the document's filter of the same
/// name is refused.
fn svg_filter_name(s: &Session, xml: &str, cmd: &str) -> Result<String> {
    let name = effects::svg_filter::check(xml).map_err(|e| bad(cmd, e))?;
    if let Some(held) = s.doc()?.doc.svg_filters.iter().find(|f| f.name == name)
        && held.xml != xml.trim()
    {
        return Err(bad(
            cmd,
            format!("the document has another SVG filter named {name:?}: use svgFilter.apply, or svgFilter.define with `replace` to edit it"),
        ));
    }
    Ok(name)
}

/// The effect commands of the Effect menu and the Appearance panel, and the 3D and Materials
/// panel's Graphics.
pub fn specs() -> Vec<CommandSpec> {
    let mut specs = effect_specs();
    specs.extend(graphics::specs());
    specs
}

fn effect_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "effect.apply",
            "Apply Effect",
            [],
            None,
            "{effect: id (see effect.list, e.g. \"stylize.dropShadow\", \"distort.roughen\", \"warp.arc\", or \"plugin.<id>\" for an installed effect plug-in), params?: {…} (missing keys take the dialog defaults), item?: appearance item index|null (apply to that fill/stroke only; omitted: the Appearance panel's active item, else the whole object), ids?|id?: [..] (layers too), target?: \"object\"|\"contents\" (contents: the objects inside groups and layers)} append a live effect to each selected object's appearance (a group's or layer's apply to its members as one piece: one combined shadow; a Pathfinder effect, which combines a group's contents, groups several loose selected objects first, in the same undo step) → {ids, index, item, grouped?}",
            has_doc,
            apply
        ),
        cmd!(
            query "effect.list",
            "Effects",
            [],
            None,
            "{ids?|id?: object ids (default: the selection), target?: \"object\"|\"contents\"} → {catalog: [{id, label, menu, params, defaults, raster, lengths: {always, absolute (while relative is false)} (distance params Scale Strokes & Effects scales)}] (installed effect plug-ins last: id plugin.<plug-in id>, menu Effect › Plug-ins), applied: [{id, effects, items: [{index, kind: fill|stroke, effects}]}] (an effect has skipped: true while Safe Mode skips it: a plug-in effect, which the object keeps and draws without), activeItem} for the selection",
            always,
            list
        ),
        cmd!(
            "effect.remove",
            "Remove Effect",
            [],
            None,
            "{index: int (position in the effect list), indices?: [int] (several positions in the same list instead of index, removed in one undo step; every one must exist), item?: appearance item index|null (that fill/stroke's effects; omitted: the active item, else the object's), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} → {ids}",
            has_doc,
            remove
        ),
        cmd!(
            "effect.setParams",
            "Effect Options",
            [],
            None,
            "{index: int, params?: {…} (merged into the current parameters), warpStyle?: string (an existing Warp style suffix, e.g. arc or flag; changes only an applied Warp in place), threeDType?: plane|extrude|revolve|inflate (the 3D and Materials panel's object type: changes only an applied 3D effect in place into Rotate, Extrude & Bevel, Revolve or Inflate; the settings the new effect also takes keep their values, rotation angles wrapped into -180..180 outside Revolve, and the others start at its defaults; params then merge into the new effect's; the type the effect already has, with nothing else given, changes nothing and records no undo step), visible?: bool, item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} → {ids}",
            has_doc,
            set_params
        ),
        cmd!(
            "effect.lightingPreset",
            "3D Lighting Preset",
            [],
            None,
            "{index: int, preset: standard|diffuse|topLeft|right, item?: appearance item index|null (omitted: the active item, else the object's), ids?|id?: [..], target?: object|contents, aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} replace an applied 3D effect's light rig and ambient with an original lighting preset; preserve shading, geometry and view; all targets must have a 3D effect at index other than Extrude & Bevel (Classic), Revolve (Classic) or Rotate (Classic), which have no lighting presets; one undo step → {ids}",
            has_doc,
            lighting_preset
        ),
        cmd!(
            "effect.expandAppearance",
            "Expand Appearance",
            ["Object"],
            None,
            "{ids?|id?: [..], target?} turn each object's appearance into objects: every fill and stroke becomes an object of its own (a stroke stays a stroke, with its weight, dashes, arrowheads and width profile, on its own copy of the path, which object.expand or object.path.outlineStroke outline; brushed strokes become their brush art) grouped in paint order under the object's id and transparency, geometry effects are baked, raster effects become an embedded image (shadows and outer glows below the art; blur, feather and inner glow replace it; an object whose only raster effect is its own Rasterize becomes the image object.rasterize makes with the effect's options), type with effects or fills of its own is outlined; Crop Marks become a group of the object and its marks; Vector Halftone (addon.color.halftone) becomes a group of its dot paths, one per ink; a group's or layer's own fills and strokes become objects among its members, which are expanded too; a symbol instance whose topmost fill or stroke is visible, has an opacity above 0 and has a geometry effect becomes a group of its art, that fill's effects reshaping the art's filled shapes and that stroke's effects reshaping its stroked shapes, as the canvas draws them. An outlined frame of threaded text leaves its thread with the text it shows, and a frame kept as type in a group stays in its thread. A dimension whose art it changes becomes a plain group of its art, except one selected with art it measures, directly or in groups without fills, strokes or effects of their own: its lines and label stay as they are, and if the new art no longer fits its link it stays where it was, no longer linked. Hidden fills, strokes and effects are dropped. In Safe Mode, an object with a visible plug-in effect (in it, or in a symbol it places) is an error and nothing changes. Enabled when a selected or targeted object's appearance isn't basic (`ids` then pick which objects) → {ids, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
            can_expand,
            expand_appearance
        ),
        cmd!(
            "effect.duplicate",
            "Duplicate Effect",
            [],
            None,
            "{index: int, indices?: [int] (several positions in the same list instead of index; every one must exist), item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} insert a copy of each effect right after it, in one undo step → {ids}",
            has_doc,
            duplicate
        ),
        cmd!(
            "effect.move",
            "Move Effect",
            [],
            None,
            "{from: int|[int] (position in the source list; a list moves several effects of that list, which keep their order and land together), to: int (the position of the (first) moved effect in the destination list afterwards), fromItem?: appearance item index|null (the source list: that fill/stroke's effects, null the object's; omitted: the active item, else the object's), toItem?: item index|null (the destination list; default: the source list), copy?: bool (copy instead of move, as Alt-dragging the row), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (refused when the targets' appearances differ)} reorder an effect or move it between the object and its fills/strokes, as one undo step → {ids, index, item}",
            has_doc,
            move_effect
        ),
        cmd!(
            query "effect.deferredStatus",
            "Deferred Effects",
            [],
            None,
            "{} the live-effect plug-in stages the active document left undrawn: while a document is drawn on screen, the plug-in effects of its first view share a 2,000 ms budget (each run also keeps to its own 5,000 ms limit); a stage left out for want of time, or one that times out, fails or has no plug-in, draws its input art unchanged and stays listed until it gives a result (Effect › Refresh Deferred Effects, an edit to its input or parameters, or an export, which runs every stage under the per-call limit), or until its object is deleted or its effect is removed, moved or hidden (or its fill or stroke is hidden) → {budgetMs: 2000, effects: [{objectId (also an object in the art of a symbol, pattern, brush or opacity mask), effectIndex (in the object's effects, or in the fill's or stroke's), item? (the fill or stroke whose effect it is), pluginId, reason: budget|timeout|missing|failed|safeMode}]} (empty: none)",
            has_doc,
            deferred_status
        ),
        cmd!(
            "effect.refreshDeferred",
            "Refresh Deferred Effects",
            ["Effect"],
            None,
            "{ids?|id?: [object id] (objects of the document, also in the art of a symbol, pattern, brush or opacity mask; an object without a deferred stage adds nothing; an id of no object is an error; `id` names one; default: every object with a deferred stage)} run the deferred live-effect plug-in stages of those objects again (effect.deferredStatus), sharing a fresh 2,000 ms budget; a stage that gives a result is drawn with it; not an undo step. Enabled when a stage is deferred → {refreshed, stillDeferred (left out for want of time again, or not reached because the object, or a group or layer holding it, is hidden, or with a part of its input too small for the refresh's drawing), failed (timed out, failed or has no plug-in)}",
            has_deferred,
            refresh_deferred
        ),
    ]
}

/// The active document has deferred live-effect plug-in stages ([`deferred_status`]).
fn has_deferred(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if deferred_effects(&st.doc, &st.plugin_effects).is_empty() { Err("no plug-in effect is deferred".into()) } else { Ok(()) }
}

/// The objects the deferred stages of `fx` name, by id, each with whether it is in `d`'s layers.
/// An object that is not is looked for in the art `d` draws from elsewhere, where live-effect
/// plug-ins run too: the art of its symbols, patterns and brushes, and opacity masks.
fn stage_objects(d: &Document, fx: &vectorcraft_plugins::deferred::Deferrals) -> std::collections::HashMap<u64, (Arc<Node>, bool)> {
    let mut out = std::collections::HashMap::new();
    let mut rest = Vec::new();
    for object in fx.list().into_iter().map(|x| x.stage.object) {
        match d.node_arc(NodeId(object)) {
            Some(n) => {
                out.insert(object, (n.clone(), true));
            }
            None => rest.push(object),
        }
    }
    for (id, n) in definition_objects(d, &rest) {
        out.entry(id).or_insert((n, false));
    }
    out
}

/// The objects of `ids` found in the art `d` draws from outside its layers: the art of its
/// symbols, patterns and brushes, and opacity masks (in that art or in the layers).
fn definition_objects(d: &Document, ids: &[u64]) -> std::collections::HashMap<u64, Arc<Node>> {
    let mut out = std::collections::HashMap::new();
    if ids.is_empty() {
        return out;
    }
    let ids: std::collections::HashSet<u64> = ids.iter().copied().collect();
    let brushes = vectorcraft_brush::library(d);
    let symbols = d.symbols.iter().map(|s| &*s.art);
    let patterns = d.patterns.iter().flat_map(|p| p.art.iter().map(|a| &**a));
    let layers = d.layers.iter().map(|a| &**a);
    let mut look = |n: &Node| {
        if ids.contains(&n.id.0) {
            out.entry(n.id.0).or_insert_with(|| Arc::new(n.clone()));
        }
    };
    for a in symbols.chain(patterns).chain(brushes.iter().flat_map(|b| b.art())).chain(layers) {
        vectorcraft_doc::swatches::walk_with_masks(a, &mut look);
    }
    out
}

/// Is plug-in `plugin`'s effect at `stage`'s place in `n` (`None`: no such object), and visible,
/// on a visible fill or stroke when it is one's?
fn stage_holds(n: Option<&Node>, stage: vectorcraft_plugins::deferred::Stage, plugin: &str) -> bool {
    let Some(n) = n else { return false };
    let list = match stage.item {
        None => Some(n.appearance.effects.as_slice()),
        Some(i) => n.appearance.items.get(i).filter(|it| it.visible()).map(|it| it.effects().as_slice()),
    };
    list.and_then(|l| l.get(stage.effect)).is_some_and(|e| e.visible && e.id == vectorcraft_plugins::effect::effect_id(plugin))
}

/// Drops the records of `fx`'s stages whose object or plug-in effect is no longer at their place
/// in `d` ([`stage_holds`]), and gives the objects of the others ([`stage_objects`]).
fn prune(d: &Document, fx: &vectorcraft_plugins::deferred::Deferrals) -> std::collections::HashMap<u64, (Arc<Node>, bool)> {
    let objects = stage_objects(d, fx);
    fx.retain(|stage, plugin| stage_holds(objects.get(&stage.object).map(|(n, _)| &**n), stage, plugin));
    objects
}

/// The deferred stages of `d` as `effect.deferredStatus` lists them. The records of stages whose
/// object or plug-in effect is no longer at their place in `d` are dropped first.
fn deferred_effects(d: &Document, fx: &vectorcraft_plugins::deferred::Deferrals) -> Vec<Value> {
    prune(d, fx);
    fx.list()
        .into_iter()
        .map(|x| {
            let mut v = json!({"objectId": x.stage.object, "effectIndex": x.stage.effect, "pluginId": x.plugin, "reason": x.reason.name()});
            if let Some(i) = x.stage.item {
                v["item"] = json!(i);
            }
            v
        })
        .collect()
}

/// `effect.deferredStatus`.
fn deferred_status(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    Ok(json!({"budgetMs": vectorcraft_plugins::deferred::OPENING_BUDGET_MS, "effects": deferred_effects(&st.doc, &st.plugin_effects)}))
}

/// Draws `n` alone, small, as the canvas would (its live-effect plug-ins run inside the view that
/// applies): what matters is that it is drawn, not its pixels. Is every part of it drawn, none left
/// out for its size?
fn draw_small(renderer: &mut vectorcraft_render::Renderer, doc: &Document, n: &Arc<Node>) -> bool {
    let Some(b) = n.geometric_bounds() else { return false };
    let scale = 64.0 / b.width().max(b.height()).max(1.0);
    let view = vectorcraft_geom::Affine::scale(scale) * vectorcraft_geom::Affine::translate((1.0 - b.x0, 1.0 - b.y0));
    renderer.stats = Default::default();
    renderer.render_node(doc, n, 66, 66, view);
    renderer.stats.culled == 0
}

/// `effect.refreshDeferred`: each object of the document's layers is drawn again alone, small,
/// inside the document's view, so its stages are run with the input they were deferred with; a
/// hidden object, or one in a hidden layer or group, is not drawn. An object in the art of a
/// symbol, pattern, brush or opacity mask runs where that art is used: the visible layers are
/// drawn then. A stage input left out of a small drawing for its size stays deferred.
fn refresh_deferred(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let (doc, fx) = (st.doc.clone(), st.plugin_effects.clone());
    let ids = super::named_ids(p)?;
    let objects = prune(&doc, &fx);
    // An object of the art drawn outside the layers is valid whether or not it has a stage.
    let elsewhere: Vec<u64> = ids.iter().flatten().filter(|id| doc.node(**id).is_none() && !objects.contains_key(&id.0)).map(|id| id.0).collect();
    let found = definition_objects(&doc, &elsewhere);
    if let Some(id) = elsewhere.iter().find(|id| !found.contains_key(id)) {
        return Err(EngineError::NoNode(NodeId(*id)));
    }
    // Only the objects with a stage have anything to refresh.
    let ids: Option<Vec<u64>> = ids.map(|ids| {
        let mut ids: Vec<u64> = ids.iter().map(|id| id.0).filter(|id| objects.contains_key(id)).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    });
    let counts = fx.refresh(ids.as_deref(), |targets| {
        let mut renderer = vectorcraft_render::Renderer::new();
        let mut whole = Vec::new();
        fx.view(|| {
            let mut elsewhere = false;
            for id in targets {
                match objects.get(id) {
                    Some((n, true)) if doc.is_visible(NodeId(*id)) && draw_small(&mut renderer, &doc, n) => whole.push(*id),
                    Some((_, false)) => elsewhere = true,
                    _ => {}
                }
            }
            if elsewhere {
                for layer in doc.layers.iter().filter(|l| l.visible) {
                    draw_small(&mut renderer, &doc, layer);
                }
            }
        });
        whole
    });
    Ok(json!({"refreshed": counts.refreshed, "stillDeferred": counts.still_deferred, "failed": counts.failed}))
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

/// One undo step (`label`) running `f` on the effect list `item` addresses on each target (an
/// appearance item's own effects, or the object's). Errors with `none` when `f` changed no object;
/// returns the changed ids.
/// Each target of an effect edit aimed at `item` and effect `index`: the object, the item whose
/// effects it edits (`None`: the object's own) and the effect's index there. With `aligned` (see
/// [`ItemTarget::per_target`]) `item` and `index` are the first target's, and each other target
/// edits its equal item, or its equal object-level effect, each target matched on its own whatever
/// the other targets have; in a target without them, `item` and `index` stand for themselves.
pub(crate) fn effect_targets(s: &Session, p: &Value, item: ItemTarget, index: usize, cmd: &str) -> Result<Vec<(NodeId, Option<usize>, usize)>> {
    let roots = appearance_targets(s, p)?;
    let d = &s.doc()?.doc;
    let per = item.per_target(s, &roots);
    let first = roots.first().and_then(|f| d.node(*f));
    // Whether `item` names an item in the stack it is a row of (else the edit is object-level there).
    let anchor = roots.get(item.anchor(s, &roots)).and_then(|a| d.node(*a));
    let anchor_level = anchor.map(|a| item.effects_item(&a.appearance, cmd)).transpose()?.flatten();
    let aligned = aligned(p, cmd)?;
    let mut out = Vec::with_capacity(roots.len());
    for (id, item) in roots.iter().zip(per) {
        let (Some(item), Some(n)) = (item, d.node(*id)) else { continue };
        let at = item.effects_item(&n.appearance, cmd)?;
        // An object-level effect this target shares with the first is where it is in this target.
        let index = match (first.filter(|_| aligned), at, anchor_level) {
            (Some(f), None, None) => SharedStack::of(&[&*f.appearance, &*n.appearance]).effect_in(index, 1).unwrap_or(index),
            _ => index,
        };
        out.push((*id, at, index));
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn edit_effects(
    s: &mut Session,
    p: &Value,
    item: ItemTarget,
    cmd: &str,
    label: &str,
    none: &str,
    index: usize,
    mut f: impl FnMut(&mut Vec<Effect>, usize) -> bool,
) -> Result<Vec<NodeId>> {
    let targets = effect_targets(s, p, item, index, cmd)?;
    s.edit(label, |d, _| {
        let mut done = vec![];
        for &(id, at, index) in &targets {
            let Some(n) = d.node_mut(id) else { continue };
            if n.appearance.effects_mut(at).is_some_and(|fx| f(fx, index)) {
                check_revolve_count(n, cmd)?;
                done.push(id);
            }
        }
        if done.is_empty() {
            return Err(EngineError::Other(none.into()));
        }
        Ok(done)
    })
}

/// [`edit_effects`] for several effects of one list (`indices`): `f` gets each target's list and
/// the positions of `indices` there, ascending; with `aligned`, a position a target doesn't share
/// is left out of it, and a target sharing none of them is left out.
#[allow(clippy::too_many_arguments)]
fn edit_effects_at(
    s: &mut Session,
    p: &Value,
    item: ItemTarget,
    cmd: &str,
    label: &str,
    none: &str,
    indices: &[usize],
    mut f: impl FnMut(&mut Vec<Effect>, &[usize]) -> bool,
) -> Result<Vec<NodeId>> {
    // Each target's list and the positions in it, in the order the targets first come up.
    let mut targets: Vec<(NodeId, Option<usize>, Vec<usize>)> = vec![];
    for &index in indices {
        for (id, at, i) in effect_targets(s, p, item, index, cmd)? {
            match targets.iter_mut().find(|t| t.0 == id && t.1 == at) {
                Some(t) => t.2.push(i),
                None => targets.push((id, at, vec![i])),
            }
        }
    }
    for t in &mut targets {
        t.2.sort_unstable();
        t.2.dedup();
    }
    s.edit(label, |d, _| {
        let mut done = vec![];
        for (id, at, idx) in &targets {
            let Some(n) = d.node_mut(*id) else { continue };
            if n.appearance.effects_mut(*at).is_some_and(|fx| f(fx, idx)) {
                check_revolve_count(n, cmd)?;
                done.push(*id);
            }
        }
        if done.is_empty() {
            return Err(EngineError::Other(none.into()));
        }
        Ok(done)
    })
}

fn check_revolve_count(n: &Node, cmd: &str) -> Result<()> {
    let all = || n.appearance.effects.iter().chain(n.appearance.items.iter().flat_map(|i| i.effects()));
    if all().filter(|e| e.id == effects::REVOLVE).count() > 1 {
        return Err(bad(cmd, "A profile supports one Revolve effect. Edit the existing effect's options instead."));
    }
    if all().filter(|e| effects::is_three_d(&e.id)).count() > 1 {
        return Err(bad(cmd, ONE_3D));
    }
    Ok(())
}

pub(crate) const ONE_3D: &str = "An object takes one 3D effect. Edit the existing effect's options, or remove it first.";

/// `effect.apply`: adds effect `effect` (or `id`) with `params`, its defaults filled in, to the
/// targets' active item as one undo step. A 3D effect must fit each target, which takes only one;
/// a Pathfinder effect on loose objects groups them first. → `{ids, index, item}`.
pub(crate) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.apply";
    let id = str_param(p, "effect").or_else(|| str_param(p, "id")).ok_or_else(|| bad(C, "missing `effect` id"))?;
    // An effect given as `id` names no objects: the targets are read with that `id` as null.
    let effect_as_id;
    let p = if str_param(p, "effect").is_none() {
        let mut q = p.clone();
        q["id"] = Value::Null;
        effect_as_id = q;
        &effect_as_id
    } else {
        p
    };
    let mut params = p.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err(bad(C, "`params` must be an object"));
    }
    super::params::check_effect_color(C, id, &params)?;
    super::params::check_effect_channel(C, id, &params)?;
    // Crop Marks take their style from the preferences when applied.
    if id == effects::CROP_MARKS && params.get("style").is_none() {
        params["style"] = json!(s.crop_mark_style().id());
    }
    if texture_load_effect(id) {
        load_texture(&mut params, C)?;
    }
    // An SVG filter needs its <filter> element, and is named by its id.
    if id == effects::SVG_FILTER {
        let xml = params.get("xml").and_then(Value::as_str).ok_or_else(|| bad(C, "give `xml`, one SVG <filter> element"))?;
        params["name"] = json!(svg_filter_name(s, xml, C)?);
    }
    let effect = effects::new_effect(id, &params).ok_or_else(|| bad(C, format!("unknown effect `{id}`")))?;
    let label = effects::effect_info(id).map(|e| e.label.trim_end_matches('…').to_string()).unwrap_or_default();
    aligned(p, C)?;
    if let Some(loose) = loose_for_pathfinder(s, p, id)? {
        let gid = s.edit(&label, |d, sel| {
            let gid = super::object::group_nodes(d, &loose)?;
            d.node_mut(gid).ok_or(EngineError::NoNode(gid))?.appearance.effects.push(effect.clone());
            sel.set([gid]);
            Ok(gid)
        })?;
        return Ok(json!({ "ids": [gid.0], "index": 0, "item": null, "grouped": true }));
    }
    let item = item_target(s, p, C)?;
    if effects::is_three_d(id) {
        let targets = appearance_targets(s, p)?;
        for (target, at) in targets.iter().copied().zip(item.per_target(s, &targets)) {
            let d = &s.doc()?.doc;
            let n = d.node(target).ok_or(EngineError::NoNode(target))?;
            // A new effect goes after the item's (or the object's) others.
            let on = at.and_then(|at| at.effects_item(&n.appearance, C).ok()).flatten();
            let on = on.and_then(|i| Some((Some(i), n.appearance.items.get(i)?.effects().len())));
            let on = on.unwrap_or((None, n.appearance.effects.len()));
            effects::validate_three_d_at_in(Some(d), n, id, &effect.params, on).map_err(|e| bad(C, e))?;
            if let Some(name) = effects::missing_symbol(d, &effect.params, None) {
                return Err(bad(C, format!("no symbol named `{name}`")));
            }
            if id == effects::REVOLVE && effects::has_revolve(n) {
                return Err(bad(C, "A profile supports one Revolve effect. Edit the existing effect's options instead."));
            }
            if effects::has_revolve(n) || effects::has_solid(n) {
                return Err(bad(C, ONE_3D));
            }
        }
    }
    let ids = edit_effects(s, p, item, C, &label, "Apply Effect: select objects", 0, |fx, _| {
        fx.push(effect.clone());
        true
    })?;
    // The item the effect landed on in the object whose row the item is (the first selected object
    // for the active item, which applies where it fits), and the effect's index in that object.
    let first = ids.get(item.anchor(s, &ids)).and_then(|id| s.doc().ok()?.doc.node(*id));
    let landed = first.and_then(|n| item.effects_item(&n.appearance, C).ok().flatten());
    let index = first.and_then(|n| n.appearance.effects_at(landed)).map_or(0, |fx| fx.len().saturating_sub(1));
    Ok(json!({ "ids": ids_json(&ids), "index": index, "item": landed }))
}

/// The selected objects a Pathfinder effect should group before it applies: it combines the
/// contents of a group or layer, so on several loose objects (none a group) it would do nothing.
/// `None` when the effect isn't one, objects are named (`ids`) or another target is asked for.
fn loose_for_pathfinder(s: &Session, p: &Value, id: &str) -> Result<Option<Vec<NodeId>>> {
    if !effects::is_pathfinder(id) || ["ids", "id", "item", "target"].iter().any(|k| p.get(k).is_some()) {
        return Ok(None);
    }
    let roots = super::appearance::subject_roots(s)?;
    let d = &s.doc()?.doc;
    let grouped = roots.iter().any(|r| d.node(*r).is_some_and(|n| matches!(n.kind, NodeKind::Group { clip: false, .. } | NodeKind::Layer { .. })));
    Ok((roots.len() >= 2 && !grouped).then_some(roots))
}

/// `effect.list`: the effect catalog (built-in and plug-in effects) and the effects applied to the selection, with the active item.
fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let catalog: Vec<Value> = effects::effect_catalog()
        .into_iter()
        .chain(effects::plugin_effects())
        .map(|e| {
            json!({"id": e.id, "label": e.label, "menu": e.menu, "params": e.params, "defaults": e.defaults, "raster": e.raster,
            "lengths": {"always": e.lengths.always, "absolute": e.lengths.absolute}})
        })
        .collect();
    let mut applied = vec![];
    if s.active().is_some() {
        // A plug-in effect Safe Mode skips says so (`skipped`): the object keeps it.
        let fx = |e: &[Effect]| {
            Value::Array(
                e.iter()
                    .map(|e| {
                        let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
                        if vectorcraft_plugins::effect::skipped(&e.id)
                            && let Some(o) = v.as_object_mut()
                        {
                            o.insert("skipped".into(), json!(true));
                        }
                        v
                    })
                    .collect(),
            )
        };
        for id in appearance_targets(s, p)? {
            if let Some(n) = s.doc()?.doc.node(id) {
                let items: Vec<Value> = n
                    .appearance
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, it)| json!({"index": i, "kind": it.kind_name(), "effects": fx(it.effects())}))
                    .collect();
                applied.push(json!({"id": id.0, "effects": fx(&n.appearance.effects), "items": items}));
            }
        }
    }
    Ok(json!({ "catalog": catalog, "applied": applied, "activeItem": s.appearance_item() }))
}

/// The effects `effect.remove` / `effect.duplicate` act on: `indices`, else `index` (ascending, no
/// repeats), and the "no effect" message naming them.
fn effect_indices(p: &Value, cmd: &str) -> Result<(Vec<usize>, String)> {
    let idx = item_indices(p, cmd)?;
    let list = idx.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    let none = if idx.len() == 1 { format!("{cmd}: no effect at index {list}") } else { format!("{cmd}: no effects at indices {list}") };
    Ok((idx, none))
}

/// `effect.remove`: removes effect `index`, or every one of `indices`, from the object's list or fill/stroke `item`'s, in one undo step.
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.remove";
    let (idx, none) = effect_indices(p, C)?;
    let item = item_target(s, p, C)?;
    let label = if idx.len() == 1 { "Remove Effect" } else { "Remove Effects" };
    let ids = edit_effects_at(s, p, item, C, label, &none, &idx, |fx, idx| {
        // Every effect must exist; from the top down, so the lower positions stay valid.
        if idx.iter().any(|i| *i >= fx.len()) {
            return false;
        }
        for &i in idx.iter().rev() {
            fx.remove(i);
        }
        true
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// `effect.duplicate`: copies effect `index`, or each of `indices`, right after its original, in one undo step.
fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.duplicate";
    let (idx, none) = effect_indices(p, C)?;
    let item = item_target(s, p, C)?;
    let label = if idx.len() == 1 { "Duplicate Effect" } else { "Duplicate Effects" };
    let ids = edit_effects_at(s, p, item, C, label, &none, &idx, |fx, idx| {
        if idx.iter().any(|i| *i >= fx.len()) {
            return false;
        }
        // From the top down, so each copy goes right after its original.
        for &i in idx.iter().rev() {
            if let Some(e) = fx.get(i).cloned() {
                fx.insert(i + 1, e);
            }
        }
        true
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// `effect.move`: moves (with `copy`: copies) effect `from`, or the effects `from` lists as one block in their order, to `to` in its own list or in `toItem`'s.
fn move_effect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.move";
    // One position or several (ascending, no repeats).
    let from = match p.get("from") {
        Some(Value::Array(_)) => {
            let list = json!({ "indices": p["from"] });
            item_indices(&list, C)?
        }
        _ => vec![index_param(p, "from", C)?],
    };
    let to = index_param(p, "to", C)?;
    let src = item_target_at(s, p, "fromItem", C)?;
    let dst = if p.get("toItem").is_some() { item_target_at(s, p, "toItem", C)? } else { src };
    let copy = switch_or(p, "copy", false, C)?;
    let roots = appearance_targets(s, p)?;
    // Where an aligned effect would go in each of several different stacks isn't one place.
    if aligned(p, C)? && super::appearance::appearances_differ(&s.doc()?.doc, &roots) {
        return Err(bad(C, "effects of objects whose appearances differ can't be moved together"));
    }
    // Where the effect landed in each object it moved in, in the order of `ids`.
    let mut landed = vec![];
    let ids = s.edit(if copy { "Duplicate Effect" } else { "Move Effect" }, |d, _| {
        let mut done = vec![];
        for id in &roots {
            let Some(n) = d.node_mut(*id) else { continue };
            let (si, di) = (src.effects_item(&n.appearance, C)?, dst.effects_item(&n.appearance, C)?);
            let Some(fx) = n.appearance.effects_mut(si).filter(|fx| from.iter().all(|i| *i < fx.len())) else { continue };
            // Taken out from the top down, so the lower positions stay valid; put back in order.
            let mut taken: Vec<Effect> = from.iter().rev().filter_map(|&i| if copy { fx.get(i).cloned() } else { Some(fx.remove(i)) }).collect();
            taken.reverse();
            let Some(fx) = n.appearance.effects_mut(di) else { continue };
            let at = to.min(fx.len());
            fx.splice(at..at, taken);
            check_revolve_count(n, C)?;
            landed.push((at, di));
            done.push(*id);
        }
        if done.is_empty() {
            let list = from.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
            return Err(EngineError::Other(format!("{C}: no effect at index {list}")));
        }
        Ok(done)
    })?;
    // The reply names the object whose row the destination is: the first selected object for the
    // active item (which need not paint first), as `effect.apply` does; the first target for an
    // explicit `toItem` or the object's effects.
    let (index, item) = landed.get(dst.anchor(s, &ids)).copied().unzip();
    Ok(json!({ "ids": ids_json(&ids), "index": index, "item": item.flatten() }))
}

/// The ids of the effects `effect.setParams` with `p` edits, for the check of their parameters
/// (none when `p` doesn't say which: running it reports why).
pub(crate) fn set_params_effects(s: &Session, p: &Value) -> Vec<String> {
    const C: &str = "effect.setParams";
    let (Ok(index), Ok(item), Ok(st)) = (index_param(p, "index", C), item_target(s, p, C), s.doc()) else {
        return vec![];
    };
    let Ok(targets) = effect_targets(s, p, item, index, C) else { return vec![] };
    // A 3D effect whose type `threeDType` changes takes the new effect's parameters.
    let kind = three_d_type(p, C).ok().flatten();
    let effect = |&(id, at, index): &(NodeId, Option<usize>, usize)| {
        let id = &st.doc.node(id)?.appearance.effects_at(at)?.get(index)?.id;
        Some(kind.filter(|_| effects::is_three_d(id)).map_or_else(|| id.clone(), str::to_string))
    };
    targets.iter().filter_map(effect).collect()
}

/// The effect `threeDType` in `p` turns a 3D effect into ([`effects::THREE_D_TYPES`]), when given.
fn three_d_type(p: &Value, cmd: &str) -> Result<Option<&'static str>> {
    let key = "threeDType";
    let Some(v) = p.get(key) else { return Ok(None) };
    match v.as_str().and_then(|t| effects::THREE_D_TYPES.iter().find(|(name, _)| *name == t)) {
        Some((_, id)) => Ok(Some(id)),
        None => {
            let names: Vec<&str> = effects::THREE_D_TYPES.iter().map(|(name, _)| *name).collect();
            Err(bad(cmd, format!("`{key}` must be one of {}", names.join(", "))))
        }
    }
}

fn lighting_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.lightingPreset";
    let patch = str_param(p, "preset")
        .and_then(effects::lighting_preset_params)
        .ok_or_else(|| bad(C, "preset must be standard, diffuse, topLeft or right"))?;
    let index = index_param(p, "index", C)?;
    let item = item_target(s, p, C)?;
    // Validate the whole selection before changing any object. An unrelated effect or
    // a missing index must not leave just some objects with a new lighting rig.
    for (id, slot, index) in effect_targets(s, p, item, index, C)? {
        let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
        let effect = n.appearance.effects_at(slot).and_then(|fx| fx.get(index));
        if !effect.is_some_and(|e| effects::is_three_d(&e.id)) {
            return Err(bad(C, format!("object {} needs a 3D effect at index {index}", id.0)));
        }
        // The classic 3D effects have no lighting presets; their lights are set in their own options.
        if let Some(e) = effect.filter(|e| effects::is_classic_three_d(&e.id)) {
            let name = effects::effect_info(&e.id).map_or("This effect", |info| info.label.trim_end_matches('…'));
            return Err(bad(C, format!("object {}: {name} at index {index} has no lighting presets; set its lights with effect.setParams", id.0)));
        }
    }
    let mut options = p.clone();
    let map = options.as_object_mut().ok_or_else(|| bad(C, "parameters must be an object"))?;
    map.remove("preset");
    map.insert("params".into(), patch);
    set_params(s, &options)
}

/// `effect.setParams`: merges `params` into the effect at `index` of the targets' active item, and
/// sets its Warp style (`warpStyle`), its 3D type (`threeDType`) or visibility (`visible`), as one
/// undo step. They are checked as `effect.apply` checks them, and a visible 3D effect must still
/// fit; a hidden one has its given values checked. → `{ids}`.
fn set_params(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.setParams";
    let index = index_param(p, "index", C)?;
    let mut params = p.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err(bad(C, "`params` must be an object"));
    }
    let kind = three_d_type(p, C)?;
    let warp_id = p
        .get("warpStyle")
        .map(|v| -> Result<String> {
            let style = v.as_str().filter(|style| effects::WarpStyle::from_id(style).is_some()).ok_or_else(|| bad(C, "unknown Warp style"))?;
            Ok(format!("warp.{style}"))
        })
        .transpose()?;
    let visible = p.get("visible").and_then(Value::as_bool);
    let item = item_target(s, p, C)?;
    let edited = set_params_effects(s, p);
    for effect in &edited {
        super::params::check_effect_color(C, effect, &params)?;
        super::params::check_effect_channel(C, effect, &params)?;
    }
    if edited.iter().any(|e| texture_load_effect(e)) {
        load_texture(&mut params, C)?;
    }
    let targets = effect_targets(s, p, item, index, C)?;
    // The type each 3D effect already has, with nothing else to change, is no edit (and is not
    // checked again).
    let nothing_else = params.as_object().is_none_or(serde_json::Map::is_empty) && visible.is_none() && warp_id.is_none();
    if let Some(to) = kind
        && nothing_else
    {
        let d = &s.doc()?.doc;
        // The effects edited: a target without an effect at `index` is left as it is.
        let ids: Vec<&str> =
            targets.iter().filter_map(|&(id, at, index)| d.node(id)?.appearance.effects_at(at)?.get(index).map(|e| e.id.as_str())).collect();
        if !ids.is_empty() && ids.iter().all(|id| *id == to) {
            return Ok(json!({ "ids": [] }));
        }
    }
    // An SVG filter is named by its id: another filter renames the effect.
    if edited.iter().any(|e| e == effects::SVG_FILTER)
        && let Some(xml) = params.get("xml").and_then(Value::as_str)
    {
        params["name"] = json!(svg_filter_name(s, xml, C)?);
    }
    for &(id, target, index) in &targets {
        let d = &s.doc()?.doc;
        let n = d.node(id).ok_or(EngineError::NoNode(id))?;
        let effect = n.appearance.effects_at(target).and_then(|fx| fx.get(index));
        if warp_id.is_some() && effect.is_some_and(|e| e.id.strip_prefix("warp.").and_then(effects::WarpStyle::from_id).is_none()) {
            return Err(bad(C, "warpStyle can only edit an applied Warp effect"));
        }
        if kind.is_some() && effect.is_some_and(|e| !effects::is_three_d(&e.id)) {
            return Err(bad(C, format!("object {} needs a 3D effect at index {index}", id.0)));
        }
        if let Some(e) = effect
            && effects::is_three_d(&e.id)
        {
            let to = kind.unwrap_or(&e.id);
            let mut prospective =
                if to == e.id { effects::merged_params(&e.id, &e.params) } else { effects::three_d_params_as(&e.id, &e.params, to) };
            if let (Some(cur), Value::Object(new)) = (prospective.as_object_mut(), effects::upgrade_params(to, &params)) {
                cur.extend(new);
            }
            if let Some(name) = effects::missing_symbol(d, &prospective, Some(&e.params)) {
                return Err(bad(C, format!("no symbol named `{name}`")));
            }
            if visible != Some(false) {
                effects::validate_three_d_at_in(Some(d), n, to, &prospective, (target, index)).map_err(|e| bad(C, e))?;
            } else {
                // Hidden, the effect draws nothing, so its object need not fit. The values given are
                // checked, and after a change of 3D type so is every value it keeps.
                let checked = if to == e.id { effects::upgrade_params(to, &params) } else { prospective };
                effects::validate_three_d_params(to, &checked).map_err(|e| bad(C, e))?;
            }
        }
    }
    let ids = edit_effects(s, p, item, C, "Effect Options", &format!("{C}: no effect at index {index}"), index, |fx, index| {
        let Some(e) = fx.get_mut(index) else { return false };
        if let Some(id) = &warp_id {
            e.id.clone_from(id);
        }
        if let Some(to) = kind
            && e.id != to
        {
            e.params = effects::three_d_params_as(&e.id, &e.params, to);
            e.id = to.to_string();
        }
        if let (Value::Object(new), cur) = (effects::upgrade_params(&e.id, &params), &mut e.params) {
            if !cur.is_object() {
                *cur = effects::merged_params(&e.id, &Value::Null);
            }
            // Earlier parameters (a Perspective strength, a light by azimuth and elevation) take
            // their current form first, so the new values replace them and the rest is kept.
            *cur = effects::upgrade_params(&e.id, cur);
            if let Value::Object(m) = cur {
                m.extend(new);
            }
        }
        if let Some(v) = visible {
            e.visible = v;
        }
        true
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

// ---------- Load Texture ----------

/// Effect › Distort › Glass.
const GLASS: &str = "distort.glass";
/// Effect › Texture › Texturizer.
const TEXTURIZER: &str = "texture.texturizer";

/// Does effect `id` have Load Texture: a `textureImage` that `effect.apply` reads from an image
/// file (Glass, Texturizer)?
pub(crate) fn texture_load_effect(id: &str) -> bool {
    id == GLASS || id == TEXTURIZER
}

/// Glass's `textureImage` in `params` as Glass stores it: an image file (`{path}`, or `{name?,
/// dataBase64}`) is read with [`fileio::texture_image`] and picked (`texture: "loaded"`) unless
/// `params` give a `texture`; a string must be a texture as Glass stores it.
fn load_texture(params: &mut Value, cmd: &str) -> Result<()> {
    let pick = params.get("texture").is_none();
    let Some(v) = params.get_mut(effects::pixel::TEXTURE_IMAGE) else { return Ok(()) };
    match v {
        Value::String(s) if effects::pixel::texture(s).is_some() => {}
        Value::Object(_) => {
            let bytes = fileio::source(v, cmd)?.bytes;
            *v = json!(fileio::texture_image(&bytes).map_err(|e| bad(cmd, format!("textureImage: {e}")))?);
            if pick {
                params["texture"] = json!(effects::pixel::LOADED);
            }
        }
        _ => return Err(bad(cmd, "textureImage: give {path} or {name, dataBase64} of an image file, or a texture as effect.list gives it")),
    }
    Ok(())
}

// ---------- Expand Appearance ----------

/// Has `n` an appearance Expand Appearance turns into objects: a path that isn't basic (several
/// fills or strokes, effects, transparency of a fill or stroke, a brush), a group or layer with
/// fills, strokes or effects of its own or such members, other objects with effects, type with
/// fills or strokes of its own, or a symbol instance whose active fill or stroke has a geometry
/// effect ([`effects::has_item_geometry`])?
pub(crate) fn expandable(n: &Node) -> bool {
    expandable_except(n, &[])
}

/// [`expandable`], leaving out the objects `skip` lists and what they hold.
pub(crate) fn expandable_except(n: &Node, skip: &[NodeId]) -> bool {
    if skip.contains(&n.id) {
        return false;
    }
    match &n.kind {
        NodeKind::Path { clipping: true, .. } | NodeKind::Path { guide: true, .. } => false,
        NodeKind::Path { .. } | NodeKind::Compound { .. } => !n.appearance.is_basic() || vectorcraft_brush::has_brush(n),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            !n.appearance.items.is_empty() || !n.appearance.effects.is_empty() || children.iter().any(|c| expandable_except(c, skip))
        }
        NodeKind::Text(_) => !n.appearance.items.is_empty() || !n.appearance.effects.is_empty(),
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => effects::has_item_geometry(n) || !n.appearance.effects.is_empty(),
        _ => !n.appearance.effects.is_empty(),
    }
}

fn can_expand(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let ids = appearance_targets(s, &Value::Null).map_err(|e| e.to_string())?;
    let d = &s.doc().map_err(|e| e.to_string())?.doc;
    // A dimension selected with the art it measures is left as it is.
    let kept = super::dimension::with_their_art(d, &ids);
    if ids.iter().filter_map(|id| d.node(*id)).any(|n| expandable_except(n, &kept)) {
        Ok(())
    } else {
        Err("select (or target) objects whose appearance isn't basic".into())
    }
}

fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

/// The raster effects of `n` (its own, its fills' and strokes') as an embedded image rendered at
/// the document's raster effects resolution or a Rasterize effect's finer one, with whether they
/// all paint below it (shadows and outer glows: the image then holds just them). A group's or
/// layer's image leaves out its members' raster effects, which are expanded with them. `None`
/// when it has none.
fn raster_image(d: &mut Document, n: &Node) -> Option<(Node, bool)> {
    let fx = rasterfx::raster_fx(n);
    if fx.is_empty() {
        return None;
    }
    let below = fx.iter().all(effects::RasterFx::is_below);
    Some((rasterfx::raster_image(d, n, below, true)?, below))
}

/// Expand the pieces of `m`, art evaluated from the object of the same id (its pieces share that
/// id): those made from its own fills and strokes, or with `all` every member. The other members
/// that repeat an id ([`repeats_an_id`]) and hold a random Transform effect ([`has_seeded`]) are
/// expanded as well ([`expand_in_place`]). Then the members get ids of their own. Random in a
/// Transform effect takes its seed from the id, and the canvas draws the pieces with the ids they
/// have before that. Returns what each of those other members' expansions lists, by the member's
/// new id.
fn expand_pieces(
    d: &mut Document,
    m: &mut Node,
    all: bool,
    brushes: &[vectorcraft_brush::Brush],
    stroke_art: effects::StrokeArt,
) -> std::collections::HashMap<NodeId, Vec<Listed>> {
    let id = m.id;
    let mut seen = std::collections::HashSet::from([id]);
    let mut expanded = std::collections::HashMap::new();
    for c in m.children_mut().into_iter().flatten() {
        // A layer's ruler guides are not art: they keep their ids and expand to nothing.
        if c.is_ruler_guide() {
            continue;
        }
        let c = Arc::make_mut(c);
        let mut listed = None;
        if all || c.id == id {
            effects::expand_art(d, c, stroke_art);
        } else if has_seeded(c) && repeats_an_id(c, &seen) {
            listed = Some(expand_in_place(d, c, brushes));
        }
        fresh_ids_listed(d, c, &mut seen, listed.as_deref_mut().unwrap_or_default());
        if let Some(listed) = listed {
            expanded.insert(c.id, listed);
        }
    }
    expanded
}

/// Replace each symbol instance in `m`, art an instance placed (`depth` instances deep), whose
/// active fill or stroke has a geometry effect ([`effects::has_item_geometry`]) by its art as
/// [`effects::expand_outlined`] makes it, and the instances in that art in turn, so
/// [`expand_pieces`] bakes their effects with the rest. An instance stays as it is when it is more
/// than [`vectorcraft_doc::depth::MAX_DEPTH`] instances deep, its symbol is missing, or its art
/// would take the objects placed so far past `budget`.
fn expand_nested_instances(d: &Document, m: &mut Node, depth: usize, budget: &mut u64) {
    for c in m.children_mut().into_iter().flatten() {
        if !matches!(c.kind, NodeKind::SymbolInstance { .. }) {
            if c.children().is_some() {
                expand_nested_instances(d, Arc::make_mut(c), depth, budget);
            }
            continue;
        }
        let NodeKind::SymbolInstance { symbol, .. } = &c.kind else { continue };
        if depth >= vectorcraft_doc::depth::MAX_DEPTH || !effects::has_item_geometry(c) {
            continue;
        }
        let Some(sym) = d.symbols.iter().find(|s| s.name == *symbol) else { continue };
        let count = sym.art.count() as u64;
        if count > *budget {
            continue;
        }
        *budget -= count;
        let art = vectorcraft_render::instance_art(&sym.art, c);
        if let Some(mut a) = effects::expand_outlined(Some(d), c, Some(&art)) {
            expand_nested_instances(d, &mut a, depth + 1, budget);
            *c = Arc::new(a);
        }
    }
}

/// Does `n` hold an id of `seen`, or one id twice: does [`effects::fresh_ids`] give it new ids?
fn repeats_an_id(n: &Node, seen: &std::collections::HashSet<NodeId>) -> bool {
    let mut ids = std::collections::HashSet::new();
    let mut repeats = false;
    n.walk(&mut |c| repeats |= seen.contains(&c.id) || !ids.insert(c.id));
    repeats
}

/// An object whose appearance expanded: its id, and its place in the walk ([`Node::walk`]) of the
/// art it was listed in. Art evaluated from an object can hold one id more than once, and the place
/// tells which of those nodes it is.
type Listed = (NodeId, usize);

/// [`expanded_node`] of `c` in place. Returns what the expansion lists, by places in the walk of `c`.
fn expand_in_place(d: &mut Document, c: &mut Node, brushes: &[vectorcraft_brush::Brush]) -> Vec<Listed> {
    let mut listed = vec![];
    if let Some(e) = expanded_node(d, c.clone(), brushes, &mut listed) {
        *c = e;
    }
    listed
}

/// [`effects::fresh_ids`] on `c`, with each of `listed` (places in the walk of `c`) given the new
/// id of the node at its place.
fn fresh_ids_listed(d: &mut Document, c: &mut Node, seen: &mut std::collections::HashSet<NodeId>, listed: &mut [Listed]) {
    effects::fresh_ids(d, c, seen);
    let mut ids = vec![];
    c.walk(&mut |n| ids.push(n.id));
    for (id, at) in listed {
        if let Some(new) = ids.get(*at) {
            *id = *new;
        }
    }
}

/// Expand Appearance of `id` (and, recursively, of a group's or layer's members): every fill and
/// stroke becomes an object of its own (a stroke a path stroked by it alone, brushes as their art),
/// geometry effects are baked, raster effects become an embedded image, and the object keeps its
/// transparency.
pub(crate) fn expand_node(d: &mut Document, id: NodeId, brushes: &[vectorcraft_brush::Brush], out: &mut Vec<NodeId>) {
    let Some(n) = d.node(id).cloned() else { return };
    let mut listed = vec![];
    let m = expanded_node(d, n, brushes, &mut listed);
    out.extend(listed.into_iter().map(|(id, _)| id));
    if let Some(mut m) = m {
        // Art made from type takes its emoji's pixels into the document.
        d.adopt_pixels(&mut m);
        if let Some(slot) = d.node_mut(id) {
            *slot = m;
        }
    }
}

/// [`expand_node`] of `n`, which need not be in `d`: `n` with its appearance expanded, `None` when
/// it has nothing to expand. `out` gets the objects whose appearance expands, by places in the walk
/// of the expanded `n`, `n`'s first when its own does.
fn expanded_node(d: &mut Document, mut n: Node, brushes: &[vectorcraft_brush::Brush], out: &mut Vec<Listed>) -> Option<Node> {
    let id = n.id;
    if !expandable(&n) {
        return None;
    }
    // Colour adjustments: the colours inside become the adjusted ones (an embedded image a
    // recoloured copy), then the rest of the appearance expands.
    let mut changed = false;
    // Is `n` in `out` yet?
    let mut listed_n = false;
    // A symbol instance's fill tints its art before the adjustment, as the canvas draws it.
    let symbol_art = match &n.kind {
        NodeKind::SymbolInstance { symbol, .. } if effects::has_adjustment(&n) => {
            d.symbols.iter().find(|s| s.name == *symbol).map(|s| vectorcraft_render::instance_art(&s.art, &n))
        }
        _ => None,
    };
    if effects::has_adjustment(&n)
        && let Some(m) = effects::adjust_in_document(d, &n, symbol_art)
    {
        out.push((id, 0));
        listed_n = true;
        n = m;
        changed = true;
        if !expandable(&n) {
            return Some(n);
        }
    }
    // (A symbol instance has become its art, a group.)
    let container = is_container(&n);
    // Crop Marks: the object and its marks become a group. The object expands on its own first,
    // with `id`: the canvas draws it with that id, which Random in a Transform effect takes its
    // seed from. Then the object and the marks get ids of their own.
    if let Some(mut m) = effects::crop_marks_art(&n) {
        let mut seen = std::collections::HashSet::from([id]);
        let mut expanded = vec![];
        for (i, c) in m.children_mut().into_iter().flatten().enumerate() {
            let c = Arc::make_mut(c);
            if i == 0 {
                expanded = expand_in_place(d, c, brushes);
                fresh_ids_listed(d, c, &mut seen, &mut expanded);
            } else {
                effects::fresh_ids(d, c, &mut seen);
            }
        }
        if !listed_n {
            out.push((id, 0));
        }
        out.extend(expanded.into_iter().map(|(id, at)| (id, at + 1)));
        return Some(m);
    }
    // Vector Halftone: the object becomes its dots, a group (with `id`) of paths with ids of their own.
    if let Some(mut m) = vectorcraft_render::addons::halftone::art(d, &n) {
        let mut seen = std::collections::HashSet::from([id]);
        for c in m.children_mut().into_iter().flatten() {
            effects::fresh_ids(d, Arc::make_mut(c), &mut seen);
        }
        if !listed_n {
            out.push((id, 0));
        }
        return Some(m);
    }
    let image = raster_image(d, &n);
    let mut v = n.clone();
    rasterfx::strip_raster(&mut v, false);
    let mut stroke_art =
        |d: &mut Document, path: &PathData, rule: FillRule, st: &StrokeLayer| super::pathops::outlined_stroke(d, brushes, path, rule, st);
    // Members of a group or layer that `expand_pieces` expanded, by id, with what their expansion lists.
    let mut members = std::collections::HashMap::new();
    let expanded = match image {
        // Blur, feather and inner glow change the object itself: it becomes the image (a layer
        // keeps it as its only member).
        Some((image, false)) => Some(rasterfx::replace_with_image(v, image)),
        image => {
            let m = if (container || matches!(n.kind, NodeKind::Text(_))) && (effects::has_solid(&v) || effects::has_revolve(&v)) {
                // A 3D effect on a group or layer, or on type: the shaded faces of its solids.
                effects::expand_leaf(d, &v, &mut stroke_art)
            } else if container {
                // A group's or layer's own fills and strokes become art among its members.
                effects::evaluate_container(Some(&*d), &v).map(|mut m| {
                    members = expand_pieces(d, &mut m, false, brushes, &mut stroke_art);
                    m
                })
            } else if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
                && !effects::has_transform_art(&v)
                && !effects::has_transform_copies(&v)
            {
                effects::expand_leaf(d, &v, &mut stroke_art)
            } else {
                // Type, images, symbol instances, live objects (and paths whose Transform effect copies
                // them, each copy an object of its own, or changes their fills and strokes): through
                // their outlines.
                let symbol = match &n.kind {
                    NodeKind::SymbolInstance { symbol, .. } => {
                        d.symbols.iter().find(|s| s.name == *symbol).map(|s| vectorcraft_render::instance_art(&s.art, &n))
                    }
                    _ => None,
                };
                effects::expand_outlined(Some(&*d), &v, symbol.as_ref()).map(|mut m| {
                    let mut budget = vectorcraft_doc::MAX_SYMBOL_NODES;
                    expand_nested_instances(d, &mut m, 1, &mut budget);
                    expand_pieces(d, &mut m, true, brushes, &mut stroke_art);
                    m
                })
            };
            match image {
                Some((image, _)) => {
                    let mut m = m.unwrap_or(v);
                    rasterfx::put_below(d, &mut m, image);
                    Some(m)
                }
                None => m,
            }
        }
    };
    if let Some(m) = expanded {
        n = m;
        changed = true;
        if !listed_n {
            out.push((id, 0));
        }
    }
    if container {
        // The place of each member in the walk of `n`: after `n` and the members before it.
        let mut at = 1;
        for c in n.children_mut().into_iter().flatten() {
            if c.is_ruler_guide() {
                // A layer's ruler guide expands to nothing, but the walk of `n` still visits it.
                at += 1;
                continue;
            }
            let from = out.len();
            match members.remove(&c.id) {
                Some(listed) => out.extend(listed),
                None => {
                    if let Some(e) = expanded_node(d, (**c).clone(), brushes, out) {
                        *c = Arc::new(e);
                        changed = true;
                    }
                }
            }
            for (_, place) in &mut out[from..] {
                *place += at;
            }
            // Every node the walk visits, ruler guides in a sublayer too (`Node::count` leaves them out).
            c.walk(&mut |_| at += 1);
        }
    }
    changed.then_some(n)
}

/// Does `n`, an object in it, or the art of a symbol placed in it hold a plug-in effect that
/// expanding it draws: a visible one, the object's own or a visible fill's or stroke's, on an
/// object that expands ([`expandable`]; in a symbol's art, `in_art`, every object does). A symbol's
/// art counts where an instance of it expands. `seen` holds the symbols already looked into.
fn holds_plugin_effect(d: &Document, n: &Node, in_art: bool, seen: &mut std::collections::HashSet<String>) -> bool {
    let plugin = |e: &Effect| e.visible && vectorcraft_plugins::effect::plugin_id(&e.id).is_some();
    let mut symbols = vec![];
    let mut holds = false;
    n.walk(&mut |c| {
        if !in_art && !expandable(c) {
            return;
        }
        holds |= c.appearance.effects.iter().any(plugin) || c.appearance.items.iter().any(|i| i.visible() && i.effects().iter().any(plugin));
        if let NodeKind::SymbolInstance { symbol, .. } = &c.kind {
            symbols.push(symbol.clone());
        }
    });
    holds
        || symbols.into_iter().any(|name| {
            seen.insert(name.clone()) && d.symbols.iter().find(|s| s.name == name).is_some_and(|s| holds_plugin_effect(d, &s.art, true, seen))
        })
}

fn expand_appearance(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = appearance_targets(s, p)?;
    // In Safe Mode a plug-in effect draws nothing: expanding it would drop the effect and keep the
    // geometry it hasn't changed.
    if vectorcraft_plugins::safe_mode::active() {
        // A dimension selected with the art it measures is left as it is.
        let d = &*super::dimension::without_kept(&s.doc()?.doc, &roots);
        let mut seen = std::collections::HashSet::new();
        if roots.iter().filter_map(|id| d.node(*id)).any(|n| holds_plugin_effect(d, n, false, &mut seen)) {
            return Err(EngineError::Other("Expand Appearance: a plug-in effect in the selection can't be expanded in Safe Mode".into()));
        }
    }
    let failed = vectorcraft_render::decode_failures();
    let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
    let ids = s.edit("Expand Appearance", |d, _| {
        let before = d.clone();
        // A dimension selected with the art it measures keeps its own lines and label: it is out
        // of the document while the art expands.
        let lifted = super::dimension::lift(d, &roots);
        let kept = !lifted.is_empty();
        let ended = super::dimension::end_links(d, &roots);
        let brushes = vectorcraft_brush::library(d);
        let mut out = vec![];
        let threaded = super::threads::frames(d);
        for id in &roots {
            expand_node(d, *id, &brushes, &mut out);
        }
        super::dimension::put_back(d, lifted);
        // A dimension whose source was replaced stays where it was, no longer linked.
        super::dimension::restore_unchanged(&before, d, ended);
        // Outlined frames of threaded text leave their threads with the text they show; type that
        // stays type under a new id takes its frame's place.
        super::threads::leave_threads(d, &threaded);
        if out.is_empty() || (kept && d.layers == before.layers) {
            return Err(EngineError::Other("Expand Appearance: the selection has a basic appearance".into()));
        }
        super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad("Expand Appearance", e))?;
        Ok(out)
    })?;
    let mut out = json!({ "ids": ids_json(&ids) });
    // Raster effects become images rendered from the art.
    if let Some(w) = rasterfx::undecoded_warning(failed) {
        out["warning"] = json!(w);
    }
    Ok(out)
}

mod graphics;

#[cfg(test)]
mod lighting_presets_tests;

#[cfg(test)]
mod three_d_source_tests;

#[cfg(test)]
mod three_d_type_tests;

#[cfg(test)]
mod tests;
