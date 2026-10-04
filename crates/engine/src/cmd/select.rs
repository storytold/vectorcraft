//! Select menu.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("select.all", "All", ["Select"], Some("Cmd+A"), "{}", has_doc, all),
        cmd!("select.allOnArtboard", "All on Active Artboard", ["Select"], Some("Cmd+Alt+A"), "{artboard?: index}", has_doc, all_on_artboard),
        cmd!("select.none", "Deselect", ["Select"], Some("Cmd+Shift+A"), "{}", has_doc, none),
        cmd!("select.reselect", "Reselect", ["Select"], Some("Cmd+6"), "{}", has_doc, reselect),
        cmd!("select.inverse", "Inverse", ["Select"], None, "{}", has_doc, inverse),
        cmd!("select.nextAbove", "Next Object Above", ["Select"], Some("Cmd+Alt+]"), "{}", has_selection, |s, _| step(s, 1)),
        cmd!("select.nextBelow", "Next Object Below", ["Select"], Some("Cmd+Alt+["), "{}", has_selection, |s, _| step(s, -1)),
        cmd!("select.set", "Select Objects", [], None, "{ids: [id…]}", has_doc, set),
        cmd!("select.add", "Add to Selection", [], None, "{ids: [id…]}", has_doc, add),
        cmd!("select.toggle", "Toggle Selection", [], None, "{id}", has_doc, toggle),
        cmd!("select.key", "Set Key Object", [], None, "{id?} (none clears)", has_doc, key),
        cmd!("select.anchors", "Select Anchors", [], None, "{id, anchors: [[subpath, anchor]…], mode: \"set\"|\"add\"|\"toggle\"}", has_doc, anchors),
        cmd!("select.anchorsMany", "Select Anchors", [], None, "{items: [{id, anchors}], add?: bool}", has_doc, anchors_many),
        cmd!("select.same.fillColor", "Fill Color", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.fillColor",
            |a, b| a.appearance.fill_paint() == b.appearance.fill_paint()
        )),
        cmd!("select.same.strokeColor", "Stroke Color", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.strokeColor",
            |a, b| a.appearance.stroke_paint() == b.appearance.stroke_paint()
        )),
        cmd!("select.same.strokeWeight", "Stroke Weight", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.strokeWeight",
            |a, b| (a.appearance.stroke_width() - b.appearance.stroke_width()).abs() < 1e-9
        )),
        cmd!("select.same.fillAndStroke", "Fill & Stroke", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.fillAndStroke",
            |a, b| a.appearance.fill_paint() == b.appearance.fill_paint() && a.appearance.stroke_paint() == b.appearance.stroke_paint()
        )),
        cmd!("select.same.opacity", "Opacity", ["Select", "Same"], None, "{}", has_selection, |s, _| same(s, "select.same.opacity", |a, b| (a
            .opacity
            - b.opacity)
            .abs()
            < 1e-6)),
        cmd!("select.same.blendingMode", "Blending Mode", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.blendingMode",
            |a, b| a.blend == b.blend
        )),
        cmd!("select.same.appearance", "Appearance", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.appearance",
            |a, b| a.appearance == b.appearance && a.opacity == b.opacity && a.blend == b.blend
        )),
        cmd!("select.same.shapeType", "Shape", ["Select", "Same"], None, "{}", has_selection, |s, _| same(s, "select.same.shapeType", |a, b| a
            .kind_label()
            == b.kind_label())),
        cmd!("select.object.allOnSameLayers", "All on Same Layers", ["Select", "Object"], None, "{}", has_selection, same_layers),
        cmd!("select.object.clippingMasks", "Clipping Masks", ["Select", "Object"], None, "{}", has_doc, |s, _| by_kind(s, |n| matches!(
            n.kind,
            NodeKind::Path { clipping: true, .. }
        ))),
        cmd!("select.object.textObjects", "All Text Objects", ["Select", "Object"], None, "{}", has_doc, |s, _| by_kind(s, |n| matches!(
            n.kind,
            NodeKind::Text(_)
        ))),
        cmd!("select.object.strayPoints", "Stray Points", ["Select", "Object"], None, "{}", has_doc, |s, _| by_kind(s, |n| n
            .path_data()
            .is_some_and(|p| p.anchor_count() == 1))),
        cmd!("select.object.openPaths", "Open Paths", ["Select", "Object"], None, "{}", has_doc, |s, _| by_kind(s, |n| n
            .path_data()
            .is_some_and(|p| !p.is_closed()))),
        cmd!(
            "select.same.graphicStyle",
            "Graphic Style",
            ["Select", "Same"],
            None,
            "{} the objects linked to the graphic style the first selected object is linked to",
            has_selection,
            same_graphic_style
        ),
        cmd!(
            "select.same.appearanceAttribute",
            "Appearance Attribute",
            ["Select", "Same"],
            None,
            "{item?: appearance item index} the objects sharing an appearance attribute of the first selected object: its fill or stroke `item` (default: the Appearance panel's active item), else its first effect, else its topmost fill",
            has_selection,
            same_attribute
        ),
    ]
}

/// Selectable objects: children of visible, unlocked layers (or of the isolation container).
fn selectable(d: &Document, iso: Option<NodeId>) -> Vec<NodeId> {
    match iso.and_then(|i| d.node(i)) {
        Some(c) => c.children().map(|v| v.iter().filter(|n| n.visible && !n.locked).map(|n| n.id).collect()).unwrap_or_default(),
        None => d
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.children().into_iter().flatten())
            .filter(|n| n.visible && !n.locked)
            .map(|n| n.id)
            .collect(),
    }
}

fn all(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = selectable(&st.doc, st.isolation);
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "count": s.doc()?.selection.len() }))
}

fn all_on_artboard(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let i = p.get("artboard").and_then(Value::as_u64).unwrap_or(0) as usize;
    let r = st.doc.artboards.get(i).map(|a| a.rect).ok_or_else(|| bad("select.allOnArtboard", "no such artboard"))?;
    let ids: Vec<NodeId> = selectable(&st.doc, st.isolation)
        .into_iter()
        .filter(|id| st.doc.node(*id).and_then(|n| n.geometric_bounds()).is_some_and(|b| b.intersect(r).area() > 0.0 || r.contains(b.center())))
        .collect();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    ok()
}

fn none(s: &mut Session, _: &Value) -> Result<Value> {
    s.select(|_, sel| sel.clear())?;
    ok()
}

fn reselect(s: &mut Session, _: &Value) -> Result<Value> {
    let Some((c, p)) = s.doc()?.last_selection_cmd.clone() else { return ok() };
    s.execute(&c, &p)
}

fn inverse(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids: Vec<NodeId> = selectable(&st.doc, st.isolation).into_iter().filter(|id| !st.selection.contains(*id)).collect();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    ok()
}

fn step(s: &mut Session, dir: i64) -> Result<Value> {
    let st = s.doc()?;
    let Some(cur) = st.selection.objects.first().copied() else { return ok() };
    let Some((par, idx, n)) = st.doc.position(cur) else { return ok() };
    let j = idx as i64 + dir;
    if j < 0 || j >= n as i64 {
        return ok();
    }
    let Some(id) = st.doc.children(par).and_then(|c| c.get(j as usize)).map(|n| n.id) else { return ok() };
    s.select(|_, sel| sel.set([id]))?;
    ok()
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = ids_param(p, "ids").ok_or_else(|| bad("select.set", "missing ids"))?;
    s.select(|d, sel| sel.set(ids.iter().copied().filter(|i| d.node(*i).is_some())))?;
    ok()
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = ids_param(p, "ids").ok_or_else(|| bad("select.add", "missing ids"))?;
    s.select(|_, sel| {
        for i in ids {
            sel.add(i);
        }
    })?;
    ok()
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("select.toggle", "missing id"))?;
    s.select(|_, sel| sel.toggle(id))?;
    ok()
}

fn key(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id");
    s.select(|_, sel| sel.key = id.filter(|i| sel.contains(*i)))?;
    ok()
}

fn parse_refs(v: Option<&Value>) -> Vec<(usize, usize)> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| Some((x.get(0)?.as_u64()? as usize, x.get(1)?.as_u64()? as usize))).collect())
        .unwrap_or_default()
}

fn anchors(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("select.anchors", "missing id"))?;
    let refs = parse_refs(p.get("anchors"));
    let mode = str_param(p, "mode").unwrap_or("set").to_string();
    s.select(|_, sel| {
        if mode == "set" {
            sel.clear();
        }
        sel.add(id);
        let e = sel.anchors.entry(id).or_default();
        for r in refs {
            if mode == "toggle" && e.contains(&r) {
                e.remove(&r);
            } else {
                e.insert(r);
            }
        }
    })?;
    ok()
}

fn anchors_many(s: &mut Session, p: &Value) -> Result<Value> {
    let items: Vec<(NodeId, BTreeSet<(usize, usize)>)> = p
        .get("items")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|it| Some((NodeId(it.get("id")?.as_u64()?), parse_refs(it.get("anchors")).into_iter().collect()))).collect())
        .unwrap_or_default();
    let add = bool_or(p, "add", false);
    s.select(|d, sel| {
        if !add {
            sel.clear();
        }
        for (id, refs) in items {
            let total = d.node(id).and_then(|n| n.path_data()).map(|p| p.anchor_count()).unwrap_or(0);
            sel.add(id);
            if refs.len() < total {
                sel.anchors.entry(id).or_default().extend(refs);
            } else {
                sel.anchors.remove(&id);
            }
        }
    })?;
    ok()
}

fn same(s: &mut Session, cmd: &str, eq: fn(&Node, &Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let refn = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().ok_or_else(|| bad(cmd, "nothing selected"))?;
    select_where(s, cmd, &json!({}), |_, n| !n.is_container() && eq(n, &refn))
}

/// Select the visible, unlocked objects in visible, unlocked layers that `f` accepts (not looking
/// inside accepted ones); Select > Reselect repeats `cmd` with `p`.
fn select_where(s: &mut Session, cmd: &str, p: &Value, f: impl Fn(&Document, &Node) -> bool) -> Result<Value> {
    fn visit(d: &Document, n: &Node, f: &impl Fn(&Document, &Node) -> bool, ids: &mut Vec<NodeId>) {
        if n.visible && !n.locked && !n.is_layer() && f(d, n) {
            return ids.push(n.id);
        }
        for c in n.children().into_iter().flatten() {
            visit(d, c, f, ids);
        }
    }
    let st = s.doc()?;
    let mut ids = vec![];
    for l in st.doc.layers.iter().filter(|l| l.visible && !l.locked) {
        visit(&st.doc, l, &f, &mut ids);
    }
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    s.doc_mut()?.last_selection_cmd = Some((cmd.to_string(), p.clone()));
    Ok(json!({ "count": ids.len() }))
}

fn same_graphic_style(s: &mut Session, _: &Value) -> Result<Value> {
    let cmd = "select.same.graphicStyle";
    let st = s.doc()?;
    let id = st
        .selection
        .objects
        .first()
        .and_then(|id| super::style::linked_style(&st.doc, st.doc.node(*id)?))
        .map(|g| g.id)
        .ok_or_else(|| bad(cmd, "the selection has no graphic style"))?;
    select_where(s, cmd, &json!({}), |d, n| n.graphic_style == Some(id) && super::style::linked_style(d, n).is_some())
}

/// An appearance attribute Select > Same > Appearance Attribute matches.
enum Attribute {
    Item(vectorcraft_doc::AppearanceItem),
    Effect(vectorcraft_doc::Effect),
}

fn same_attribute(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "select.same.appearanceAttribute";
    let st = s.doc()?;
    let refn = st.selection.objects.first().and_then(|id| st.doc.node(*id)).ok_or_else(|| bad(cmd, "nothing selected"))?;
    let ap = &refn.appearance;
    let item = p.get("item").and_then(Value::as_u64).map(|i| i as usize).or(s.appearance_item());
    let attr = match (item.and_then(|i| ap.items.get(i)), ap.effects.first()) {
        (Some(it), _) => Attribute::Item(it.clone()),
        (None, Some(e)) => Attribute::Effect(e.clone()),
        (None, None) => {
            Attribute::Item(ap.items.iter().rev().find(|i| i.is_fill()).cloned().ok_or_else(|| bad(cmd, "the object has no fill, stroke or effect"))?)
        }
    };
    select_where(s, cmd, p, |_, n| match &attr {
        Attribute::Item(it) => n.appearance.items.contains(it),
        Attribute::Effect(e) => n.appearance.effects.iter().any(|x| x.id == e.id && x.params == e.params),
    })
}

fn same_layers(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let layers: BTreeSet<NodeId> = st.selection.objects.iter().filter_map(|id| st.doc.layer_of(*id)).collect();
    let ids: Vec<NodeId> = layers
        .iter()
        .filter_map(|l| st.doc.node(*l))
        .flat_map(|l| l.children().into_iter().flatten())
        .filter(|n| n.visible && !n.locked)
        .map(|n| n.id)
        .collect();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    ok()
}

fn by_kind(s: &mut Session, f: fn(&Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let mut ids = vec![];
    for l in st.doc.layers.iter().filter(|l| l.visible && !l.locked) {
        l.walk(&mut |n| {
            if f(n) {
                ids.push(n.id)
            }
        });
    }
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "count": ids.len() }))
}
