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

use super::appearance::{ItemTarget, appearance_targets, index_param, item_target, item_target_at};
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
            "{index: int (position in the effect list), item?: appearance item index|null (that fill/stroke's effects; omitted: the active item, else the object's), ids?|id?: [..], target?: \"object\"|\"contents\"} → {ids}",
            has_doc,
            remove
        ),
        cmd!(
            "effect.setParams",
            "Effect Options",
            [],
            None,
            "{index: int, params?: {…} (merged into the current parameters), warpStyle?: string (an existing Warp style suffix, e.g. arc or flag; changes only an applied Warp in place), visible?: bool, item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\"} → {ids}",
            has_doc,
            set_params
        ),
        cmd!(
            "effect.expandAppearance",
            "Expand Appearance",
            ["Object"],
            None,
            "{ids?|id?: [..], target?} turn each object's appearance into objects: every fill and stroke becomes an object of its own (strokes outlined, brushed strokes their brush art) grouped in paint order under the object's id and transparency, geometry effects are baked, raster effects become an embedded image (shadows and outer glows below the art; blur, feather and inner glow replace it; an object whose only raster effect is its own Rasterize becomes the image object.rasterize makes with the effect's options), type with effects or fills of its own is outlined; Crop Marks become a group of the object and its marks; a group's or layer's own fills and strokes become objects among its members, which are expanded too. Hidden fills, strokes and effects are dropped. Enabled when a selected or targeted object's appearance isn't basic (`ids` then pick which objects) → {ids, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
            can_expand,
            expand_appearance
        ),
        cmd!(
            "effect.duplicate",
            "Duplicate Effect",
            [],
            None,
            "{index: int, item?: appearance item index|null (as effect.remove), ids?|id?: [..], target?: \"object\"|\"contents\"} insert a copy of the effect right after it → {ids}",
            has_doc,
            duplicate
        ),
        cmd!(
            "effect.move",
            "Move Effect",
            [],
            None,
            "{from: int (position in the source list), to: int (its position in the destination list afterwards), fromItem?: appearance item index|null (the source list: that fill/stroke's effects, null the object's; omitted: the active item, else the object's), toItem?: item index|null (the destination list; default: the source list), copy?: bool (copy instead of move, as Alt-dragging the row), ids?|id?: [..], target?: \"object\"|\"contents\"} reorder an effect or move it between the object and its fills/strokes, as one undo step → {ids, index, item}",
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
fn edit_effects(
    s: &mut Session,
    p: &Value,
    item: ItemTarget,
    cmd: &str,
    label: &str,
    none: &str,
    mut f: impl FnMut(&mut Vec<Effect>) -> bool,
) -> Result<Vec<NodeId>> {
    let roots = appearance_targets(s, p)?;
    s.edit(label, |d, _| {
        let mut done = vec![];
        for id in &roots {
            let Some(n) = d.node_mut(*id) else { continue };
            let index = item.effects_item(&n.appearance, cmd)?;
            if n.appearance.effects_mut(index).is_some_and(&mut f) {
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

const ONE_3D: &str = "An object takes one 3D effect. Edit the existing effect's options, or remove it first.";

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
    if id == GLASS {
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
            if id == effects::REVOLVE {
                effects::validate_revolve(n, &effect.params).map_err(|e| bad(C, e))?;
                if effects::has_revolve(n) {
                    return Err(bad(C, "A profile supports one Revolve effect. Edit the existing effect's options instead."));
                }
            } else {
                effects::validate_solid(n, id, &effect.params).map_err(|e| bad(C, e))?;
            }
            if effects::has_revolve(n) || effects::has_solid(n) {
                return Err(bad(C, ONE_3D));
            }
        }
    }
    let mut index = 0;
    let ids = edit_effects(s, p, item, C, &label, "Apply Effect: select objects", |fx| {
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
    let ids = edit_effects(s, p, item, C, "Remove Effect", &format!("{C}: no effect at index {index}"), |fx| {
        (index < fx.len()).then(|| fx.remove(index)).is_some()
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.duplicate";
    let index = index_param(p, "index", C)?;
    let item = item_target(s, p, C)?;
    let ids = edit_effects(s, p, item, C, "Duplicate Effect", &format!("{C}: no effect at index {index}"), |fx| match fx.get(index).cloned() {
        Some(e) => {
            fx.insert(index + 1, e);
            true
        }
        None => false,
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
    let (Ok(index), Ok(item), Ok(ids), Ok(st)) = (index_param(p, "index", C), item_target(s, p, C), appearance_targets(s, p), s.doc()) else {
        return vec![];
    };
    let effect = |id: &NodeId| {
        let ap = &st.doc.node(*id)?.appearance;
        Some(ap.effects_at(item.effects_item(ap, C).ok()?)?.get(index)?.id.clone())
    };
    ids.iter().filter_map(effect).collect()
}

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
    if edited.iter().any(|e| e == GLASS) {
        load_texture(&mut params, C)?;
    }
    for id in appearance_targets(s, p)? {
        let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
        let target = item.effects_item(&n.appearance, C)?;
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
            if e.id == effects::REVOLVE {
                effects::validate_revolve(n, &prospective).map_err(|e| bad(C, e))?;
            } else {
                effects::validate_solid(n, &e.id, &prospective).map_err(|e| bad(C, e))?;
            }
        }
    }
    let ids = edit_effects(s, p, item, C, "Effect Options", &format!("{C}: no effect at index {index}"), |fx| {
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

/// Give the members of `m`, art evaluated from the object of the same id (its pieces share that
/// id), ids of their own and expand the pieces: those made from its own fills and strokes, or with
/// `all` every member.
fn expand_pieces(d: &mut Document, m: &mut Node, all: bool, stroke_art: effects::StrokeArt) {
    let id = m.id;
    let mut seen = std::collections::HashSet::from([id]);
    for c in m.children_mut().into_iter().flatten() {
        let own = all || c.id == id;
        let c = Arc::make_mut(c);
        effects::fresh_ids(d, c, &mut seen);
        if own {
            effects::expand_art(d, c, stroke_art);
        }
    }
}

/// Expand Appearance of `id` (and, recursively, of a group's or layer's members): every fill and
/// stroke becomes an object of its own (strokes outlined, brushes as their art), geometry effects
/// are baked, raster effects become an embedded image, and the object keeps its transparency.
pub(crate) fn expand_node(d: &mut Document, id: NodeId, brushes: &[vectorcraft_brush::Brush], out: &mut Vec<NodeId>) {
    let Some(mut n) = d.node(id).cloned() else { return };
    if !expandable(&n) {
        return;
    }
    // Colour adjustments: the colours inside become the adjusted ones (an embedded image a
    // recoloured copy), then the rest of the appearance expands.
    if effects::has_adjustment(&n)
        && let Some(m) = effects::adjust_in_document(d, &n)
        && let Some(slot) = d.node_mut(id)
    {
        *slot = m.clone();
        out.push(id);
        n = m;
        if !expandable(&n) {
            return;
        }
    }
    // (A symbol instance has become its art, a group.)
    let container = is_container(&n);
    // Crop Marks: a group of the object and its marks, then the object expands on its own.
    if let Some(mut m) = effects::crop_marks_art(&n) {
        let mut seen = std::collections::HashSet::from([id]);
        for c in m.children_mut().into_iter().flatten() {
            effects::fresh_ids(d, Arc::make_mut(c), &mut seen);
        }
        let object = m.children().and_then(|c| c.first()).map(|c| c.id);
        if let Some(slot) = d.node_mut(id) {
            *slot = m;
            out.push(id);
        }
        if let Some(object) = object {
            expand_node(d, object, brushes, out);
        }
        return;
    }
    let image = raster_image(d, &n);
    let mut v = n.clone();
    rasterfx::strip_raster(&mut v, false);
    let mut stroke_art =
        |d: &mut Document, path: &PathData, rule: FillRule, st: &StrokeLayer| super::pathops::outlined_stroke(d, brushes, path, rule, st);
    let expanded = match image {
        // Blur, feather and inner glow change the object itself: it becomes the image (a layer
        // keeps it as its only member).
        Some((image, false)) => Some(rasterfx::replace_with_image(v, image)),
        image => {
            let m = if container {
                // A group's or layer's own fills and strokes become art among its members.
                effects::evaluate_container(&v).map(|mut m| {
                    expand_pieces(d, &mut m, false, &mut stroke_art);
                    m
                })
            } else if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && !effects::has_gradient_copies(&v) {
                effects::expand_leaf(d, &v, &mut stroke_art)
            } else {
                // Type, images, symbol instances, live objects (and paths whose Transform effect copies
                // them with a gradient): through their outlines.
                let symbol = match &n.kind {
                    NodeKind::SymbolInstance { symbol, .. } => d.symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
                    _ => None,
                };
                effects::expand_outlined(&v, symbol.as_deref()).map(|mut m| {
                    expand_pieces(d, &mut m, true, &mut stroke_art);
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
    if let Some(m) = expanded
        && let Some(slot) = d.node_mut(id)
    {
        *slot = m;
        if out.last() != Some(&id) {
            out.push(id);
        }
    }
    if container {
        let children: Vec<NodeId> = d.node(id).and_then(|n| n.children()).map(|c| c.iter().map(|c| c.id).collect()).unwrap_or_default();
        for c in children {
            expand_node(d, c, brushes, out);
        }
    }
}

fn expand_appearance(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = appearance_targets(s, p)?;
    let failed = vectorcraft_render::decode_failures();
    let ids = s.edit("Expand Appearance", |d, _| {
        let brushes = vectorcraft_brush::library(d);
        let mut out = vec![];
        for id in &roots {
            expand_node(d, *id, &brushes, &mut out);
        }
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
mod tests {
    use serde_json::json;
    use vectorcraft_doc::NodeKind;
    use vectorcraft_render::effects;

    use crate::{NodeId, Session};

    fn session_with_rect() -> (Session, NodeId) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        (s, NodeId(r["id"].as_u64().unwrap()))
    }

    fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
        s.doc().unwrap().doc.node(id).cloned().unwrap()
    }

    #[test]
    fn revolve_commands_keep_profile_live_edit_options_expand_and_undo() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
        let r = s.execute("path.create", &json!({"d":"M 150 40 L 180 60 L 180 140 L 150 160"})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        let source = node(&s, id).path_data().unwrap().clone();
        s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null})).unwrap();
        let before = effects_for_test(&s, id);
        assert_eq!(node(&s, id).path_data(), Some(&source));
        let doc = &s.doc().unwrap().doc;
        let restored = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
        assert_eq!(restored.node(id).unwrap().path_data(), Some(&source));
        assert_eq!(effects::revolve_art(restored.node(id).unwrap()).unwrap(), before);
        let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
        let bytes = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        assert!(s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null})).is_err());
        assert!(s.execute("effect.duplicate", &json!({"index":0,"ids":[id.0],"item":null})).is_err());
        s.execute("effect.setParams", &json!({"index":0,"params":{"angle":180,"offset":10},"ids":[id.0],"item":null})).unwrap();
        assert_ne!(effects_for_test(&s, id), before);
        assert!(s.execute("effect.setParams", &json!({"index":0,"params":{"edge":"invalid"},"ids":[id.0],"item":null})).is_err());
        s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
        assert!(matches!(node(&s, id).kind, NodeKind::Group { .. }));
        assert!(!effects::has_revolve(&node(&s, id)));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id).path_data(), Some(&source));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(effects_for_test(&s, id), before);
    }

    /// Extrude & Bevel, Inflate and Rotate: applied by command, saved and reopened, edited,
    /// exported, expanded and undone; one 3D effect per object.
    #[test]
    fn solid_effects_apply_save_edit_export_expand_and_undo() {
        for effect in effects::SOLIDS {
            let (mut s, id) = session_with_rect();
            let source = node(&s, id).path_data().unwrap().clone();
            s.execute("effect.apply", &json!({"effect": effect, "ids": [id.0], "item": null})).unwrap();
            let art = effects::solid_art(&node(&s, id)).unwrap();
            assert_eq!(node(&s, id).path_data(), Some(&source), "{effect}: the source stays editable");
            let doc = &s.doc().unwrap().doc;
            let restored = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
            assert_eq!(effects::solid_art(restored.node(id).unwrap()).unwrap(), art, "{effect}: reopens the same");
            // One 3D effect per object, whichever kind.
            for other in ["threeD.revolve", "threeD.extrude", "threeD.rotate"] {
                assert!(s.execute("effect.apply", &json!({"effect": other, "ids": [id.0], "item": null})).is_err(), "{effect} + {other}");
            }
            s.execute("effect.setParams", &json!({"index": 0, "params": {"rotationY": 50, "rotationX": 10}, "ids": [id.0], "item": null})).unwrap();
            assert_ne!(effects::solid_art(&node(&s, id)).unwrap(), art, "{effect}: options change the art");
            assert!(s.execute("effect.setParams", &json!({"index": 0, "params": {"bevel": "nope"}, "ids": [id.0], "item": null})).is_err());
            let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
            let svg = svg["data"].as_str().or_else(|| svg["text"].as_str()).map(str::to_string).unwrap_or_else(|| svg.to_string());
            assert!(svg.matches("<path").count() >= 2, "{effect}: exported as vector faces");
            s.execute("effect.expandAppearance", &json!({"ids": [id.0]})).unwrap();
            assert!(matches!(node(&s, id).kind, NodeKind::Group { .. }), "{effect}");
            assert!(!effects::has_solid(&node(&s, id)));
            s.execute("edit.undo", &json!({})).unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(node(&s, id).appearance.effects.len(), 0, "{effect}: back to the plain rectangle");
        }
        // Type has no outline to extrude.
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let t = s.execute("text.create", &json!({"x": 50, "y": 50, "text": "3D"})).unwrap();
        let err = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "ids": [t["id"]], "item": null})).unwrap_err();
        assert!(err.to_string().contains("requires a path"), "{err}");
    }

    /// Scripts from before the Perspective angle give `perspective` as a 0–100 % strength: it is
    /// stored as the angle that shows the same view, and a later angle replaces an earlier strength.
    #[test]
    fn an_earlier_perspective_strength_is_set_as_its_angle() {
        let (mut s, id) = session_with_rect();
        s.execute("effect.apply", &json!({"effect": effects::EXTRUDE, "ids": [id.0], "item": null, "params": {"perspective": 60}})).unwrap();
        let params = |s: &Session| node(s, id).appearance.effects[0].params.clone();
        let angle_of = |pct: f64| effects::merged_params(effects::EXTRUDE, &json!({"perspective": pct}))["perspectiveAngle"].as_f64().unwrap();
        let angle = angle_of(60.0);
        assert!((angle - 30.12).abs() < 0.01, "{angle}");
        assert!(
            params(&s).get("perspective").is_none() && (params(&s)["perspectiveAngle"].as_f64().unwrap() - angle).abs() < 1e-12,
            "{}",
            params(&s)
        );
        s.execute("effect.setParams", &json!({"index": 0, "params": {"perspective": 100}, "ids": [id.0], "item": null})).unwrap();
        assert!((params(&s)["perspectiveAngle"].as_f64().unwrap() - angle_of(100.0)).abs() < 1e-12);
        // A file of then: the strength stored; setting an angle replaces it.
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance.effects[0].params =
            json!({"depth": 50, "perspective": 60});
        s.execute("effect.setParams", &json!({"index": 0, "params": {"perspectiveAngle": 120}, "ids": [id.0], "item": null})).unwrap();
        assert_eq!(params(&s), json!({"depth": 50, "perspectiveAngle": 120}));
    }

    /// Scripts from before Rotation and Height give the light by `lightAzimuth` and
    /// `lightElevation`: stored as the same light; a later Height keeps that light's Rotation.
    #[test]
    fn an_earlier_light_direction_is_set_as_rotation_and_height() {
        let (mut s, id) = session_with_rect();
        s.execute(
            "effect.apply",
            &json!({"effect": effects::ROTATE, "ids": [id.0], "item": null, "params": {"lightAzimuth": 180, "lightElevation": 0}}),
        )
        .unwrap();
        let params = |s: &Session| node(s, id).appearance.effects[0].params.clone();
        // Straight from behind the object, level with it.
        assert!(params(&s).get("lightAzimuth").is_none() && params(&s).get("lightElevation").is_none(), "{}", params(&s));
        assert_eq!((params(&s)["lightHeight"].as_f64().unwrap().round(), &params(&s)["lightBehind"]), (90.0, &json!(true)));
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance.effects[0].params =
            json!({"lightAzimuth": 90, "lightElevation": 0});
        s.execute("effect.setParams", &json!({"index": 0, "params": {"lightHeight": 60}, "ids": [id.0], "item": null})).unwrap();
        assert_eq!(params(&s), json!({"lightRotation": 0.0, "lightHeight": 60, "lightBehind": false}));
    }

    /// Scripts from before Scribble's separate variations give one `variation`, which is stored as the
    /// Path Overlap and Spacing variations. An angle is stored above -180° and up to 180°.
    #[test]
    fn scribble_stores_an_earlier_variation_as_two_and_its_angle_within_180() {
        let (mut s, id) = session_with_rect();
        let p = json!({"effect": "stylize.scribble", "ids": [id.0], "item": null, "params": {"angle": 270, "variation": 2}});
        s.execute("effect.apply", &p).unwrap();
        let params = |s: &Session| node(s, id).appearance.effects[0].params.clone();
        assert_eq!((params(&s)["angle"].as_f64(), params(&s).get("variation")), (Some(-90.0), None), "{}", params(&s));
        assert_eq!((&params(&s)["overlapVariation"], &params(&s)["spacingVariation"]), (&json!(2), &json!(2)));
        s.execute("effect.setParams", &json!({"index": 0, "params": {"angle": -200}, "ids": [id.0], "item": null})).unwrap();
        assert_eq!(params(&s)["angle"].as_f64(), Some(160.0));
        // A file of then stores its variation; setting one of the two keeps it for the other.
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance.effects[0].params = json!({"variation": 3});
        s.execute("effect.setParams", &json!({"index": 0, "params": {"spacingVariation": 1}, "ids": [id.0], "item": null})).unwrap();
        assert_eq!(params(&s), json!({"overlapVariation": 3, "spacingVariation": 1}));
    }

    fn effects_for_test(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
        effects::revolve_art(&node(s, id)).unwrap()
    }

    #[test]
    fn smooth_revolve_commands_validate_save_expand_and_undo() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
        let r = s.execute("path.create", &json!({"d":"M150 40 C190 60 190 140 150 160"})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null,"params":{"offset":20,"segments":16}})).unwrap();
        let initial = node(&s, id);
        let source = initial.path_data().unwrap().clone();
        assert!(!initial.appearance.effects[0].params.as_object().unwrap().contains_key("smoothShading"));
        assert!(initial.appearance.effects[0].params["cap"].as_bool().unwrap());
        let mut gradients = 0;
        effects_for_test(&s, id).walk(&mut |face| {
            gradients += face.appearance.items.iter().filter(|item| {
                matches!(item, vectorcraft_doc::AppearanceItem::Fill(fill) if matches!(fill.paint, vectorcraft_doc::color::Paint::Gradient(_)))
            }).count();
        });
        assert!(gradients > 0, "commands use smooth vector shading by default");
        s.execute("effect.setParams", &json!({"index":0,"ids":[id.0],"item":null,"params":{"lightAzimuth":30}})).unwrap();
        let smooth = node(&s, id);
        assert_eq!(smooth.path_data(), Some(&source));
        assert_ne!(effects_for_test(&s, id), effects::revolve_art(&initial).unwrap());
        let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        assert_eq!(effects::revolve_art(restored.node(id).unwrap()), Some(effects_for_test(&s, id)));
        s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
        assert!(!effects::has_revolve(&node(&s, id)));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id), smooth);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id), initial);
    }

    #[test]
    fn revolve_caps_save_export_expand_and_undo_without_changing_the_source() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
        let r = s.execute("path.create", &json!({"d":"M150 40 L150 160"})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        let source = node(&s, id).path_data().unwrap().clone();
        s.execute(
            "effect.apply",
            &json!({"effect":"threeD.revolve","ids":[id.0],"item":null,
            "params":{"offset":30,"rotationX":-35,"segments":16,"expandVisibleOnly":false,"cap":false}}),
        )
        .unwrap();
        let uncapped = node(&s, id);
        assert_eq!(uncapped.appearance.effects[0].params["cap"], json!(false));
        let count = |n: &vectorcraft_doc::Node| n.children().unwrap()[0].children().unwrap().len();
        let open_count = count(&effects_for_test(&s, id));
        s.execute("effect.setParams", &json!({"index":0,"ids":[id.0],"item":null,"params":{"cap":true}})).unwrap();
        let capped = node(&s, id);
        let art = effects_for_test(&s, id);
        assert_eq!(count(&art), open_count + 2, "each endpoint adds one continuous vector cap");
        assert_eq!(capped.path_data(), Some(&source));
        assert!(s.execute("effect.setParams", &json!({"index":0,"ids":[id.0],"item":null,"params":{"cap":"yes"}})).is_err());
        assert_eq!(node(&s, id), capped, "a bad boolean does not mutate the document");
        let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        assert_eq!(restored.node(id).unwrap().appearance.effects[0].params["cap"], json!(true));
        assert_eq!(effects::revolve_art(restored.node(id).unwrap()), Some(art.clone()));
        let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
        assert!(vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap().starts_with(b"%PDF"));
        s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
        assert_eq!(count(&node(&s, id)), count(&art));
        assert!(!effects::has_revolve(&node(&s, id)));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id), capped);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id), uncapped);
    }

    #[test]
    fn revolve_visibility_parameter_saves_expands_and_undoes_without_other_appearance_changes() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
        let r = s.execute("path.create", &json!({"d":"M 150 40 L 180 60 L 180 140 L 150 160"})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null})).unwrap();
        let live = node(&s, id);
        let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        assert_eq!(restored.node(id).unwrap().appearance.effects[0].params["expandVisibleOnly"], json!(true));
        let count = |n: &vectorcraft_doc::Node| n.children().unwrap()[0].children().unwrap().len();
        s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
        let visible = count(&node(&s, id));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id), live);
        s.execute("effect.setParams", &json!({"index":0,"item":null,"ids":[id.0],"params":{"expandVisibleOnly":false}})).unwrap();
        s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
        assert!(count(&node(&s, id)) > visible);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id).appearance.effects[0].params["expandVisibleOnly"], json!(false));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id), live);
        // The shared command still expands ordinary multiple-fill artwork as before.
        let r = s.execute("shape.rectangle", &json!({"x":10,"y":10,"width":20,"height":20})).unwrap();
        let ordinary = NodeId(r["id"].as_u64().unwrap());
        s.execute("effect.apply", &json!({"effect":"distort.twist","ids":[ordinary.0],"params":{"angle":15}})).unwrap();
        s.execute("effect.expandAppearance", &json!({"ids":[ordinary.0]})).unwrap();
        assert!(node(&s, ordinary).appearance.effects.is_empty());
    }

    #[test]
    fn apply_uses_catalog_defaults_and_undoes() {
        let (mut s, id) = session_with_rect();
        let r = s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "params": {"x": 3}})).unwrap();
        assert_eq!(r["index"], json!(0));
        let n = node(&s, id);
        assert_eq!(n.appearance.effects.len(), 1);
        assert_eq!(n.appearance.effects[0].params["x"], json!(3));
        assert_eq!(n.appearance.effects[0].params["opacity"], json!(75.0));
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(node(&s, id).appearance.effects.is_empty());
        assert!(s.execute("effect.apply", &json!({"effect": "nope"})).is_err());
    }

    #[test]
    fn warp_options_style_change_preserves_slot_and_visibility() {
        let (mut s, id) = session_with_rect();
        for effect in ["path.offsetPath", "warp.arc", "stylize.roundCorners"] {
            s.execute("effect.apply", &json!({"effect": effect, "item": 1})).unwrap();
        }
        s.execute("effect.setParams", &json!({"index": 1, "item": 1, "visible": false})).unwrap();
        s.doc_mut().unwrap().mark_saved();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let result = s.execute("effect.setParams", &json!({"index": 1, "item": 1, "warpStyle": "flag", "params": {"bend": 25.0}}));
        assert!(result.is_ok(), "an applied warp can change style in place: {result:?}");
        let old = before.node(id).unwrap().appearance.effects_at(Some(1)).unwrap();
        let n = node(&s, id);
        let new = n.appearance.effects_at(Some(1)).unwrap();
        assert_eq!(new.len(), 3);
        assert_eq!(new[0], old[0]);
        assert_eq!(new[2], old[2]);
        assert_eq!(new[1].id, "warp.flag");
        assert_eq!(new[1].params["bend"], 25.0);
        assert!(!new[1].visible);
        assert!(n.appearance.effects.is_empty());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        let after = s.doc().unwrap().doc.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        assert!(!s.doc().unwrap().is_dirty());
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, after);
    }

    #[test]
    fn warp_options_non_warp_target_rejection_is_atomic() {
        let (mut s, first) = session_with_rect();
        s.execute("effect.apply", &json!({"effect": "warp.arc", "item": null})).unwrap();
        let second = s.execute("shape.rectangle", &json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
        s.execute("effect.apply", &json!({"effect": "distort.twist", "item": null})).unwrap();
        s.execute("select.set", &json!({"ids": [first.0, second]})).unwrap();
        s.doc_mut().unwrap().mark_saved();
        let before = s.doc().unwrap().doc.clone();
        let selection = s.doc().unwrap().selection.clone();
        let undo = s.doc().unwrap().history.undo.len();
        assert!(s.execute("effect.setParams", &json!({"index": 0, "item": null, "warpStyle": "flag"})).is_err());
        let state = s.doc().unwrap();
        assert_eq!(state.doc, before);
        assert_eq!(state.selection, selection);
        assert_eq!(state.history.undo.len(), undo);
        assert!(!state.is_dirty());
    }

    #[test]
    fn list_remove_and_set_params() {
        let (mut s, id) = session_with_rect();
        s.execute("effect.apply", &json!({"effect": "distort.twist"})).unwrap();
        s.execute("effect.apply", &json!({"effect": "stylize.outerGlow"})).unwrap();
        let l = s.execute("effect.list", &json!({})).unwrap();
        assert!(l["catalog"].as_array().unwrap().len() >= 34);
        assert_eq!(l["applied"][0]["effects"].as_array().unwrap().len(), 2);
        s.execute("effect.setParams", &json!({"index": 0, "params": {"angle": 45}, "visible": false})).unwrap();
        let e = &node(&s, id).appearance.effects[0];
        assert_eq!(e.params["angle"], json!(45));
        assert!(!e.visible);
        s.execute("effect.remove", &json!({"index": 0})).unwrap();
        let n = node(&s, id);
        assert_eq!(n.appearance.effects.len(), 1);
        assert_eq!(n.appearance.effects[0].id, "stylize.outerGlow");
        assert!(s.execute("effect.remove", &json!({"index": 5})).is_err());
    }

    #[test]
    fn expand_appearance_bakes_geometry() {
        let (mut s, id) = session_with_rect();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        s.execute("effect.apply", &json!({"effect": "path.offsetPath", "params": {"offset": 10}})).unwrap();
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        // One fill: the path itself, reshaped.
        let n = node(&s, id);
        let b = n.geometric_bounds().unwrap();
        assert!((b.width() - 120.0).abs() < 0.1, "{b:?}");
        assert!(n.appearance.effects.is_empty());
        assert!(matches!(n.kind, NodeKind::Path { live: None, .. }));
        // Nothing left to expand.
        assert!(s.execute("effect.expandAppearance", &json!({})).is_err());
    }

    #[test]
    fn expand_transform_copies_makes_compound() {
        let (mut s, id) = session_with_rect();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 120, "copies": 1}})).unwrap();
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let n = node(&s, id);
        assert!(matches!(n.kind, NodeKind::Compound { .. }));
        let b = n.geometric_bounds().unwrap();
        assert!((b.x1 - 320.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn pathfinder_effect_on_loose_objects_groups_them_first() {
        let (mut s, a) = session_with_rect();
        let b = NodeId(s.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        let r = s.execute("effect.apply", &json!({"effect": "pathfinder.add"})).unwrap();
        assert_eq!(r["grouped"], true);
        let g = NodeId(r["ids"][0].as_u64().unwrap());
        let gn = node(&s, g);
        assert_eq!(gn.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [a, b], "stacking order kept");
        assert!(vectorcraft_render::effects::has_pathfinder(&gn));
        assert_eq!(s.doc().unwrap().selection.objects, [g]);
        // Live: one united shape, so expanding leaves a single path.
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        assert_eq!(node(&s, g).children().unwrap().len(), 1);
        // Grouping and applying are one undo step.
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert!(d.node(g).is_none());
        assert!(d.node(a).is_some_and(|n| n.appearance.effects.is_empty()) && d.parent_of(a) == d.parent_of(b));
        // A selection holding a group applies to each object as before; so does a single object.
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        let r = s.execute("effect.apply", &json!({"effect": "pathfinder.add"})).unwrap();
        assert!(r.get("grouped").is_none());
        assert_eq!(node(&s, a).appearance.effects.len(), 1);
    }

    #[test]
    fn pathfinder_effect_on_a_group_with_type_cuts_the_glyphs_out() {
        let (mut s, a) = session_with_rect();
        let t = NodeId(s.execute("text.create", &json!({"x": 110, "y": 180, "text": "H", "size": 100})).unwrap()["id"].as_u64().unwrap());
        let glyphs = effects::outline_text(&node(&s, t)).unwrap();
        let b = glyphs.geometric_bounds().unwrap();
        assert!(b.x0 > 105.0 && b.y0 > 105.0 && b.x1 < 195.0 && b.y1 < 195.0, "the type lies inside the square: {b:?}");
        let area = |n: &vectorcraft_doc::Node| effects::member_shapes(n).iter().map(|(p, r)| vectorcraft_pathops::area(p, *r)).sum::<f64>();
        let glyph_area = area(&glyphs);
        s.execute("select.set", &json!({"ids": [a.0, t.0]})).unwrap();
        let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("effect.apply", &json!({"effect": "pathfinder.subtract"})).unwrap();
        // Live: the renderer leaves the glyph's place in the square empty.
        let doc = s.doc().unwrap().doc.clone();
        let img = vectorcraft_render::Renderer::new().render(&doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        let empty = (105..195).flat_map(|y| (105..195).map(move |x| (x, y))).filter(|&(x, y)| img.pixel(x, y)[3] == 0).count();
        assert!(empty as f64 > glyph_area / 2.0, "{empty} empty pixels for a glyph area of {glyph_area}");
        // Expanded: the group holds the square minus the glyph.
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let left = area(&node(&s, g));
        assert!((left - (10_000.0 - glyph_area)).abs() < 0.5, "{left} left of the square for a glyph area of {glyph_area}");
    }

    #[test]
    fn pathfinder_effect_on_a_group_renders_live_and_expands() {
        let (mut s, a) = session_with_rect();
        let b = s.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
        s.execute("select.set", &json!({"ids": [a.0, b["id"]]})).unwrap();
        let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("effect.apply", &json!({"effect": "pathfinder.subtract"})).unwrap();
        // Live: the renderer shows only the back square minus the front one.
        let doc = s.doc().unwrap().doc.clone();
        let img = vectorcraft_render::Renderer::new().render(&doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert!(img.pixel(125, 150)[3] > 0);
        assert_eq!(img.pixel(200, 150)[3], 0);
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let n = node(&s, g);
        assert!(n.appearance.effects.is_empty());
        let ch = n.children().unwrap();
        assert_eq!(ch.len(), 1);
        assert!((ch[0].geometric_bounds().unwrap().width() - 50.0).abs() < 1e-6);
    }
    #[test]
    fn expand_appearance_gives_each_copy_of_a_transform_its_own_gradient() {
        use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
        let (mut s, id) = session_with_rect();
        let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
        let ramp = Paint::Gradient(Box::new(GradientPaint::new(Gradient { kind: GradientKind::Linear, stops })));
        s.edit("gradient", |d, _| {
            let n = d.node_mut(id).unwrap();
            n.appearance.set_fill(ramp);
            Ok(())
        })
        .unwrap();
        s.execute("effect.apply", &json!({"effect":"distort.transform","ids":[id.0],"item":null})).unwrap();
        s.execute("effect.setParams", &json!({"index":0,"params":{"moveH":150,"copies":2},"ids":[id.0],"item":null})).unwrap();
        s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
        let mut boxes = vec![];
        node(&s, id).walk(&mut |n| {
            if matches!(n.kind, NodeKind::Path { .. }) && matches!(n.appearance.fill_paint(), Paint::Gradient(_)) {
                boxes.push(n.geometric_bounds().unwrap().width());
            }
        });
        assert_eq!(boxes, vec![100.0; 3], "three squares, each with its own gradient");
    }
}
