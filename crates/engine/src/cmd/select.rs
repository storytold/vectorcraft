//! Select menu.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::wand::{fill_of, stroke_of};
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
        cmd!(
            "select.allOnArtboard",
            "All on Active Artboard",
            ["Select"],
            Some("Cmd+Alt+A"),
            "{artboard?: index (0-based; the app passes the active one; default: the first)}",
            has_doc,
            all_on_artboard
        ),
        cmd!("select.none", "Deselect", ["Select"], Some("Cmd+Shift+A"), "{}", has_doc, none),
        cmd!("select.reselect", "Reselect", ["Select"], Some("Cmd+6"), "{}", has_doc, reselect),
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
            "Cancel Key Object",
            [],
            None,
            "{id?: a selected top-level object (not a layer, a compound-path member or an object inside another selected object); one of two or more or the only one with ruler guides selected (the Selection tool: a click on one object of a selection of several, or of one with guides; on the key again: no id), guide?: index (a selected ruler guide, guide.list, with one or more objects selected: the Selection tool's click on a selected guide; Align lines the objects up on it and the guide stays put)} the key object Align aligns to and Distribute Spacing spaces from; without `id` or `guide` it cancels the key, as does a selection that holds only the key and objects inside it (one object, or a group key and its own members) and no ruler guide, or a key guide with no object selected or taken out of the selection; any other id or guide is an error that leaves the key as it was → {key, guide?} (the resulting key object, or null; guide: the key guide, left out without one)",
            has_doc,
            key
        ),
        cmd!("select.anchors", "Select Anchors", [], None, "{id, anchors: [[subpath, anchor]…], mode: \"set\"|\"add\"|\"toggle\"}", has_doc, anchors),
        cmd!(
            "select.segments",
            "Select Segments",
            [],
            None,
            "{id, segments: [[subpath, segment]…]} select path segments; their endpoints move with them, Delete removes only their connections",
            has_doc,
            segments
        ),
        cmd!(
            "select.anchorsMany",
            "Select Anchors",
            [],
            None,
            "{items: [{id, anchors}], mode?: \"set\"|\"add\"|\"toggle\"|\"subtract\" (set by default; add: true is mode add), add?: bool (an older way to ask for mode add)}: toggle selects the anchors not selected and deselects the others, subtract deselects them; a path left with none leaves the selection",
            has_doc,
            anchors_many
        ),
        cmd!(
            "select.color",
            "Select by Color",
            [],
            None,
            "{fill?: colour, stroke?: colour (as paint.setFill's color; at least one)} select the objects whose fill (and stroke) reads as that colour in #rrggbb, as document.inspect reports colours, whatever its swatch or tint; with both, the objects that match both. No object needs to be selected first; Select > Reselect repeats it → {count}",
            has_doc,
            select_color
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
                same(s, "select.same.fillColor", |a, b| same_paint(&fill_of(a), &fill_of(b), tint))
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
                same(s, "select.same.strokeColor", |a, b| same_paint(&stroke_of(a).0, &stroke_of(b).0, tint))
            }
        ),
        cmd!("select.same.strokeWeight", "Stroke Weight", ["Select", "Same"], None, "{}", has_selection, |s, _| same(
            s,
            "select.same.strokeWeight",
            |a, b| (stroke_weight(a) - stroke_weight(b)).abs() < 1e-9
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
                    same_paint(&fill_of(a), &fill_of(b), tint) && same_paint(&stroke_of(a).0, &stroke_of(b).0, tint)
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
/// through sublayers, which are not objects). In isolation mode there are none while the isolated
/// container, or a group or layer it is in, is hidden, locked or a template layer.
fn selectable(d: &Document, iso: Option<NodeId>) -> Vec<NodeId> {
    match iso.and_then(|i| d.node(i)) {
        Some(c) if d.ancestry(c.id).is_some_and(|a| a.iter().all(|i| d.node(*i).is_some_and(|n| n.visible && !n.locked && !n.is_template()))) => {
            // A single object isolated is all there is to select.
            if c.is_container() { c.layer_art(true) } else { vec![c.id] }
        }
        Some(_) => vec![],
        None => d.selectable_art(),
    }
}

/// Visit, in paint order, the objects Select All takes ([`selectable`]) and the objects inside
/// them, leaving out every hidden or locked object with what it holds. `f` returns whether to
/// visit the objects inside the one it is given.
pub(crate) fn walk_selectable(d: &Document, iso: Option<NodeId>, mut f: impl FnMut(&Node) -> bool) {
    fn inside(n: &Node, f: &mut impl FnMut(&Node) -> bool) {
        if n.visible && !n.locked && f(n) {
            for c in n.children().into_iter().flatten() {
                inside(c, f);
            }
        }
    }
    fn find(n: &Node, tops: &BTreeSet<NodeId>, f: &mut impl FnMut(&Node) -> bool) {
        if tops.contains(&n.id) {
            return inside(n, f);
        }
        for c in n.children().into_iter().flatten() {
            find(c, tops, f);
        }
    }
    let tops: BTreeSet<NodeId> = selectable(d, iso).into_iter().collect();
    for l in &d.layers {
        find(l, &tops, &mut f);
    }
}

/// The objects [`walk_selectable`] visits.
pub(crate) fn selectable_ids(d: &Document, iso: Option<NodeId>) -> BTreeSet<NodeId> {
    let mut ids = BTreeSet::new();
    walk_selectable(d, iso, |n| {
        ids.insert(n.id);
        true
    });
    ids
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
    // Symbol instances are measured by their art.
    let symbols = st.doc.symbol_boxes();
    let ids: Vec<NodeId> = selectable(&st.doc, st.isolation)
        .into_iter()
        .filter(|id| {
            st.doc
                .node(*id)
                .and_then(|n| n.geometric_bounds_with(&symbols))
                .is_some_and(|b| b.intersect(r).area() > 0.0 || vectorcraft_doc::artboard_holds(r, b.center()))
        })
        .collect();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    ok()
}

fn none(s: &mut Session, _: &Value) -> Result<Value> {
    s.select(|_, sel| sel.clear())?;
    ok()
}

fn reselect(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let Some((c, p, from)) = st.last_selection_cmd.clone() else { return ok() };
    // After Deselect, a Same command takes its reference again from the objects that were selected
    // when it ran, those still there (#903).
    let from: Vec<NodeId> = from.into_iter().filter(|id| st.doc.node(*id).is_some()).collect();
    let restore = st.selection.objects.is_empty() && !from.is_empty();
    if restore {
        s.select(|_, sel| sel.set(from))?;
    }
    let r = s.execute(&c, &p);
    if restore && r.is_err() {
        s.select(|_, sel| sel.clear())?;
    }
    r
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
    let Some((par, idx, _)) = st.doc.position(cur) else { return ok() };
    let Some(siblings) = st.doc.children(par) else { return ok() };
    // The nearest sibling in that direction that `walk_selectable` visits: hidden and locked
    // objects and sublayers are passed over.
    let can = selectable_ids(&st.doc, st.isolation);
    let next = if dir > 0 {
        siblings.iter().skip(idx + 1).find(|n| can.contains(&n.id))
    } else {
        siblings.iter().take(idx).rev().find(|n| can.contains(&n.id))
    };
    let Some(id) = next.map(|n| n.id) else { return ok() };
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
    let id = p.get("id").map(|value| checked_id(s, value, "select.key")).transpose()?;
    let guide = match p.get("guide").filter(|v| !v.is_null()) {
        Some(v) => {
            Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("select.key", "`guide` must be a guide index (guide.list)"))?)
        }
        None => None,
    };
    if let Some(g) = guide {
        let sel = &s.doc()?.selection;
        if id.is_some() {
            return Err(bad("select.key", "give `id` or `guide`, not both"));
        }
        if !sel.guides.contains(&g) {
            return Err(bad("select.key", format!("guide {g} is not selected")));
        }
        if sel.is_empty() {
            return Err(bad("select.key", "select objects to align to the guide"));
        }
        s.select(|_, sel| {
            sel.key = None;
            sel.key_guide = Some(g);
        })?;
        return key_result(s);
    }
    if let Some(id) = id {
        let st = s.doc()?;
        // A selected top-level object is the only top-level object among the selected ones of
        // itself and its ancestors.
        let selected = st.doc.ancestry(id).unwrap_or_default().into_iter().filter(|a| st.selection.contains(*a)).collect();
        if super::edit::roots_of(&st.doc, selected) != [id] {
            return Err(bad("select.key", format!("object {id} is not a selected top-level object")));
        }
    }
    s.select(|_, sel| {
        sel.key = id;
        sel.key_guide = None;
    })?;
    key_result(s)
}

/// `{key}`, with `guide` when a guide is the key.
fn key_result(s: &Session) -> Result<Value> {
    let sel = &s.doc()?.selection;
    let mut out = json!({ "key": sel.key });
    if let Some(g) = sel.key_guide {
        out["guide"] = json!(g);
    }
    Ok(out)
}

/// `[[subpath, anchor]…]` anchor references of `cmd`: a malformed entry fails rather than being
/// left out, which would act on other anchors than the ones given.
pub(crate) fn checked_refs(v: Option<&Value>, cmd: &str) -> Result<Vec<(usize, usize)>> {
    let refs = v.and_then(Value::as_array).ok_or_else(|| bad(cmd, "`anchors` must be an array of [subpath, anchor] pairs"))?;
    refs.iter()
        .map(|v| {
            let Some([subpath, anchor]) = v.as_array().map(Vec::as_slice) else {
                return Err(bad(cmd, format!("invalid anchor reference {v}: expected [subpath, anchor]")));
            };
            let coordinate = |v: &Value| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| bad(cmd, format!("invalid anchor index {v}: expected a non-negative integer")))
            };
            Ok((coordinate(subpath)?, coordinate(anchor)?))
        })
        .collect()
}

fn segments(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.segments";
    let id = checked_id(s, p.get("id").ok_or_else(|| bad(C, "missing id"))?, C)?;
    let refs: BTreeSet<_> = checked_refs(p.get("segments"), C)?.into_iter().collect();
    let path = s.doc()?.doc.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(C, "object is not a path"))?;
    let mut endpoints = BTreeSet::new();
    for &(si, seg) in &refs {
        let sp = path.subpaths.get(si).filter(|sp| seg < sp.segment_count()).ok_or_else(|| bad(C, "segment does not exist"))?;
        endpoints.extend([(si, seg), (si, (seg + 1) % sp.anchors.len())]);
    }
    s.select(|_, sel| {
        sel.clear();
        if !refs.is_empty() {
            sel.add(id);
            sel.anchors.insert(id, endpoints);
            sel.segments.insert(id, refs);
        }
    })?;
    ok()
}

fn anchors(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.anchors";
    let id = checked_id(s, p.get("id").ok_or_else(|| bad(C, "missing id"))?, C)?;
    let refs = checked_refs(p.get("anchors"), C)?;
    let mode = match str_param(p, "mode") {
        None | Some("set") => "set",
        Some("add") => "add",
        Some("toggle") => "toggle",
        Some(m) => return Err(bad(C, format!("unknown mode {m:?}"))),
    };
    s.select(|d, sel| {
        if mode == "set" {
            sel.clear();
        }
        // A path selected as a whole has every anchor selected: toggle and add start from them all.
        let whole = sel.contains(id) && sel.partial(id).is_none();
        sel.add(id);
        // A segment pick's endpoints aren't selected anchors: start from none of them.
        if sel.segments.remove(&id).is_some() {
            sel.anchors.remove(&id);
        }
        let e = sel.anchors.entry(id).or_default();
        if whole && let Some(path) = d.node(id).and_then(|n| n.path_data()) {
            e.extend(path.anchors().map(|(s, i, _)| (s, i)));
        }
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
    const C: &str = "select.anchorsMany";
    let raw = p.get("items").and_then(Value::as_array).ok_or_else(|| bad(C, "`items` must be an array of {id, anchors}"))?;
    let items: Vec<(NodeId, BTreeSet<(usize, usize)>)> = raw
        .iter()
        .map(|it| {
            let id = checked_id(s, it.get("id").ok_or_else(|| bad(C, "each item needs an id"))?, C)?;
            let refs = checked_refs(it.get("anchors"), C)?;
            Ok((id, refs.into_iter().collect()))
        })
        .collect::<Result<_>>()?;
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
            // A segment pick's endpoints aren't selected anchors: start from none of them.
            let segment_pick = sel.segments.remove(&id).is_some();
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
                _ if segment_pick => BTreeSet::new(),
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

/// Select › Same › Stroke Weight: an unpainted stroke weighs nothing, as
/// [`vectorcraft_doc::Appearance::stroke_width`] has it.
fn stroke_weight(n: &Node) -> f64 {
    let (paint, width) = stroke_of(n);
    if paint.is_none() { 0.0 } else { width }
}

/// Select by Color: the objects whose fill and stroke read as the colours given, compared as the
/// `#rrggbb` `document.inspect` reports, so a script can pick art by colour without an object id.
fn select_color(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.color";
    let hex = |key: &str| match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => color_value(v).map(|c| Some(c.to_hex())).ok_or_else(|| bad(C, format!("`{key}`: not a colour: {v}"))),
    };
    let (fill, stroke) = (hex("fill")?, hex("stroke")?);
    if fill.is_none() && stroke.is_none() {
        return Err(bad(C, "give `fill`, `stroke` or both"));
    }
    let reads_as = |paint: Paint, hex: Option<&str>| hex.is_none_or(|h| paint.color().is_some_and(|c| c.to_hex() == h));
    select_where(s, C, p, |d, n| paint_reported(d, n) && reads_as(fill_of(n), fill.as_deref()) && reads_as(stroke_of(n).0, stroke.as_deref()))
}

/// Does `document.inspect` report a fill and stroke for `n`? A compound path's are its own, though
/// it holds its members, and its members report none.
fn paint_reported(d: &Document, n: &Node) -> bool {
    let compound = |n: &Node| matches!(n.kind, NodeKind::Compound { .. });
    compound(n) || (!n.is_container() && !d.parent_of(n.id).and_then(|p| d.node(p)).is_some_and(compound))
}

fn same(s: &mut Session, cmd: &str, eq: impl Fn(&Node, &Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let refn = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().ok_or_else(|| bad(cmd, "nothing selected"))?;
    select_where(s, cmd, &json!({}), |_, n| !n.is_container() && eq(n, &refn))
}

/// Select the objects [`walk_selectable`] visits that `f` accepts (not looking inside accepted
/// ones); Select > Reselect repeats `cmd` with `p`.
fn select_where(s: &mut Session, cmd: &str, p: &Value, f: impl Fn(&Document, &Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let mut ids = vec![];
    walk_selectable(&st.doc, st.isolation, |n| {
        if f(&st.doc, n) {
            ids.push(n.id);
            return false;
        }
        true
    });
    let from = st.selection.objects.clone();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    s.doc_mut()?.last_selection_cmd = Some((cmd.to_string(), p.clone(), from));
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
    select_where(s, cmd, &json!({}), |d, n| n.extras().graphic_style == Some(id) && super::style::linked_style(d, n).is_some())
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
    // The layers or sublayers the selected objects are on (art in their sublayers too).
    let layers: BTreeSet<NodeId> = st.selection.objects.iter().filter_map(|id| st.doc.layer_containing(*id)).collect();
    let mut on = BTreeSet::new();
    for l in layers.iter().filter_map(|l| st.doc.node(*l)) {
        l.walk(&mut |n| {
            on.insert(n.id);
        });
    }
    // The objects Select All takes there (in isolation mode, those in the isolated group or layer).
    let ids: Vec<NodeId> = selectable(&st.doc, st.isolation).into_iter().filter(|id| on.contains(id)).collect();
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    ok()
}

fn by_kind(s: &mut Session, f: fn(&Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let mut ids = vec![];
    walk_selectable(&st.doc, st.isolation, |n| {
        if f(n) {
            ids.push(n.id);
        }
        true
    });
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "count": ids.len() }))
}
