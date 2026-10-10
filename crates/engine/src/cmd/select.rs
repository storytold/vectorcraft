//! Select menu.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use crate::LastSelection;

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "select.all",
            "All",
            ["Select"],
            Some("Cmd+A"),
            "{} → {count}; while the Type tool edits text, all of that text instead → {editing, start, end} (byte offsets)",
            has_doc,
            all
        ),
        cmd!("select.allOnArtboard", "All on Active Artboard", ["Select"], Some("Cmd+Alt+A"), "{artboard?: index}", has_doc, all_on_artboard),
        cmd!("select.none", "Deselect", ["Select"], Some("Cmd+Shift+A"), "{}", has_doc, none),
        cmd!(
            "select.reselect",
            "Reselect",
            ["Select"],
            Some("Cmd+6"),
            "{} the objects the last Select → Same chose, skipping any since deleted, hidden, locked or taken off the layer that isolation mode left out → {count, ids}",
            has_doc,
            reselect
        ),
        cmd!("select.inverse", "Inverse", ["Select"], None, "{}", has_doc, inverse),
        cmd!("select.nextAbove", "Next Object Above", ["Select"], Some("Cmd+Alt+]"), "{}", has_selection, |s, _| step(s, 1)),
        cmd!("select.nextBelow", "Next Object Below", ["Select"], Some("Cmd+Alt+["), "{}", has_selection, |s, _| step(s, -1)),
        cmd!(
            "select.set",
            "Select Objects",
            [],
            None,
            "{ids: [id…]} all ids must be non-negative integers naming existing objects → {count, ids} (resulting selection)",
            has_doc,
            set
        ),
        cmd!(
            "select.add",
            "Add to Selection",
            [],
            None,
            "{ids: [id…]} all ids must be non-negative integers naming existing objects → {count, ids} (resulting selection)",
            has_doc,
            add
        ),
        cmd!(
            "select.toggle",
            "Toggle Selection",
            [],
            None,
            "{id} or {ids: [id…]}: each selected one leaves the selection, the others join it; all ids must be non-negative integers naming existing objects → {count, ids} (resulting selection)",
            has_doc,
            toggle
        ),
        cmd!(
            "select.key",
            "Set Key Object",
            [],
            None,
            "{id?: a selected object (the Selection tool: a click on one object of a selection of several; on the key again: none)} the key object Align aligns to and Distribute Spacing spaces from; none clears it",
            has_doc,
            key
        ),
        cmd!("select.anchors", "Select Anchors", [], None, "{id, anchors: [[subpath, anchor]…], mode: \"set\"|\"add\"|\"toggle\"}", has_doc, anchors),
        cmd!(
            "select.anchorsMany",
            "Select Anchors",
            [],
            None,
            "{items: [{id, anchors}], mode?: \"set\"|\"add\"|\"toggle\"|\"subtract\" (set by default; add: true is mode add)}: toggle selects the anchors not selected and deselects the others, subtract deselects them; a path left with none leaves the selection",
            has_doc,
            anchors_many
        ),
        cmd!(
            "select.same.fillColor",
            "Fill Color",
            ["Select", "Same"],
            None,
            "{} the objects filled with the first selected object's fill colour (any tint of its global or spot swatch, the same tint with prefs selectSameTintPercent)",
            has_selection,
            |s, _| {
                let tint = s.prefs.select_same_tint_percent;
                same(s, "select.same.fillColor", |a, b| same_paint(&a.appearance.fill_paint(), &b.appearance.fill_paint(), tint))
            }
        ),
        cmd!(
            "select.same.strokeColor",
            "Stroke Color",
            ["Select", "Same"],
            None,
            "{} the objects stroked with the first selected object's stroke colour (tints as select.same.fillColor)",
            has_selection,
            |s, _| {
                let tint = s.prefs.select_same_tint_percent;
                same(s, "select.same.strokeColor", |a, b| same_paint(&a.appearance.stroke_paint(), &b.appearance.stroke_paint(), tint))
            }
        ),
        cmd!("select.same.strokeWeight", "Stroke Weight", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.strokeWeight",
            |a, b| (a.appearance.stroke_width() - b.appearance.stroke_width()).abs() < 1e-9
        )),
        cmd!(
            "select.same.fillAndStroke",
            "Fill & Stroke",
            ["Select", "Same"],
            None,
            "{} the objects with the first selected object's fill and stroke colours (tints as select.same.fillColor)",
            has_selection,
            |s, _| {
                let tint = s.prefs.select_same_tint_percent;
                same(s, "select.same.fillAndStroke", |a, b| {
                    let (x, y) = (&a.appearance, &b.appearance);
                    same_paint(&x.fill_paint(), &y.fill_paint(), tint) && same_paint(&x.stroke_paint(), &y.stroke_paint(), tint)
                })
            }
        ),
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
/// The objects Select All takes: those of the isolated container, else of every layer (looking
/// through sublayers, which are not objects).
fn selectable(d: &Document, iso: Option<NodeId>) -> Vec<NodeId> {
    match iso.and_then(|i| d.node(i)) {
        Some(c) => c.layer_art(true),
        None => d.selectable_art(),
    }
}

fn all(s: &mut Session, _: &Value) -> Result<Value> {
    // While the Type tool edits text, Select All takes all of that text and leaves the art
    // selection as it is (Illustrator: text cursor in a text object).
    if s.tool_wants_text() {
        s.set_tool_option("selectAll", &json!(true));
        let editing = s.tool_options()["editing"].clone();
        let id = editing.as_u64().map(NodeId);
        let end = id.and_then(|id| match &s.doc().ok()?.doc.node(id)?.kind {
            NodeKind::Text(t) => Some(t.plain_text().len()),
            _ => None,
        });
        return Ok(json!({ "editing": editing, "start": 0, "end": end.unwrap_or(0) }));
    }
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

/// Reselect puts back the objects the last Select → Same chose. It works from the saved ids, not
/// by repeating the command: those commands read their reference off the selection, which a
/// Deselect has cleared (#903). Objects deleted, hidden, locked or out of the isolated group since
/// are skipped.
fn reselect(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(last) = s.doc()?.last_selection.clone() else { return ok() };
    let reach: BTreeSet<NodeId> = {
        let st = s.doc()?;
        selectable(&st.doc, st.isolation).into_iter().collect()
    };
    let ids: Vec<NodeId> = last.ids.into_iter().filter(|id| reach.contains(id)).collect();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    selection_result(s)
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
    let ids = checked_ids_param(s, p, "ids", "select.set")?;
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    selection_result(s)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = checked_ids_param(s, p, "ids", "select.add")?;
    s.select(|_, sel| {
        for i in ids {
            sel.add(i);
        }
    })?;
    selection_result(s)
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = if p.get("ids").is_some() {
        checked_ids_param(s, p, "ids", "select.toggle")?
    } else {
        let value = p.get("id").ok_or_else(|| bad("select.toggle", "missing id or ids"))?;
        vec![checked_id(s, value, "select.toggle")?]
    };
    s.select(|_, sel| {
        for id in ids {
            sel.toggle(id);
        }
    })?;
    selection_result(s)
}

fn selection_result(s: &Session) -> Result<Value> {
    let selection = &s.doc()?.selection;
    Ok(json!({ "count": selection.len(), "ids": selection.objects }))
}

fn key(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id");
    s.select(|_, sel| sel.key = id.filter(|i| sel.contains(*i)))?;
    ok()
}

/// `[[subpath, anchor]…]` anchor references (malformed entries are left out).
pub(crate) fn parse_refs(v: Option<&Value>) -> Vec<(usize, usize)> {
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
    // `add: true` is the older spelling of mode add.
    let mode = match str_param(p, "mode") {
        Some(m @ ("set" | "add" | "toggle" | "subtract")) => m,
        Some(m) => return Err(bad("select.anchorsMany", format!("unknown mode {m:?}"))),
        None if bool_or(p, "add", false) => "add",
        None => "set",
    };
    s.select(|d, sel| {
        if mode == "set" {
            sel.clear();
        }
        for (id, refs) in items {
            let all: BTreeSet<(usize, usize)> =
                d.node(id).and_then(|n| n.path_data()).map(|p| p.anchors().map(|(s, i, _)| (s, i)).collect()).unwrap_or_default();
            // Anything but a path is selected, toggled or deselected as a whole.
            if all.is_empty() {
                match mode {
                    "toggle" => sel.toggle(id),
                    "subtract" => sel.remove(id),
                    _ => sel.add(id),
                }
                continue;
            }
            // From the anchors selected now (all of them for a path selected as a whole).
            let now = match sel.partial(id) {
                Some(a) => a.clone(),
                None if sel.contains(id) => all.clone(),
                None => BTreeSet::new(),
            };
            let refs: BTreeSet<(usize, usize)> = refs.intersection(&all).copied().collect();
            let next: BTreeSet<(usize, usize)> = match mode {
                "toggle" => now.symmetric_difference(&refs).copied().collect(),
                "subtract" => now.difference(&refs).copied().collect(),
                _ => now.union(&refs).copied().collect(),
            };
            if next.is_empty() {
                sel.remove(id);
                continue;
            }
            sel.add(id);
            if next.len() < all.len() {
                sel.anchors.insert(id, next);
            } else {
                sel.anchors.remove(&id);
            }
        }
    })?;
    ok()
}

/// Select › Same › Fill/Stroke Color: is `a` the same colour as `b`? Tints of one global or spot
/// swatch are, unless `tint` (General › Select Same Tint %) asks for the same tint too.
fn same_paint(a: &Paint, b: &Paint, tint: bool) -> bool {
    match (a, b) {
        (Paint::Solid { swatch: Some(x), tint: ta, .. }, Paint::Solid { swatch: Some(y), tint: tb, .. }) if x == y => !tint || (ta - tb).abs() < 1e-4,
        _ => a == b,
    }
}

fn same(s: &mut Session, cmd: &str, eq: impl Fn(&Node, &Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let refn = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().ok_or_else(|| bad(cmd, "nothing selected"))?;
    select_where(s, cmd, |_, n| !n.is_container() && eq(n, &refn))
}

/// Select the visible, unlocked objects in visible, unlocked layers that `f` accepts (not looking
/// inside accepted ones); Select ▸ Reselect repeats the choice from the ids it lands here.
fn select_where(s: &mut Session, cmd: &str, f: impl Fn(&Document, &Node) -> bool) -> Result<Value> {
    fn visit(d: &Document, n: &Node, f: &impl Fn(&Document, &Node) -> bool, ids: &mut Vec<NodeId>) {
        // Hidden or locked objects and sublayers (and what they hold) are out of reach.
        if !n.visible || n.locked || n.is_template() {
            return;
        }
        if !n.is_layer() && f(d, n) {
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
    s.doc_mut()?.last_selection = Some(LastSelection { cmd: cmd.to_string(), ids: ids.to_vec() });
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
    select_where(s, cmd, |d, n| n.graphic_style == Some(id) && super::style::linked_style(d, n).is_some())
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
    select_where(s, cmd, |_, n| match &attr {
        Attribute::Item(it) => n.appearance.items.contains(it),
        Attribute::Effect(e) => n.appearance.effects.iter().any(|x| x.id == e.id && x.params == e.params),
    })
}

fn same_layers(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    // The layers or sublayers the selected objects are on (art in their sublayers too).
    let layers: BTreeSet<NodeId> = st.selection.objects.iter().filter_map(|id| st.doc.layer_containing(*id)).collect();
    let ids: Vec<NodeId> = layers.iter().filter_map(|l| st.doc.node(*l)).flat_map(|l| l.layer_art(true)).collect();
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
