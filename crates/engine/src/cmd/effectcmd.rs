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
            "{effect: id (see effect.list, e.g. \"stylize.dropShadow\", \"distort.roughen\", \"warp.arc\", or \"plugin.<id>\" for an installed effect plug-in), params?: {…} (missing keys take the dialog defaults), item?: appearance item index|null (apply to that fill/stroke only; omitted: the Appearance panel's active item, else the whole object), ids?: [..] (layers too), target?: \"object\"|\"contents\" (contents: the objects inside groups and layers)} append a live effect to each selected object's appearance (a group's or layer's apply to its members as one piece: one combined shadow; a Pathfinder effect, which combines a group's contents, groups several loose selected objects first, in the same undo step) → {ids, index, item, grouped?}",
            has_doc,
            apply
        ),
        cmd!(
            query "effect.list",
            "Effects",
            [],
            None,
            "{} → {catalog: [{id, label, menu, params, defaults, raster, lengths: {always, absolute (while relative is false)} (distance params Scale Strokes & Effects scales)}] (installed effect plug-ins last: id plugin.<plug-in id>, menu Effect › Plug-ins), applied: [{id, effects, items: [{index, kind: fill|stroke, effects}]}], activeItem} for the selection",
            always,
            list
        ),
        cmd!(
            "effect.remove",
            "Remove Effect",
            [],
            None,
            "{index: int (position in the effect list), item?: appearance item index|null (that fill/stroke's effects; omitted: the active item, else the object's), ids?: [..]} → {ids}",
            has_doc,
            remove
        ),
        cmd!(
            "effect.setParams",
            "Effect Options",
            [],
            None,
            "{index: int, params?: {…} (merged into the current parameters), visible?: bool, item?: appearance item index|null (as effect.remove), ids?: [..]} → {ids}",
            has_doc,
            set_params
        ),
        cmd!(
            "effect.expandAppearance",
            "Expand Appearance",
            ["Object"],
            None,
            "{ids?: [..], target?} turn each object's appearance into objects: every fill and stroke becomes an object of its own (strokes outlined, brushed strokes their brush art) grouped in paint order under the object's id and transparency, geometry effects are baked, raster effects become an embedded image (shadows and outer glows below the art; blur, feather and inner glow replace it), type with effects or fills of its own is outlined; Crop Marks become a group of the object and its marks; a group's or layer's own fills and strokes become objects among its members, which are expanded too. Hidden fills, strokes and effects are dropped. Enabled when a selected or targeted object's appearance isn't basic (`ids` then pick which objects) → {ids}",
            can_expand,
            expand_appearance
        ),
        cmd!(
            "effect.duplicate",
            "Duplicate Effect",
            [],
            None,
            "{index: int, item?: appearance item index|null (as effect.remove), ids?: [..]} insert a copy of the effect right after it → {ids}",
            has_doc,
            duplicate
        ),
        cmd!(
            "effect.move",
            "Move Effect",
            [],
            None,
            "{from: int (position in the source list), to: int (its position in the destination list afterwards), fromItem?: appearance item index|null (the source list: that fill/stroke's effects, null the object's; omitted: the active item, else the object's), toItem?: item index|null (the destination list; default: the source list), copy?: bool (copy instead of move, as Alt-dragging the row), ids?: [..]} reorder an effect or move it between the object and its fills/strokes, as one undo step → {ids, index, item}",
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
    let count = n.appearance.effects.iter().chain(n.appearance.items.iter().flat_map(|i| i.effects())).filter(|e| e.id == effects::REVOLVE).count();
    if count > 1 {
        return Err(bad(cmd, "A profile supports one Revolve effect. Edit the existing effect's options instead."));
    }
    Ok(())
}

pub(crate) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.apply";
    let id = str_param(p, "effect").or_else(|| str_param(p, "id")).ok_or_else(|| bad(C, "missing `effect` id"))?;
    let mut params = p.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err(bad(C, "`params` must be an object"));
    }
    // Crop Marks take their style from the preferences when applied.
    if id == effects::CROP_MARKS && params.get("style").is_none() {
        params["style"] = json!(s.crop_mark_style().id());
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
    if id == effects::REVOLVE {
        for id in appearance_targets(s, p)? {
            let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
            effects::validate_revolve(n, &effect.params).map_err(|e| bad(C, e))?;
            if effects::has_revolve(n) {
                return Err(bad(C, "A profile supports one Revolve effect. Edit the existing effect's options instead."));
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

fn set_params(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.setParams";
    let index = index_param(p, "index", C)?;
    let params = p.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err(bad(C, "`params` must be an object"));
    }
    let visible = p.get("visible").and_then(Value::as_bool);
    let item = item_target(s, p, C)?;
    for id in appearance_targets(s, p)? {
        let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
        let target = item.effects_item(&n.appearance, C)?;
        if let Some(e) = n.appearance.effects_at(target).and_then(|fx| fx.get(index))
            && e.id == effects::REVOLVE
            && visible != Some(false)
        {
            let mut prospective = effects::merged_params(&e.id, &e.params);
            if let (Some(cur), Some(new)) = (prospective.as_object_mut(), params.as_object()) {
                cur.extend(new.clone());
            }
            effects::validate_revolve(n, &prospective).map_err(|e| bad(C, e))?;
        }
    }
    let ids = edit_effects(s, p, item, C, "Effect Options", &format!("{C}: no effect at index {index}"), |fx| {
        let Some(e) = fx.get_mut(index) else { return false };
        if let (Value::Object(new), cur) = (&params, &mut e.params) {
            if !cur.is_object() {
                *cur = effects::merged_params(&e.id, &Value::Null);
            }
            if let Value::Object(m) = cur {
                for (k, v) in new {
                    m.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(v) = visible {
            e.visible = v;
        }
        true
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
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
/// the document's raster effects resolution, with whether they all paint below it (shadows and
/// outer glows: the image then holds just them). A group's or layer's image leaves out its
/// members' raster effects, which are expanded with them. `None` when it has none.
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
fn expand_node(d: &mut Document, id: NodeId, brushes: &[vectorcraft_brush::Brush], out: &mut Vec<NodeId>) {
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
            } else if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
                effects::expand_leaf(d, &v, &mut stroke_art)
            } else {
                // Type, images, symbol instances, live objects: through their outlines.
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
    Ok(json!({ "ids": ids_json(&ids) }))
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

    fn effects_for_test(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
        effects::revolve_art(&node(s, id)).unwrap()
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
}
