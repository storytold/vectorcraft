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

use super::appearance::{
    ItemTarget, aligned, appearance_targets, design_reference, edit_targets, index_param, item_indices, item_target, item_target_at, reference_of,
    with_reference,
};
use super::expand::has_seeded;
use super::rasterfx;
use super::{CommandSpec, EngineError, NodeId, Result, Session, always, bad, check_switches, cmd, fileio, has_doc, str_param, switch_or};

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
            "{effect: id (see effect.list, e.g. \"stylize.dropShadow\", \"distort.roughen\", \"warp.arc\", or \"plugin.<id>\" for an installed effect plug-in), params?: {…} (missing keys take the dialog defaults), item?: appearance item index|null (apply to that fill/stroke only; omitted: the Appearance panel's active item, else the whole object), ids?|id?: [..] (layers too), target?: \"object\"|\"contents\" (contents: the objects inside groups and layers), aligned?: bool (item is a row of the first target's stack: each other target gets the effect on its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, a row of the art's stack: docs/mcp.md), savedBevel?: [[x, y]…] (with params bevel {custom: name}: the custom bevel's points, used in place of the bevels file's; the journal records them so a replay draws the same bevel after the file changes)} append a live effect to each selected object's appearance (a group's or layer's apply to its members as one piece: one combined shadow; a Pathfinder effect, which combines a group's contents, groups several loose selected objects first, in the same undo step); the art of a graph design in a graph series is left out, and objects that are all such art are an error (docs/mcp.md) → {ids, index, item, grouped?}",
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
            "{index: int (position in the effect list), indices?: [int] (several positions in the same list instead of index, removed in one undo step; every one must exist), item?: appearance item index|null (that fill/stroke's effects; omitted: the active item, else the object's), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, rows of the art's stack: docs/mcp.md)} → {ids}",
            has_doc,
            remove
        ),
        cmd!(
            "effect.setParams",
            "Effect Options",
            [],
            None,
            "{index: int, params?: {…} (merged into the current parameters), warpStyle?: string (an existing Warp style suffix, e.g. arc or flag; changes only an applied Warp in place; params are checked against the new style), threeDType?: plane|extrude|revolve|inflate (the 3D and Materials panel's object type: changes only an applied 3D effect in place into Rotate, Extrude & Bevel, Revolve or Inflate; the settings the new effect also takes keep their values, rotation angles wrapped into -180..180 outside Revolve, and the others start at its defaults; params then merge into the new effect's; the type the effect already has, with nothing else given, changes nothing and records no undo step), visible?: bool (any other value but null is refused, when a step plays too), item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, rows of the art's stack: docs/mcp.md), savedBevel?: [[x, y]…] (as effect.apply)} → {ids}",
            has_doc,
            set_params
        ),
        cmd!(
            "effect.lightingPreset",
            "3D Lighting Preset",
            [],
            None,
            "{index: int, preset: standard|diffuse|topLeft|right, item?: appearance item index|null (omitted: the active item, else the object's), ids?|id?: [..], target?: object|contents, aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, rows of the art's stack: docs/mcp.md)} replace an applied 3D effect's light rig and ambient with an original lighting preset; preserve shading, geometry and view; all targets must have a 3D effect at index other than Extrude & Bevel (Classic), Revolve (Classic) or Rotate (Classic), which have no lighting presets; one undo step → {ids}",
            has_doc,
            lighting_preset
        ),
        cmd!(
            "effect.expandAppearance",
            "Expand Appearance",
            ["Object"],
            None,
            "{ids?|id?: [..], target?} turn each object's appearance into objects: every fill and stroke becomes an object of its own (a stroke stays a stroke, with its weight, dashes, arrowheads and width profile, on its own copy of the path, which object.expand or object.path.outlineStroke outline; brushed strokes become their brush art) grouped in paint order under the object's id and transparency, geometry effects are baked, raster effects become an embedded image (shadows and outer glows below the art; blur, feather and inner glow replace it; an object whose only raster effect is its own Rasterize becomes the image object.rasterize makes with the effect's options), type with effects or fills of its own is outlined; Crop Marks become a group of the object and its marks; Vector Halftone (addon.color.halftone) becomes a group of its dot paths, one per ink; a group's or layer's own fills and strokes become objects among its members, which are expanded too; a symbol instance whose topmost fill or stroke is visible, has an opacity above 0 and has a geometry or raster effect becomes a group of its art, that fill's effects reshaping the art's filled shapes and that stroke's effects reshaping its stroked shapes, as the canvas draws them, its raster effects an image as for other objects; a symbol instance in that art whose topmost fill or stroke has such an effect (its own or one the outer instance passes on) becomes its art and image in turn. An outlined frame of threaded text leaves its thread with the text it shows, and a frame kept as type in a group stays in its thread. A dimension whose art it changes becomes a plain group of its art, except one selected with art it measures, directly or in groups without fills, strokes or effects of their own: its lines and label stay as they are, and if the new art no longer fits its link it stays where it was, no longer linked. Hidden fills, strokes and effects are dropped. One expansion makes at most 1,024 images of raster effects, 256 megapixels in all (VectorCraft's own limits): an object or symbol instance whose image would go past them stays as it is, with its effects, and when nothing else expands that is an error. In Safe Mode, an object with a visible plug-in effect (in it, or in a symbol it places) is an error and nothing changes. Enabled when a selected or targeted object's appearance isn't basic (`ids` then pick which objects) → {ids, warning? (an image in the art couldn't be decoded, as object.rasterize says; or raster effects were left past the limits), rasterLeft? (how many objects were left so)}",
            can_expand,
            expand_appearance
        ),
        cmd!(
            "effect.duplicate",
            "Duplicate Effect",
            [],
            None,
            "{index: int, indices?: [int] (several positions in the same list instead of index; every one must exist), item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, rows of the art's stack: docs/mcp.md)} insert a copy of each effect right after it, in one undo step → {ids}",
            has_doc,
            duplicate
        ),
        cmd!(
            "effect.move",
            "Move Effect",
            [],
            None,
            "{from: int|[int] (position in the source list; a list moves several effects of that list, which keep their order and land together), to: int (the position of the (first) moved effect in the destination list afterwards), fromItem?: appearance item index|null (the source list: that fill/stroke's effects, null the object's; omitted: the active item, else the object's), toItem?: item index|null (the destination list; default: the source list), copy?: bool (copy instead of move, as Alt-dragging the row), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (refused when the targets' appearances differ, or differ from that of a graph design's art given first)} reorder an effect or move it between the object and its fills/strokes, as one undo step → {ids, index, item}",
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
/// the other targets have; in a target without them, `item` and `index` stand for themselves. When the first object
/// given is the art of a graph design in a series ([`design_reference`]), which is left out, they are rows of its stack,
/// and a target without them is left out.
pub(crate) fn effect_targets(s: &Session, p: &Value, item: ItemTarget, index: usize, cmd: &str) -> Result<Vec<(NodeId, Option<usize>, usize)>> {
    let roots = edit_targets(s, p, cmd)?;
    let of = reference_of(s, p, item, cmd)?;
    let d = &s.doc()?.doc;
    let per = item.per_target_of(s, of, &roots);
    let first = of.or_else(|| roots.first().copied()).and_then(|f| d.node(f));
    // Whether `item` names an item in the stack it is a row of (else the edit is object-level there).
    let anchor = match of {
        Some(of) => d.node(of),
        None => roots.get(item.anchor(s, &roots)).and_then(|a| d.node(*a)),
    };
    let anchor_level = anchor.map(|a| item.effects_item(&a.appearance, cmd)).transpose()?.flatten();
    let aligned = aligned(p, cmd)?;
    let mut out = Vec::with_capacity(roots.len());
    for (id, item) in roots.iter().zip(per) {
        let (Some(item), Some(n)) = (item, d.node(*id)) else { continue };
        let at = item.effects_item(&n.appearance, cmd)?;
        // An object-level effect this target shares with the first is where it is in this target.
        let index = match (first.filter(|_| aligned), at, anchor_level) {
            (Some(f), None, None) => match SharedStack::of(&[&*f.appearance, &*n.appearance]).effect_in(index, 1) {
                Some(i) => i,
                None if of.is_some() && index != APPENDED => continue,
                None => index,
            },
            _ => index,
        };
        out.push((*id, at, index));
    }
    Ok(out)
}

/// The effect index `effect.apply` gives [`effect_targets`] for the effect it appends, which names no effect the targets
/// have.
const APPENDED: usize = usize::MAX;

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
/// is left out of it, and a target sharing none of them is left out. A position no target has is
/// an error with `none`, and nothing changes.
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
    // With a graph design's art given first, the indices are effects of its list: one it doesn't have names no effect,
    // and the edit is refused as a whole, as it is in a list that lacks one of them.
    if let Some(of) = reference_of(s, p, item, cmd)?
        && let Some(n) = s.doc()?.doc.node(of)
    {
        let ap = &n.appearance;
        let len = match item.effects_item(ap, cmd)? {
            None => ap.effects.len(),
            Some(i) => ap.items.get(i).map_or(0, |it| it.effects().len()),
        };
        if indices.iter().any(|i| *i >= len) {
            return Err(EngineError::Other(none.into()));
        }
    }
    // Each target's list and the positions in it, in the order the targets first come up.
    let mut targets: Vec<(NodeId, Option<usize>, Vec<usize>)> = vec![];
    for &index in indices {
        let found = effect_targets(s, p, item, index, cmd)?;
        // An aligned effect of a design's art that no other object has is refused, as it is given alone.
        if found.is_empty() {
            return Err(EngineError::Other(none.into()));
        }
        for (id, at, i) in found {
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

/// The Pathfinder Options that `effect.apply` gives a Pathfinder effect for the keys its `params`
/// leave out: the preferences' Pathfinder Options in force.
pub fn pathfinder_effect_options(s: &Session) -> effects::PathfinderEffectOptions {
    let current = super::pathops::current_pf_options(s);
    effects::PathfinderEffectOptions {
        precision: current.precision,
        remove_redundant_points: current.remove_redundant_points,
        remove_unpainted: current.remove_unpainted,
    }
}

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
    super::params::check_effect_center(C, id, &params)?;
    super::params::check_effect_pathfinder(C, id, &params)?;
    // Crop Marks take their style from the preferences when applied.
    if id == effects::CROP_MARKS && params.get("style").is_none() {
        params["style"] = json!(s.crop_mark_style().id());
    }
    // A Pathfinder effect keeps the Pathfinder Options in force when it is applied, those `params`
    // give aside. They join the journal entry, so replaying it gives the same effect.
    if effects::takes_pathfinder_options(id) {
        if let Value::Object(filled) = pathfinder_effect_options(s).to_params() {
            for (key, v) in filled {
                if params.get(&key).is_none() {
                    params[key.as_str()] = v;
                }
            }
        }
        s.note_journal("params", params.clone());
    }
    if texture_load_effect(id) {
        load_texture(&mut params, C)?;
    }
    // A custom bevel by name takes a copy of its points from the bevels file.
    if bevel_effect(id) {
        super::bevels::resolve_custom(s, p, &mut params, C)?;
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
    // The item as each target has it, read before the effect changes their rows: a row of a graph design's art, which is
    // left out of the targets, is the row equal to it there, and a target without one is left out.
    let targets = edit_targets(s, p, C)?;
    let of = reference_of(s, p, item, C)?;
    let per: Vec<(NodeId, Option<ItemTarget>)> = targets.iter().copied().zip(item.per_target_of(s, of, &targets)).collect();
    if effects::is_three_d(id) {
        for &(target, at) in &per {
            let Some(at) = at else { continue };
            let d = &s.doc()?.doc;
            let n = d.node(target).ok_or(EngineError::NoNode(target))?;
            // A new effect goes after the item's (or the object's) others.
            let on = at.effects_item(&n.appearance, C).ok().flatten();
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
    let ids = edit_effects(s, p, item, C, &label, "Apply Effect: select objects", APPENDED, |fx, _| {
        fx.push(effect.clone());
        true
    })?;
    // The item the effect landed on in the object whose row the item is (the first selected object
    // for the active item, which applies where it fits), and the effect's index in that object.
    let k = item.anchor(s, &ids);
    let first = ids.get(k).and_then(|id| s.doc().ok()?.doc.node(*id));
    let at = match of {
        Some(_) => ids.get(k).and_then(|id| per.iter().find(|(t, _)| t == id)).and_then(|(_, at)| *at).unwrap_or(item),
        None => item,
    };
    let landed = first.and_then(|n| at.effects_item(&n.appearance, C).ok().flatten());
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
    let d = &s.doc()?.doc;
    // The art of a graph design in a series stays where the graph draws it.
    let roots = super::appearance::without_design_art(d, super::appearance::subject_roots(s)?, "effect.apply")?;
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
    let roots = edit_targets(s, p, C)?;
    // Where an aligned effect would go in each of several different stacks isn't one place.
    if aligned(p, C)? && super::appearance::appearances_differ(&s.doc()?.doc, &with_reference(design_reference(s, p, C)?, &roots)) {
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
    // A 3D effect whose type `threeDType` changes, and a Warp whose style `warpStyle` changes, take
    // the new effect's parameters.
    let kind = three_d_type(p, C).ok().flatten();
    let warp = p.get("warpStyle").and_then(Value::as_str).filter(|style| effects::WarpStyle::from_id(style).is_some());
    let effect = |&(id, at, index): &(NodeId, Option<usize>, usize)| {
        let id = &st.doc.node(id)?.appearance.effects_at(at)?.get(index)?.id;
        Some(edited_effect_id(id, kind, warp))
    };
    targets.iter().filter_map(effect).collect()
}

/// The id effect `id` has once `effect.setParams` sets its 3D type to `kind` or its Warp style to
/// `warp` (each when given and when `id` is an effect of that family).
fn edited_effect_id(id: &str, kind: Option<&str>, warp: Option<&str>) -> String {
    let is_warp = id.strip_prefix("warp.").and_then(effects::WarpStyle::from_id).is_some();
    match (kind, warp) {
        (Some(to), _) if effects::is_three_d(id) => to.to_string(),
        (_, Some(style)) if is_warp => format!("warp.{style}"),
        _ => id.to_string(),
    }
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
pub(crate) fn set_params(s: &mut Session, p: &Value) -> Result<Value> {
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
    check_switches(p, &["visible"], C)?;
    let visible = p.get("visible").and_then(Value::as_bool);
    let item = item_target(s, p, C)?;
    let edited = set_params_effects(s, p);
    for effect in &edited {
        super::params::check_effect_color(C, effect, &params)?;
        super::params::check_effect_channel(C, effect, &params)?;
        super::params::check_effect_center(C, effect, &params)?;
        super::params::check_effect_pathfinder(C, effect, &params)?;
    }
    if edited.iter().any(|e| texture_load_effect(e)) {
        load_texture(&mut params, C)?;
    }
    if edited.iter().any(|e| bevel_effect(e)) || kind == Some(effects::EXTRUDE) {
        super::bevels::resolve_custom(s, p, &mut params, C)?;
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
    // Pathfinder effects given the values they already evaluate with, and nothing else to change,
    // are no edit and record no undo step.
    if kind.is_none() && warp_id.is_none() && visible.is_none() {
        let d = &s.doc()?.doc;
        let fx: Vec<&Effect> = targets.iter().filter_map(|&(id, at, index)| d.node(id)?.appearance.effects_at(at)?.get(index)).collect();
        if !fx.is_empty() && fx.iter().all(|e| pathfinder_edit_changes_nothing(e, &params)) {
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

/// Does effect `id` take a `bevel` (Extrude & Bevel, Extrude & Bevel (Classic)), which may be a
/// custom bevel given by name?
fn bevel_effect(id: &str) -> bool {
    id == effects::EXTRUDE || id == effects::CLASSIC_EXTRUDE
}

/// Does merging `params` into Pathfinder effect `e` leave it drawing as it does: the same Pathfinder
/// Options ([`effects::PathfinderEffectOptions`], so an effect saved without them and given their
/// defaults is unchanged), each option given equal to the one it stores (a precision that Scale
/// Strokes & Effects took past its range evaluates at the bound but is still edited), and every
/// other value given equal to the one it has?
fn pathfinder_edit_changes_nothing(e: &Effect, params: &Value) -> bool {
    if !effects::takes_pathfinder_options(&e.id) {
        return false;
    }
    let Some(given) = params.as_object().filter(|m| !m.is_empty()) else { return true };
    let current = effects::merged_params(&e.id, &e.params);
    let mut after = current.clone();
    if let Value::Object(m) = &mut after {
        m.extend(given.clone());
    }
    let others_same = given.iter().filter(|(k, _)| !effects::PATHFINDER_OPTION_KEYS.contains(&k.as_str())).all(|(k, v)| current.get(k) == Some(v));
    let same_value = |a: &Value, b: &Value| a == b || a.as_f64().is_some_and(|x| b.as_f64() == Some(x));
    let stored_same = given
        .iter()
        .filter(|(k, _)| effects::PATHFINDER_OPTION_KEYS.contains(&k.as_str()))
        .all(|(k, v)| e.params.get(k).is_none_or(|stored| same_value(stored, v)));
    others_same && stored_same && effects::PathfinderEffectOptions::from_params(&current) == effects::PathfinderEffectOptions::from_params(&after)
}

// ---------- Load Texture ----------

/// Effect › Distort › Glass.
const GLASS: &str = "distort.glass";
/// Effect › Texture › Texturizer.
const TEXTURIZER: &str = "texture.texturizer";
/// Effect › Artistic › Rough Pastels.
const ROUGH_PASTELS: &str = "artistic.roughPastels";

/// Does effect `id` have Load Texture: a `textureImage` that `effect.apply` reads from an image
/// file (Glass, Texturizer, Rough Pastels)?
pub(crate) fn texture_load_effect(id: &str) -> bool {
    id == GLASS || id == TEXTURIZER || id == ROUGH_PASTELS
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
/// fills or strokes of its own, or a symbol instance whose active fill or stroke has a geometry or
/// raster effect ([`effects::has_item_geometry`], [`effects::has_instance_row_raster`])?
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
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => {
            effects::has_item_geometry(n) || effects::has_instance_row_raster(n) || !n.appearance.effects.is_empty()
        }
        _ => !n.appearance.effects.is_empty(),
    }
}

fn can_expand(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let ids = appearance_targets(s, &Value::Null).map_err(|e| e.to_string())?;
    let d = &s.doc().map_err(|e| e.to_string())?.doc;
    // The art of a graph design in a series, selected or inside a selected graph, is drawn again from the design.
    let ids = super::appearance::drop_design_art(d, ids);
    // A dimension selected with the art it measures is left as it is.
    let mut kept = super::dimension::with_their_art(d, &ids);
    kept.extend(super::appearance::design_art_in(d, &ids));
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
/// all paint below it (shadows and outer glows: the image then holds just them, and paints with
/// their blend mode when they all use one). A group's or layer's image leaves out its members'
/// raster effects, which are expanded with them. `None` when it has none.
/// `Err` when that image would take `budget` past its limits: the object then stays as it is.
fn raster_image(d: &mut Document, n: &Node, budget: &mut RasterBudget) -> std::result::Result<Option<(Node, bool)>, OverBudget> {
    let fx = rasterfx::raster_fx(n);
    if fx.is_empty() {
        return Ok(None);
    }
    let below = fx.iter().all(effects::RasterFx::is_below);
    let pixels = budget.take(rasterfx::raster_pixels(d, n, below, true))?;
    let mut image = rasterfx::raster_image(d, n, below, true);
    if image.is_none() {
        // No image was made: it takes nothing from the budget.
        budget.images += 1;
        budget.pixels += pixels;
    }
    // Rendered alone, over nothing, the shadows and glows can't blend with the art below the
    // object, so the image does, as the canvas draws them.
    let mut modes = fx.iter().filter_map(effects::RasterFx::mode);
    if below
        && let (Some(mode), Some(image)) = (modes.next(), image.as_mut())
        && modes.all(|m| m == mode)
    {
        image.blend = mode;
    }
    Ok(image.map(|image| (image, below)))
}

/// The most images of raster effects one Expand Appearance makes.
const MAX_RASTER_IMAGES: u32 = 1_024;

/// The most pixels the images of raster effects one Expand Appearance makes hold in all.
const MAX_RASTER_PIXELS: u64 = 256_000_000;

/// The note when objects keep their raster effects because their images would take an Expand
/// Appearance past [`MAX_RASTER_IMAGES`] or [`MAX_RASTER_PIXELS`].
pub(crate) const RASTER_EFFECTS_LEFT: &str =
    "some raster effects were not made into images: they would take more than 1,024 images or 256 megapixels in all";

/// An image of raster effects that would take a [`RasterBudget`] past its limits.
#[derive(Debug)]
struct OverBudget;

/// What one Expand Appearance has left of [`MAX_RASTER_IMAGES`] and [`MAX_RASTER_PIXELS`], and the
/// objects whose raster effects it left as they are because their images would go past them.
pub(crate) struct RasterBudget {
    images: u32,
    pixels: u64,
    pub(crate) left: usize,
}

impl RasterBudget {
    pub(crate) fn new() -> Self {
        Self { images: MAX_RASTER_IMAGES, pixels: MAX_RASTER_PIXELS, left: 0 }
    }

    /// Take an image of `pixels` ([`rasterfx::raster_pixels`]) before it is made, and return
    /// them; `Err`, with its object counted in `left`, when it would go past what is left.
    fn take(&mut self, pixels: u64) -> std::result::Result<u64, OverBudget> {
        if self.images == 0 || pixels > self.pixels {
            self.left += 1;
            return Err(OverBudget);
        }
        self.images -= 1;
        self.pixels -= pixels;
        Ok(pixels)
    }
}

/// Expand the pieces of `m`, art evaluated from the object of the same id (its pieces share that
/// id): those made from its own fills and strokes, or with `all` every member. The other members
/// that repeat an id ([`repeats_an_id`]) and hold a random Transform effect ([`has_seeded`]) are
/// expanded as well ([`expand_in_place`]), except the objects `skip` lists. Then the members get ids
/// of their own. Random in a Transform effect takes its seed from the id, and the canvas draws the
/// pieces with the ids they have before that. `kept` gets the new id of each object in them that
/// had an id `skip` lists before. Returns what each of those other members' expansions lists, by
/// the member's new id. `Err` when such a member still holds a random Transform effect after the
/// raster budget left part of it as it is: its new ids would give that effect another seed.
#[allow(clippy::too_many_arguments)]
fn expand_pieces(
    d: &mut Document,
    m: &mut Node,
    all: bool,
    brushes: &[vectorcraft_brush::Brush],
    skip: &[NodeId],
    kept: &mut Vec<NodeId>,
    stroke_art: effects::StrokeArt,
    raster: &mut RasterBudget,
) -> std::result::Result<std::collections::HashMap<NodeId, Vec<Listed>>, OverBudget> {
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
            let left = raster.left;
            let expanded = expand_in_place(d, c, brushes, skip, raster);
            if raster.left > left && has_seeded(c) {
                return Err(OverBudget);
            }
            listed = Some(expanded);
        }
        let mut was = vec![];
        if !skip.is_empty() {
            c.walk(&mut |n| was.push(n.id));
        }
        fresh_ids_listed(d, c, &mut seen, listed.as_deref_mut().unwrap_or_default());
        if !was.is_empty() {
            // A copy of an object `skip` lists is left as it is too.
            let mut at = 0;
            c.walk(&mut |n| {
                if was.get(at).is_some_and(|old| *old != n.id && skip.contains(old)) {
                    kept.push(n.id);
                }
                at += 1;
            });
        }
        if let Some(listed) = listed {
            expanded.insert(c.id, listed);
        }
    }
    Ok(expanded)
}

/// `m`, the art a 3D effect expanded into, with each object in it that has effects of its own (a
/// shadow blurred by the light's Softness) expanded as Expand Appearance expands it on its own:
/// a blurred shadow becomes an image. Groups in `m` without effects that hold such an object are
/// looked into; the rest are left as they are. The objects `skip` lists are not expanded.
fn expand_raster_pieces(d: &mut Document, mut m: Node, brushes: &[vectorcraft_brush::Brush], skip: &[NodeId], raster: &mut RasterBudget) -> Node {
    for c in m.children_mut().into_iter().flatten() {
        if !c.appearance.effects.is_empty() {
            if let Some(e) = expanded_node(d, (**c).clone(), brushes, skip, &mut vec![], raster) {
                *c = Arc::new(e);
            }
        } else if c.children().is_some() {
            let mut holds = false;
            c.walk(&mut |n| holds |= !n.appearance.effects.is_empty());
            if holds {
                *c = Arc::new(expand_raster_pieces(d, (**c).clone(), brushes, skip, raster));
            }
        }
    }
    m
}

/// Replace each symbol instance in `m`, art an instance placed (`depth` instances deep), whose
/// active fill or stroke has a geometry or raster effect ([`effects::has_item_geometry`],
/// [`effects::has_instance_row_raster`]) by its art ([`expanded_instance`]), and the instances in
/// that art in turn, so [`expand_pieces`] bakes their effects with the rest.
fn expand_nested_instances(d: &mut Document, m: &mut Node, depth: usize, budget: &mut u64, raster: &mut RasterBudget) {
    for c in m.children_mut().into_iter().flatten() {
        if !matches!(c.kind, NodeKind::SymbolInstance { .. }) {
            if c.children().is_some() {
                expand_nested_instances(d, Arc::make_mut(c), depth, budget, raster);
            }
            continue;
        }
        if let Some(a) = expanded_instance(d, c, depth, budget, raster) {
            *c = Arc::new(a);
        }
    }
}

/// Symbol instance `c`, art an instance placed (`depth` instances deep), as art: the raster
/// effects of its active fill or stroke an embedded image ([`raster_image`]) that replaces the art
/// (blur, feather, inner glow) or goes below it (shadows, outer glows), and its art as
/// [`effects::expand_outlined`] makes it without them, or as [`effects::instance_as_art`] does when
/// it has no geometry effect, with the instances in that art expanded in turn. `None`, the instance
/// staying as it is, when its active fill and stroke have no such effect, it is more than
/// [`vectorcraft_doc::depth::MAX_DEPTH`] instances deep, its symbol is missing, or its art would
/// take the objects placed so far past `budget`, or the image of its raster effects would take
/// `raster` past its limits. When that image can't be made, they stay on the art its geometry
/// effects make, and without one it stays as it is.
fn expanded_instance(d: &mut Document, c: &Node, depth: usize, budget: &mut u64, raster: &mut RasterBudget) -> Option<Node> {
    let NodeKind::SymbolInstance { symbol, .. } = &c.kind else { return None };
    let has_raster = effects::has_instance_row_raster(c);
    if depth >= vectorcraft_doc::depth::MAX_DEPTH || !(has_raster || effects::has_item_geometry(c)) {
        return None;
    }
    let sym = d.symbols.iter().find(|s| s.name == *symbol)?;
    let count = sym.art.count() as u64;
    if count > *budget {
        return None;
    }
    let art = vectorcraft_render::instance_art(&sym.art, c);
    let image = if has_raster { raster_image(d, c, raster).ok()? } else { None };
    *budget -= count;
    let mut v = c.clone();
    if image.is_some() {
        rasterfx::strip_raster(&mut v, false);
    }
    let below = match image {
        Some((image, false)) => return Some(rasterfx::replace_with_image(v, image)),
        Some((image, true)) => Some(image),
        None => None,
    };
    let mut a = effects::expand_outlined(Some(&*d), &v, Some(&art))
        .or_else(|| if below.is_some() { effects::instance_as_art(Some(&*d), &v, Some(&art)) } else { None })?;
    expand_nested_instances(d, &mut a, depth + 1, budget, raster);
    if let Some(image) = below {
        rasterfx::put_below(d, &mut a, image);
    }
    Some(a)
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
fn expand_in_place(d: &mut Document, c: &mut Node, brushes: &[vectorcraft_brush::Brush], skip: &[NodeId], raster: &mut RasterBudget) -> Vec<Listed> {
    let mut listed = vec![];
    if let Some(e) = expanded_node(d, c.clone(), brushes, skip, &mut listed, raster) {
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
/// transparency. An object whose raster effects' image would take `raster` past its limits stays
/// as it is, with what is inside it.
pub(crate) fn expand_node(d: &mut Document, id: NodeId, brushes: &[vectorcraft_brush::Brush], out: &mut Vec<NodeId>, raster: &mut RasterBudget) {
    expand_node_except(d, id, brushes, &[], out, raster);
}

/// [`expand_node`], leaving the objects `skip` lists, and what they hold, as they are. A geometry
/// effect of a group or layer around one still reshapes it with the rest of its members, and the
/// copies of it that such an effect makes are left as they are too.
fn expand_node_except(
    d: &mut Document,
    id: NodeId,
    brushes: &[vectorcraft_brush::Brush],
    skip: &[NodeId],
    out: &mut Vec<NodeId>,
    raster: &mut RasterBudget,
) {
    let Some(n) = d.node(id).cloned() else { return };
    let mut listed = vec![];
    let m = expanded_node(d, n, brushes, skip, &mut listed, raster);
    out.extend(listed.into_iter().map(|(id, _)| id));
    if let Some(mut m) = m {
        // Art made from type takes its emoji's pixels into the document.
        d.adopt_pixels(&mut m);
        if let Some(slot) = d.node_mut(id) {
            *slot = m;
        }
    }
}

/// Replace `n`, a group or layer whose Transform effect's copies each carry the raster effects
/// listed before it ([`effects::outer_effects`]), with its art ([`effects::evaluate_container`]):
/// the copies become groups of their own with those effects, and they and what they hold get ids
/// of their own, so they expand as its members do; the raster effects after the Transform effect
/// stay on `n`. A copy that repeats an id ([`repeats_an_id`]) and holds a random Transform effect
/// ([`has_seeded`]) expands before its ids change ([`expand_in_place`]), as [`expand_pieces`] does,
/// and `members` gets what that expansion lists, by the copy's new id. `kept` gets the new id of
/// each copy of an object `skip` lists, and `out` gets `n` unless `listed` says it is there.
/// Returns whether `n` changed; any other object stays as it is. `Err`, with `n` as it is, when
/// such a copy still holds a random Transform effect after the raster budget left part of it as
/// it is: its new ids would give that effect another seed.
fn copies_carrying_raster(
    d: &mut Document,
    n: &mut Node,
    (brushes, skip): (&[vectorcraft_brush::Brush], &[NodeId]),
    kept: &mut Vec<NodeId>,
    members: &mut std::collections::HashMap<NodeId, Vec<Listed>>,
    (out, listed): (&mut Vec<Listed>, &mut bool),
    raster: &mut RasterBudget,
) -> std::result::Result<bool, OverBudget> {
    if !carries_copies(n) {
        return Ok(false);
    }
    let Some(mut m) = effects::evaluate_container(Some(&*d), n) else { return Ok(false) };
    let mut seen = std::collections::HashSet::from([n.id]);
    for c in m.children_mut().into_iter().flatten().filter(|c| !c.is_ruler_guide()) {
        let c = Arc::make_mut(c);
        let left = raster.left;
        let mut expanded = (has_seeded(c) && repeats_an_id(c, &seen)).then(|| expand_in_place(d, c, brushes, skip, raster));
        if expanded.is_some() && raster.left > left && has_seeded(c) {
            return Err(OverBudget);
        }
        let mut was = vec![];
        if !skip.is_empty() {
            c.walk(&mut |n| was.push(n.id));
        }
        fresh_ids_listed(d, c, &mut seen, expanded.as_deref_mut().unwrap_or_default());
        // A copy of an object `skip` lists is left as it is too.
        let mut at = 0;
        c.walk(&mut |n| {
            if was.get(at).is_some_and(|old| *old != n.id && skip.contains(old)) {
                kept.push(n.id);
            }
            at += 1;
        });
        if let Some(expanded) = expanded {
            members.insert(c.id, expanded);
        }
    }
    if !*listed {
        out.push((n.id, 0));
        *listed = true;
    }
    *n = m;
    Ok(true)
}

/// Is `n` a group or layer whose Transform effect's copies each carry the raster effects listed
/// before it ([`copies_carrying_raster`])?
fn carries_copies(n: &Node) -> bool {
    is_container(n) && effects::outer_effects(n).len() != n.appearance.effects.len()
}

/// Does `n`, something in it, or the art of the symbol it places when a color adjustment makes it
/// that art, hold a random Transform effect ([`has_seeded`])? The raster budget can then leave `n`
/// as it is after part of it expanded ([`expand_pieces`], Crop Marks).
fn holds_seeded(d: &Document, n: &Node) -> bool {
    has_seeded(n)
        || match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } if effects::has_adjustment(n) => {
                d.symbols.iter().find(|s| s.name == *symbol).is_some_and(|s| has_seeded(&s.art))
            }
            _ => false,
        }
}

/// [`expand_node`] of `n`, which need not be in `d`: `n` with its appearance expanded, `None` when
/// it has nothing to expand. `out` gets the objects whose appearance expands, by places in the walk
/// of the expanded `n`, `n`'s first when its own does. The objects `skip` lists are not expanded.
fn expanded_node(
    d: &mut Document,
    mut n: Node,
    brushes: &[vectorcraft_brush::Brush],
    skip: &[NodeId],
    out: &mut Vec<Listed>,
    raster: &mut RasterBudget,
) -> Option<Node> {
    let id = n.id;
    if skip.contains(&id) || !expandable(&n) {
        return None;
    }
    // What `out` held before `n`: an object left by the raster budget lists nothing.
    let listed_before = out.len();
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
    // The embedded images before the adjustment and the expansions of the copies
    // ([`copies_carrying_raster`]) and of the members that hold a random Transform effect, when the
    // raster budget may yet leave `n` as it is: the recolored copies and the images those add then
    // go again.
    let images_before = ((effects::has_adjustment(&n) && !rasterfx::raster_fx(&n).is_empty()) || carries_copies(&n) || holds_seeded(d, &n))
        .then(|| d.images.keys().cloned().collect::<std::collections::HashSet<_>>());
    // What the raster budget had left before those expansions: it gets back what they took.
    let budget_before = (raster.images, raster.pixels);
    // `n` stays as it is: `out` lists nothing of it, and the document and the raster budget lose
    // what expanding it added and took.
    let leave = |d: &mut Document, out: &mut Vec<Listed>, raster: &mut RasterBudget| {
        out.truncate(listed_before);
        if let Some(keys) = &images_before {
            d.images.retain(|k, _| keys.contains(k));
        }
        (raster.images, raster.pixels) = budget_before;
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
    // seed from. Then the object and the marks get ids of their own. When the raster budget leaves
    // a random Transform effect in the object, `n` stays as it is: new ids would give that effect
    // another seed.
    if let Some(mut m) = effects::crop_marks_art(&n) {
        let mut seen = std::collections::HashSet::from([id]);
        let mut expanded = vec![];
        for (i, c) in m.children_mut().into_iter().flatten().enumerate() {
            let c = Arc::make_mut(c);
            if i == 0 {
                let left = raster.left;
                expanded = expand_in_place(d, c, brushes, skip, raster);
                if raster.left > left && has_seeded(c) {
                    leave(d, out, raster);
                    return None;
                }
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
    // New ids of the copies of objects `skip` lists among the members.
    let mut kept = vec![];
    // Members of a group or layer that `expand_pieces` or `copies_carrying_raster` expanded, by id,
    // with what their expansion lists.
    let mut members = std::collections::HashMap::new();
    let copies = copies_carrying_raster(d, &mut n, (brushes, skip), &mut kept, &mut members, (out, &mut listed_n), raster);
    // Raster effects whose image would go past the budget stay: so does the object, with what is
    // inside it and its color adjustments (a symbol instance stays an instance of its symbol), and
    // so does a group or layer whose copies or members keep a random Transform effect the budget
    // left.
    let Ok(image) = copies.and_then(|copied| {
        changed |= copied;
        raster_image(d, &n, raster)
    }) else {
        leave(d, out, raster);
        return None;
    };
    let mut v = n.clone();
    rasterfx::strip_raster(&mut v, false);
    let mut stroke_art =
        |d: &mut Document, path: &PathData, rule: FillRule, st: &StrokeLayer| super::pathops::outlined_stroke(d, brushes, path, rule, st);
    let expanded = match image {
        // Blur, feather and inner glow change the object itself: it becomes the image (a layer
        // keeps it as its only member).
        Some((image, false)) => Some(rasterfx::replace_with_image(v, image)),
        image => {
            let m = if (container || matches!(n.kind, NodeKind::Text(_))) && (effects::has_solid(&v) || effects::has_revolve(&v)) {
                // A 3D effect on a group or layer, or on type: the shaded faces of its solids.
                Ok::<_, OverBudget>(effects::expand_leaf(d, &v, &mut stroke_art))
            } else if container {
                // A group's or layer's own fills and strokes become art among its members.
                effects::evaluate_container(Some(&*d), &v)
                    .map(|mut m| {
                        members.extend(expand_pieces(d, &mut m, false, brushes, skip, &mut kept, &mut stroke_art, raster)?);
                        Ok(m)
                    })
                    .transpose()
            } else if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
                && !effects::has_transform_art(&v)
                && !effects::has_transform_copies(&v)
            {
                Ok(effects::expand_leaf(d, &v, &mut stroke_art))
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
                // A symbol instance whose active fill or stroke has only effects that paint below it
                // (now in the image) becomes its art as well.
                let row_raster = image.is_some() && effects::has_instance_row_raster(&n);
                let outlined = effects::expand_outlined(Some(&*d), &v, symbol.as_ref())
                    .or_else(|| if row_raster { effects::instance_as_art(Some(&*d), &v, symbol.as_ref()) } else { None });
                outlined
                    .map(|mut m| {
                        let mut budget = vectorcraft_doc::MAX_SYMBOL_NODES;
                        expand_nested_instances(d, &mut m, 1, &mut budget, raster);
                        expand_pieces(d, &mut m, true, brushes, skip, &mut kept, &mut stroke_art, raster)?;
                        Ok(m)
                    })
                    .transpose()
            };
            let Ok(m) = m else {
                leave(d, out, raster);
                return None;
            };
            // A 3D effect's shadow blurred by the light's Softness keeps its blur in the art: it
            // expands in turn, to an image.
            let m = if effects::has_solid(&v) || effects::has_revolve(&v) { m.map(|m| expand_raster_pieces(d, m, brushes, skip, raster)) } else { m };
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
        let skip = if kept.is_empty() { std::borrow::Cow::Borrowed(skip) } else { std::borrow::Cow::Owned([skip, &kept].concat()) };
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
                    if let Some(e) = expanded_node(d, (**c).clone(), brushes, &skip, out, raster) {
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
    let roots = edit_targets(s, p, "effect.expandAppearance")?;
    // In Safe Mode a plug-in effect draws nothing: expanding it would drop the effect and keep the
    // geometry it hasn't changed.
    if vectorcraft_plugins::safe_mode::active() {
        // A dimension selected with the art it measures is left as it is, and so is the art of a graph design in a
        // series, unless an effect of a group around it, the selected graph or series group or one inside it, draws it
        // with that group: an effect of the group's own, or a raster effect of one of its fills or strokes (the group
        // then becomes an image of all of it).
        let mut d = super::dimension::without_kept(&s.doc()?.doc, &roots);
        let draws_members = |g: &Node| g.appearance.effects.iter().any(|e| e.visible) || !rasterfx::raster_fx(g).is_empty();
        let designs: Vec<NodeId> = super::appearance::design_art_in(&d, &roots)
            .into_iter()
            .filter(|art| {
                !d.ancestry(*art).is_some_and(|up| {
                    let from = up.iter().position(|a| roots.contains(a)).unwrap_or(up.len());
                    up.get(from..up.len().saturating_sub(1)).is_some_and(|around| around.iter().filter_map(|a| d.node(*a)).any(draws_members))
                })
            })
            .collect();
        if !designs.is_empty() {
            let view = d.to_mut();
            for id in designs {
                // Art already gone (inside other art removed) needs nothing more.
                let _ = view.remove(id);
            }
        }
        let d = &*d;
        let mut seen = std::collections::HashSet::new();
        if roots.iter().filter_map(|id| d.node(*id)).any(|n| holds_plugin_effect(d, n, false, &mut seen)) {
            return Err(EngineError::Other("Expand Appearance: a plug-in effect in the selection can't be expanded in Safe Mode".into()));
        }
    }
    let failed = vectorcraft_render::decode_failures();
    let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
    let mut raster = RasterBudget::new();
    let ids = s.edit("Expand Appearance", |d, _| {
        let before = d.clone();
        // The art of graph designs in the graphs and series among the roots stays as the graph draws it. A group whose
        // own effect turns it into other art as a whole (a blur, feather or inner glow makes it an image) takes its
        // art into that art, which draws it, so that art has no place to go back to.
        let designs = super::appearance::design_art_in(d, &roots);
        // A dimension selected with the art it measures keeps its own lines and label: it is out
        // of the document while the art expands.
        let lifted = super::dimension::lift(d, &roots);
        let kept = !lifted.is_empty();
        let ended = super::dimension::end_links(d, &roots);
        let brushes = vectorcraft_brush::library(d);
        let mut out = vec![];
        let threaded = super::threads::frames(d);
        for id in &roots {
            expand_node_except(d, *id, &brushes, &designs, &mut out, &mut raster);
        }
        super::dimension::put_back(d, lifted);
        // A dimension whose source was replaced stays where it was, no longer linked.
        super::dimension::restore_unchanged(&before, d, ended);
        // Outlined frames of threaded text leave their threads with the text they show; type that
        // stays type under a new id takes its frame's place.
        super::threads::leave_threads(d, &threaded);
        if out.is_empty() || (kept && d.layers == before.layers) {
            if raster.left > 0 {
                return Err(EngineError::Other(RASTER_EFFECTS_LEFT.into()));
            }
            return Err(EngineError::Other("Expand Appearance: the selection has a basic appearance".into()));
        }
        super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad("Expand Appearance", e))?;
        Ok(out)
    })?;
    let mut out = json!({ "ids": ids_json(&ids) });
    // Raster effects become images rendered from the art.
    let warnings: Vec<&str> = rasterfx::undecoded_warning(failed).into_iter().chain((raster.left > 0).then_some(RASTER_EFFECTS_LEFT)).collect();
    if !warnings.is_empty() {
        out["warning"] = json!(warnings.join("; "));
    }
    if raster.left > 0 {
        out["rasterLeft"] = json!(raster.left);
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
