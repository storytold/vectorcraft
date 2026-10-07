//! Layers panel, artboards.

use serde_json::{Value, json};
use vectorcraft_doc::{Artboard, LayerColor, NodeId, NodeKind};
use vectorcraft_geom::Rect;

use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.new", "New Layer…", ["Window", "Layers"], Some("Cmd+L"), "{name?} → {id} (above the current layer)", has_doc, new_layer),
        cmd!("layer.newSublayer", "New Sublayer…", ["Window", "Layers"], None, "{parent?: id, name?} → {id}", has_doc, new_sublayer),
        cmd!("layer.delete", "Delete Layer", ["Window", "Layers"], None, "{id?} (default: current layer)", has_doc, delete_layer),
        cmd!("layer.duplicate", "Duplicate Layer", ["Window", "Layers"], None, "{id?}", has_doc, duplicate_layer),
        cmd!("layer.setCurrent", "Set Current Layer", [], None, "{id}", has_doc, set_current),
        cmd!(
            "layer.setProps",
            "Layer Options…",
            ["Window", "Layers"],
            None,
            "{id, name?, visible?, locked?, template?, printable?, color?: index 0..26}",
            has_doc,
            set_props
        ),
        cmd!("layer.collectInNew", "Collect in New Layer", ["Window", "Layers"], None, "{}", has_selection, collect),
        cmd!("layer.selectAll", "Select All Art on Layer", [], None, "{id}", has_doc, select_all_on),
        cmd!("node.move", "Reorder", [], None, "{id, parent?: id (null = top level), index} drag-reorder in the Layers panel", has_doc, move_node),
        cmd!(
            "artboard.new",
            "New Artboard",
            ["Window", "Artboards"],
            None,
            "{x?, y?, width?, height?, name?} (default: right of the last)",
            has_doc,
            artboard_new
        ),
        cmd!("artboard.delete", "Delete Artboard", ["Window", "Artboards"], None, "{index}", has_doc, artboard_delete),
        cmd!(
            "artboard.setProps",
            "Artboard Options…",
            ["Window", "Artboards"],
            None,
            "{index, name?, x?, y?, width?, height?, background?: colour|null (the artboard's own background, painted behind its art on screen and in export; null: none)} (a locked artboard keeps its place and size)",
            has_doc,
            artboard_set
        ),
        cmd!(
            query "artboard.setActive",
            "Make Artboard Active",
            [],
            None,
            "{index?} make the artboard active (darker border; artboard commands without an index act on it); no index → {index} of the active one",
            has_doc,
            artboard_set_active
        ),
        cmd!(
            "artboard.lock",
            "Lock Artboard",
            ["Window", "Artboards"],
            None,
            "{index?: artboard (default: the active one), locked?: bool (default: toggle)} lock the artboard (the Artboard tool can't move or resize it) and every unlocked object lying on it; unlocking unlocks just the objects locking locked. One undo step → {index, locked, objects: [ids]}",
            has_doc,
            artboard_lock
        ),
        cmd!("artboard.fitToArt", "Fit to Artwork Bounds", ["Object", "Artboards"], None, "{index?}", has_doc, artboard_fit_art),
        cmd!("artboard.fitToSelection", "Fit to Selected Art", ["Object", "Artboards"], None, "{index?}", has_selection, artboard_fit_sel),
        cmd!(
            "layer.clippingMask.toggle",
            "Make/Release Clipping Mask",
            ["Window", "Layers"],
            None,
            "{id?: layer or group (default: the one selected group, else the current layer)} make: its top object (a path, compound path or text, which loses its paint) clips the rest and moves to the bottom (new art added on top is clipped); release: it stops clipping, the clipping path stays unpainted → {clip}",
            has_doc,
            clip_toggle
        ),
        cmd!(
            "layer.target",
            "Target",
            [],
            None,
            "{id} target a layer, group or object as clicking its target circle in the Layers panel does: a layer gets its visible, unlocked art selected and becomes the current layer, and appearance.*, effect.*, transparency.* and the opacity-mask commands without `ids` then act on the layer itself; anything else is selected. Any other selection change ends the targeting (`document.inspect` → target) → {id, selected: [..]}",
            has_doc,
            target
        ),
        cmd!(
            "layer.pasteRemembersLayers",
            "Paste Remembers Layers",
            ["Window", "Layers"],
            None,
            "{on?} (default: toggle) the document option: on, the Paste commands put objects back into the layers they were copied from (by name; a missing one is made on top), off into the current layer; one undo step when it changes (document.inspect → pasteRemembersLayers) → {on}",
            has_doc,
            paste_remembers_layers
        ),
    ]
}

/// Layers panel → Paste Remembers Layers.
fn paste_remembers_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let cur = s.doc()?.doc.paste_remembers_layers;
    let on = bool_or(p, "on", !cur);
    if on != cur {
        s.edit("Paste Remembers Layers", |d, _| {
            d.paste_remembers_layers = on;
            Ok(())
        })?;
    }
    Ok(json!({ "on": on }))
}

/// The Layers panel's target circle.
fn target(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.target";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let st = s.doc()?;
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    if !st.doc.is_editable(id) {
        return Err(bad(C, "the object is hidden or locked"));
    }
    let layer = n.is_layer();
    s.select(|d, sel| sel.set_target(d, id))?;
    let st = s.doc_mut()?;
    if layer {
        st.active_layer = Some(id);
    }
    Ok(json!({ "id": id.0, "selected": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The Layers panel's clipping mask button: the top object of a layer (or group) clips the rest,
/// or the clipping mask is released.
fn clip_toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let group = match st.selection.objects.as_slice() {
        [id] if matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Group { .. })) => Some(*id),
        _ => None,
    };
    let id = id_param(p, "id").or(group).or(st.current_layer()).ok_or_else(|| bad("layer.clippingMask.toggle", "no layer"))?;
    let clip = s.edit("Make/Release Clipping Mask", |d, _| {
        let n = d.node(id).ok_or(EngineError::NoNode(id))?;
        if !matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. }) {
            return Err(bad("layer.clippingMask.toggle", "not a layer or group"));
        }
        if n.clips() {
            super::object::release_clip(d, id);
            return Ok(false);
        }
        let top =
            n.children().and_then(|c| c.last()).map(|c| c.id).ok_or_else(|| EngineError::Other("the layer has no objects to clip by".into()))?;
        super::object::make_clipping_path(d, top)?;
        d.move_node(top, Some(id), 0)?;
        if let Some(n) = d.node_mut(id) {
            n.set_clips(true);
        }
        Ok(true)
    })?;
    Ok(json!({ "clip": clip }))
}

fn new_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").map(str::to_string);
    let st = s.doc()?;
    let cur = st.active_layer.and_then(|l| st.doc.position(l)).filter(|(par, _, _)| par.is_none()).map(|(_, i, _)| i);
    let id = s.edit("New Layer", |d, _| {
        let id = d.add_layer(name.as_deref());
        if let Some(i) = cur {
            d.move_node(id, None, i + 1)?;
        }
        Ok(id)
    })?;
    s.doc_mut()?.active_layer = Some(id);
    Ok(json!({ "id": id.0 }))
}

fn new_sublayer(s: &mut Session, p: &Value) -> Result<Value> {
    let parent = id_param(p, "parent").or(s.doc()?.current_layer()).ok_or_else(|| bad("layer.newSublayer", "no parent layer"))?;
    let name = str_param(p, "name").map(str::to_string);
    let id = s.edit("New Sublayer", |d, _| {
        let n = d.node(parent).and_then(|n| n.children()).map(|c| c.iter().filter(|c| c.is_layer()).count()).unwrap_or(0);
        let id = d.alloc_id();
        let color = LayerColor::Preset(((d.layers.len() + n + 1) % 27) as u8);
        let nm = name.unwrap_or_else(|| format!("Layer {}", d.node_count()));
        d.insert(Some(parent), usize::MAX, vectorcraft_doc::Node::layer(id, &nm, color))?;
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn delete_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").or(s.doc()?.current_layer()).ok_or_else(|| bad("layer.delete", "no layer"))?;
    let st = s.doc()?;
    if st.doc.layers.len() == 1 && st.doc.layers[0].id == id {
        return Err(EngineError::Other("a document needs at least one layer".into()));
    }
    s.edit("Delete Layer", |d, sel| {
        d.remove(id)?;
        sel.prune(d);
        Ok(())
    })?;
    let st = s.doc_mut()?;
    if st.active_layer == Some(id) || st.active_layer.is_some_and(|l| st.doc.node(l).is_none()) {
        st.active_layer = st.doc.default_layer();
    }
    ok()
}

fn duplicate_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").or(s.doc()?.current_layer()).ok_or_else(|| bad("layer.duplicate", "no layer"))?;
    let nid = s.edit("Duplicate Layer", |d, _| {
        let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        let n = d.node(id).cloned().ok_or(EngineError::NoNode(id))?;
        let mut c = d.reid(&n);
        c.name = Some(format!("{} copy", n.display_name()));
        Ok(d.insert(par, idx + 1, c)?)
    })?;
    Ok(json!({ "id": nid.0 }))
}

fn set_current(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("layer.setCurrent", "missing id"))?;
    let st = s.doc_mut()?;
    if !st.doc.node(id).is_some_and(|n| n.is_layer()) {
        return Err(bad("layer.setCurrent", "not a layer"));
    }
    st.active_layer = Some(id);
    st.revision += 1;
    ok()
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("layer.setProps", "missing id"))?;
    s.edit("Layer Options", |d, sel| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        if let Some(v) = str_param(p, "name") {
            n.name = Some(v.to_string());
        }
        if let Some(v) = p.get("visible").and_then(Value::as_bool) {
            n.visible = v;
        }
        if let Some(v) = p.get("locked").and_then(Value::as_bool) {
            n.locked = v;
        }
        if let NodeKind::Layer { template, printable, color, .. } = &mut n.kind {
            if let Some(v) = p.get("template").and_then(Value::as_bool) {
                *template = v;
            }
            if let Some(v) = p.get("printable").and_then(Value::as_bool) {
                *printable = v;
            }
            if let Some(v) = p.get("color").and_then(Value::as_u64) {
                *color = LayerColor::Preset((v % 27) as u8);
            }
        }
        // Hidden or locked content can't stay selected.
        let hidden: Vec<NodeId> = sel.objects.iter().copied().filter(|o| !d.is_editable(*o)).collect();
        for h in hidden {
            sel.remove(h);
        }
        Ok(())
    })?;
    ok()
}

fn collect(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = super::edit::selected_roots(s)?;
    let lid = s.edit("Collect in New Layer", |d, _| {
        let lid = d.add_layer(None);
        for id in &ids {
            d.move_node(*id, Some(lid), usize::MAX)?;
        }
        Ok(lid)
    })?;
    Ok(json!({ "id": lid.0 }))
}

fn select_all_on(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("layer.selectAll", "missing id"))?;
    let ids: Vec<NodeId> = s
        .doc()?
        .doc
        .node(id)
        .and_then(|n| n.children())
        .map(|c| c.iter().filter(|n| n.visible && !n.locked).map(|n| n.id).collect())
        .unwrap_or_default();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    ok()
}

fn move_node(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("node.move", "missing id"))?;
    let parent = id_param(p, "parent");
    let index = p.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
    if parent.is_none() && !s.doc()?.doc.node(id).is_some_and(|n| n.is_layer()) {
        return Err(bad("node.move", "only layers can be at the top level"));
    }
    s.edit("Reorder", |d, _| Ok(d.move_node(id, parent, index)?))?;
    ok()
}

fn rect_from(p: &Value, default: Rect) -> Rect {
    let x = f64_or(p, "x", default.x0);
    let y = f64_or(p, "y", default.y0);
    let w = f64_or(p, "width", default.width()).max(1.0);
    let h = f64_or(p, "height", default.height()).max(1.0);
    Rect::new(x, y, x + w, y + h)
}

fn artboard_new(s: &mut Session, p: &Value) -> Result<Value> {
    let d0 = &s.doc()?.doc;
    let last = d0.artboards.last().map(|a| a.rect).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
    let def = Rect::new(last.x1 + 20.0, last.y0, last.x1 + 20.0 + last.width(), last.y1);
    let r = rect_from(p, def);
    let idx = s.edit("New Artboard", |d, _| {
        let id = d.next_artboard_id();
        let name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| format!("Artboard {}", d.artboards.len() + 1));
        d.artboards.push(Artboard { id, name, rect: r, show_center_mark: false, show_cross_hairs: false, ..Default::default() });
        Ok(d.artboards.len() - 1)
    })?;
    Ok(json!({ "index": idx }))
}

fn artboard_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("artboard.delete", "missing index"))? as usize;
    s.edit("Delete Artboard", |d, _| {
        if d.artboards.len() <= 1 {
            return Err(EngineError::Other("a document needs at least one artboard".into()));
        }
        if i >= d.artboards.len() {
            return Err(EngineError::Other("no such artboard".into()));
        }
        d.artboards.remove(i);
        Ok(())
    })?;
    ok()
}

fn artboard_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.setProps";
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let background = match p.get("background") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(Some(super::color_value(v).ok_or_else(|| bad(C, format!("background must be a colour or null, not {v}")))?)),
    };
    let geometry = ["x", "y", "width", "height"].iter().any(|k| p.get(*k).is_some());
    s.edit("Artboard Options", |d, _| {
        let a = d.artboards.get_mut(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        if geometry {
            if a.locked {
                return Err(EngineError::Other(format!("artboard “{}” is locked", a.name)));
            }
            a.rect = rect_from(p, a.rect);
        }
        if let Some(n) = str_param(p, "name") {
            a.name = n.to_string();
        }
        if let Some(b) = background {
            a.background = b;
        }
        Ok(())
    })?;
    ok()
}

/// Top-level objects (children of layers, through sublayers) lying entirely on `rect`, that
/// `keep` accepts.
pub(crate) fn art_on(doc: &vectorcraft_doc::Document, rect: Rect, keep: &dyn Fn(&vectorcraft_doc::Node) -> bool) -> Vec<NodeId> {
    fn collect(n: &vectorcraft_doc::Node, rect: Rect, keep: &dyn Fn(&vectorcraft_doc::Node) -> bool, out: &mut Vec<NodeId>) {
        for c in n.children().into_iter().flatten() {
            if c.is_layer() {
                collect(c, rect, keep, out);
            } else if keep(c)
                && let Some(b) = c.geometric_bounds()
                && rect.contains(vectorcraft_geom::Point::new(b.x0, b.y0))
                && rect.contains(vectorcraft_geom::Point::new(b.x1, b.y1))
            {
                out.push(c.id);
            }
        }
    }
    let mut out = vec![];
    for l in &doc.layers {
        collect(l, rect, keep, &mut out);
    }
    out
}

fn artboard_set_active(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.setActive";
    let st = s.doc_mut()?;
    if let Some(v) = p.get("index") {
        let i = v.as_u64().ok_or_else(|| bad(C, "index must be an artboard number from 0"))? as usize;
        if i >= st.doc.artboards.len() {
            return Err(bad(C, format!("no artboard {i}")));
        }
        if st.active_artboard != i {
            st.active_artboard = i;
            st.revision += 1;
        }
    }
    let n = st.doc.artboards.len();
    Ok(json!({ "index": st.active_artboard.min(n.saturating_sub(1)) }))
}

fn artboard_lock(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.lock";
    let st = s.doc()?;
    let i = match p.get("index") {
        Some(v) => v.as_u64().ok_or_else(|| bad(C, "index must be an artboard number from 0"))? as usize,
        None => st.active_artboard.min(st.doc.artboards.len().saturating_sub(1)),
    };
    let ab = st.doc.artboards.get(i).ok_or_else(|| bad(C, format!("no artboard {i}")))?;
    let locked = p.get("locked").and_then(Value::as_bool).unwrap_or(!ab.locked);
    let objects = if locked { art_on(&st.doc, ab.rect, &|n| !n.locked) } else { ab.locked_art.clone() };
    let label = if locked { "Lock Artboard" } else { "Unlock Artboard" };
    s.edit(label, |d, sel| {
        for id in &objects {
            if let Some(n) = d.node_mut(*id) {
                n.locked = locked;
            }
            if locked {
                sel.remove(*id);
            }
        }
        let a = d.artboards.get_mut(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        a.locked = locked;
        a.locked_art = if locked { objects.clone() } else { vec![] };
        Ok(())
    })?;
    Ok(json!({ "index": i, "locked": locked, "objects": objects.iter().map(|o| o.0).collect::<Vec<_>>() }))
}

fn artboard_fit_art(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let b = s.doc()?.doc.art_bounds().ok_or_else(|| EngineError::Other("no artwork".into()))?;
    s.execute("artboard.setProps", &json!({"index": i, "x": b.x0, "y": b.y0, "width": b.width(), "height": b.height()}))
}

fn artboard_fit_sel(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let st = s.doc()?;
    let b = st.doc.bounds_of(&st.selection.objects, true).ok_or_else(|| EngineError::Other("no selection bounds".into()))?;
    s.execute("artboard.setProps", &json!({"index": i, "x": b.x0, "y": b.y0, "width": b.width(), "height": b.height()}))
}
