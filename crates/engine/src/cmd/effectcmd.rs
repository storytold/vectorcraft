//! Effect menu commands (live effects) and Object → Expand Appearance.
//!
//! Effects are stored on the object's appearance (`appearance.effects`) and evaluated at render
//! time by `vectorcraft-effects` (re-exported by the renderer). The commands edit the objects'
//! own stacks, groups and layers included (layers through `ids`), or with `target: "contents"`
//! the objects inside them (see `appearance`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_render::effects;

use super::appearance::{ItemTarget, aligned, appearance_targets, index_param, item_target, item_target_at, shared_stack};
use super::expand::has_seeded;
use super::rasterfx;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
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
            "{ids?|id?: object ids (default: the selection), target?: \"object\"|\"contents\"} → {catalog: [{id, label, menu, params, defaults, raster, lengths: {always, absolute (while relative is false)} (distance params Scale Strokes & Effects scales)}] (installed effect plug-ins last: id plugin.<plug-in id>, menu Effect › Plug-ins), applied: [{id, effects, items: [{index, kind: fill|stroke, effects}]}], activeItem} for the selection",
            always,
            list
        ),
        cmd!(
            "effect.remove",
            "Remove Effect",
            [],
            None,
            "{index: int (position in the effect list), item?: appearance item index|null (that fill/stroke's effects; omitted: the active item, else the object's), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} → {ids}",
            has_doc,
            remove
        ),
        cmd!(
            "effect.setParams",
            "Effect Options",
            [],
            None,
            "{index: int, params?: {…} (merged into the current parameters), warpStyle?: string (an existing Warp style suffix, e.g. arc or flag; changes only an applied Warp in place), visible?: bool, item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} → {ids}",
            has_doc,
            set_params
        ),
        cmd!(
            "effect.lightingPreset",
            "3D Lighting Preset",
            [],
            None,
            "{index: int, preset: standard|diffuse|topLeft|right, item?: appearance item index|null (omitted: the active item, else the object's), ids?|id?: [..], target?: object|contents, aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} replace an applied 3D effect's light rig and ambient with an original lighting preset; preserve shading, geometry and view; all targets must have a 3D effect at index; one undo step → {ids}",
            has_doc,
            lighting_preset
        ),
        cmd!(
            "effect.expandAppearance",
            "Expand Appearance",
            ["Object"],
            None,
            "{ids?|id?: [..], target?} turn each object's appearance into objects: every fill and stroke becomes an object of its own (strokes outlined, brushed strokes their brush art) grouped in paint order under the object's id and transparency, geometry effects are baked, raster effects become an embedded image (shadows and outer glows below the art; blur, feather and inner glow replace it; an object whose only raster effect is its own Rasterize becomes the image object.rasterize makes with the effect's options), type with effects or fills of its own is outlined; Crop Marks become a group of the object and its marks; a group's or layer's own fills and strokes become objects among its members, which are expanded too. An outlined frame of threaded text leaves its thread with the text it shows, and a frame kept as type in a group stays in its thread. Hidden fills, strokes and effects are dropped. Enabled when a selected or targeted object's appearance isn't basic (`ids` then pick which objects) → {ids, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
            can_expand,
            expand_appearance
        ),
        cmd!(
            "effect.duplicate",
            "Duplicate Effect",
            [],
            None,
            "{index: int, item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} insert a copy of the effect right after it → {ids}",
            has_doc,
            duplicate
        ),
        cmd!(
            "effect.move",
            "Move Effect",
            [],
            None,
            "{from: int (position in the source list), to: int (its position in the destination list afterwards), fromItem?: appearance item index|null (the source list: that fill/stroke's effects, null the object's; omitted: the active item, else the object's), toItem?: item index|null (the destination list; default: the source list), copy?: bool (copy instead of move, as Alt-dragging the row), ids?|id?: [..], target?: \"object\"|\"contents\", aligned?: bool (refused when the targets' appearances differ)} reorder an effect or move it between the object and its fills/strokes, as one undo step → {ids, index, item}",
            has_doc,
            move_effect
        ),
    ]
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
/// edits its equal item, or its equal object-level effect; a target without them is left out.
pub(crate) fn effect_targets(s: &Session, p: &Value, item: ItemTarget, index: usize, cmd: &str) -> Result<Vec<(NodeId, Option<usize>, usize)>> {
    let roots = appearance_targets(s, p)?;
    let d = &s.doc()?.doc;
    let per = item.per_target(d, &roots);
    let shared = if aligned(p) { shared_stack(d, &roots) } else { None };
    let mut out = Vec::with_capacity(roots.len());
    for (k, (id, item)) in roots.iter().zip(per).enumerate() {
        let (Some(item), Some(n)) = (item, d.node(*id)) else { continue };
        let at = item.effects_item(&n.appearance, cmd)?;
        // An object-level effect the targets share is, in each, where it is there.
        let first = roots.first().and_then(|f| d.node(*f));
        let first_level = first.map(|f| item.effects_item(&f.appearance, cmd)).transpose()?.flatten();
        let index = match (&shared, at, first_level) {
            (Some(sh), None, None) if sh.has_effect(index) => match sh.effect_in(index, k) {
                Some(i) => i,
                None => continue,
            },
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
    let effect = effects::new_effect(id, &params).ok_or_else(|| bad(C, format!("unknown effect `{id}`")))?;
    let label = effects::effect_info(id).map(|e| e.label.trim_end_matches('…').to_string()).unwrap_or_default();
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
        for target in appearance_targets(s, p)? {
            let n = s.doc()?.doc.node(target).ok_or(EngineError::NoNode(target))?;
            effects::validate_three_d(n, id, &effect.params).map_err(|e| bad(C, e))?;
            if id == effects::REVOLVE && effects::has_revolve(n) {
                return Err(bad(C, "A profile supports one Revolve effect. Edit the existing effect's options instead."));
            }
            if effects::has_revolve(n) || effects::has_solid(n) {
                return Err(bad(C, ONE_3D));
            }
        }
    }
    let mut index = 0;
    let ids = edit_effects(s, p, item, C, &label, "Apply Effect: select objects", 0, |fx, _| {
        fx.push(effect.clone());
        index = fx.len() - 1;
        true
    })?;
    // The item the effect landed on in the first object (the active item applies where it fits).
    let first = ids.first().and_then(|id| s.doc().ok()?.doc.node(*id));
    let landed = first.and_then(|n| item.effects_item(&n.appearance, C).ok().flatten());
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
        let fx = |e: &[Effect]| serde_json::to_value(e).unwrap_or(Value::Null);
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

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.remove";
    let index = index_param(p, "index", C)?;
    let item = item_target(s, p, C)?;
    let ids = edit_effects(s, p, item, C, "Remove Effect", &format!("{C}: no effect at index {index}"), index, |fx, index| {
        (index < fx.len()).then(|| fx.remove(index)).is_some()
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.duplicate";
    let index = index_param(p, "index", C)?;
    let item = item_target(s, p, C)?;
    let ids = edit_effects(s, p, item, C, "Duplicate Effect", &format!("{C}: no effect at index {index}"), index, |fx, index| {
        match fx.get(index).cloned() {
            Some(e) => {
                fx.insert(index + 1, e);
                true
            }
            None => false,
        }
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn move_effect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.move";
    let from = index_param(p, "from", C)?;
    let to = index_param(p, "to", C)?;
    let src = item_target_at(s, p, "fromItem", C)?;
    let dst = if p.get("toItem").is_some() { item_target_at(s, p, "toItem", C)? } else { src };
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let roots = appearance_targets(s, p)?;
    // Where an aligned effect would go in each of several different stacks isn't one place.
    if aligned(p) && super::appearance::appearances_differ(&s.doc()?.doc, &roots) {
        return Err(bad(C, "effects of objects whose appearances differ can't be moved together"));
    }
    // Where the effect landed in the first object.
    let mut landed = None;
    let ids = s.edit(if copy { "Duplicate Effect" } else { "Move Effect" }, |d, _| {
        let mut done = vec![];
        for id in &roots {
            let Some(n) = d.node_mut(*id) else { continue };
            let (si, di) = (src.effects_item(&n.appearance, C)?, dst.effects_item(&n.appearance, C)?);
            let Some(fx) = n.appearance.effects_mut(si).filter(|fx| from < fx.len()) else { continue };
            let e = if copy { fx[from].clone() } else { fx.remove(from) };
            let Some(fx) = n.appearance.effects_mut(di) else { continue };
            let at = to.min(fx.len());
            fx.insert(at, e);
            check_revolve_count(n, C)?;
            landed.get_or_insert((at, di));
            done.push(*id);
        }
        if done.is_empty() {
            return Err(EngineError::Other(format!("{C}: no effect at index {from}")));
        }
        Ok(done)
    })?;
    let (index, item) = landed.unzip();
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
    let effect = |&(id, at, index): &(NodeId, Option<usize>, usize)| Some(st.doc.node(id)?.appearance.effects_at(at)?.get(index)?.id.clone());
    targets.iter().filter_map(effect).collect()
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
    }
    let mut options = p.clone();
    let map = options.as_object_mut().ok_or_else(|| bad(C, "parameters must be an object"))?;
    map.remove("preset");
    map.insert("params".into(), patch);
    set_params(s, &options)
}

/// `effect.setParams`: merges `params` into the effect at `index` of the targets' active item, and
/// sets its Warp style (`warpStyle`) or visibility (`visible`), as one undo step. They are checked
/// as `effect.apply` checks them, and a visible 3D effect must still fit. → `{ids}`.
fn set_params(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.setParams";
    let index = index_param(p, "index", C)?;
    let mut params = p.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err(bad(C, "`params` must be an object"));
    }
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
    for (id, target, index) in effect_targets(s, p, item, index, C)? {
        let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
        let effect = n.appearance.effects_at(target).and_then(|fx| fx.get(index));
        if warp_id.is_some() && effect.is_some_and(|e| e.id.strip_prefix("warp.").and_then(effects::WarpStyle::from_id).is_none()) {
            return Err(bad(C, "warpStyle can only edit an applied Warp effect"));
        }
        if let Some(e) = effect
            && effects::is_three_d(&e.id)
            && visible != Some(false)
        {
            let mut prospective = effects::merged_params(&e.id, &e.params);
            if let (Some(cur), Value::Object(new)) = (prospective.as_object_mut(), effects::upgrade_params(&e.id, &params)) {
                cur.extend(new);
            }
            effects::validate_three_d(n, &e.id, &prospective).map_err(|e| bad(C, e))?;
        }
    }
    let ids = edit_effects(s, p, item, C, "Effect Options", &format!("{C}: no effect at index {index}"), index, |fx, index| {
        let Some(e) = fx.get_mut(index) else { return false };
        if let Some(id) = &warp_id {
            e.id.clone_from(id);
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
/// fills, strokes or effects of its own or such members, other objects with effects, or type with
/// fills or strokes of its own?
fn expandable(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { clipping: true, .. } | NodeKind::Path { guide: true, .. } => false,
        NodeKind::Path { .. } | NodeKind::Compound { .. } => !n.appearance.is_basic() || vectorcraft_brush::has_brush(n),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            !n.appearance.items.is_empty() || !n.appearance.effects.is_empty() || children.iter().any(|c| expandable(c))
        }
        NodeKind::Text(_) => !n.appearance.items.is_empty() || !n.appearance.effects.is_empty(),
        NodeKind::Image(_) => effects::has_item_geometry(n) || !n.appearance.effects.is_empty(),
        _ => !n.appearance.effects.is_empty(),
    }
}

fn can_expand(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let d = &s.doc().map_err(|e| e.to_string())?.doc;
    let ids = appearance_targets(s, &Value::Null).map_err(|e| e.to_string())?;
    if ids.iter().filter_map(|id| d.node(*id)).any(expandable) {
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
/// stroke becomes an object of its own (strokes outlined, brushes as their art), geometry effects
/// are baked, raster effects become an embedded image, and the object keeps its transparency.
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
    if effects::has_adjustment(&n)
        && let Some(m) = effects::adjust_in_document(d, &n)
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
            let m = if container {
                // A group's or layer's own fills and strokes become art among its members.
                effects::evaluate_container(Some(&*d), &v).map(|mut m| {
                    members = expand_pieces(d, &mut m, false, brushes, &mut stroke_art);
                    m
                })
            } else if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && !effects::has_transform_art(&v) {
                effects::expand_leaf(d, &v, &mut stroke_art)
            } else {
                // Type, images, symbol instances, live objects (and paths whose Transform effect copies
                // them with a gradient or changes their fills and strokes): through their outlines.
                let symbol = match &n.kind {
                    NodeKind::SymbolInstance { symbol, .. } => d.symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
                    _ => None,
                };
                effects::expand_outlined(Some(&*d), &v, symbol.as_deref()).map(|mut m| {
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
            at += c.count();
        }
    }
    changed.then_some(n)
}

fn expand_appearance(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = appearance_targets(s, p)?;
    let failed = vectorcraft_render::decode_failures();
    let ids = s.edit("Expand Appearance", |d, _| {
        let brushes = vectorcraft_brush::library(d);
        let mut out = vec![];
        let threaded = super::threads::frames(d);
        for id in &roots {
            expand_node(d, *id, &brushes, &mut out);
        }
        // Outlined frames of threaded text leave their threads with the text they show; type that
        // stays type under a new id takes its frame's place.
        super::threads::leave_threads(d, &threaded);
        if out.is_empty() {
            return Err(EngineError::Other("Expand Appearance: the selection has a basic appearance".into()));
        }
        Ok(out)
    })?;
    let mut out = json!({ "ids": ids_json(&ids) });
    // Raster effects become images rendered from the art.
    if let Some(w) = rasterfx::undecoded_warning(failed) {
        out["warning"] = json!(w);
    }
    Ok(out)
}

#[cfg(test)]
mod lighting_presets_tests;

#[cfg(test)]
mod tests;
