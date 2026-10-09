//! Layers panel, artboards.
//!
//! The Layers panel works on rows: layers, sublayers, groups and objects. Clicking a row makes it
//! the highlighted row (`layer.setCurrent`; Shift and Ctrl change several, `layer.highlight`) and
//! its layer the current one, where new art goes. Panel commands without `ids` act on the
//! highlighted rows, else on the current layer. More panel-menu operations live in
//! [`super::layerpanel`].

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Artboard, Document, LAYER_COLORS, LayerColor, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Affine, Rect};

use super::*;
use crate::EngineError;

/// Most rows one panel command takes (ids are untrusted input).
pub(crate) const MAX_ROWS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.new",
            "New Layer…",
            ["Window", "Layers"],
            Some("Cmd+L"),
            "{name?, top?: bool, color?, template?, locked?, visible?, printable?, preview?, dimImages?} a layer directly above the current layer, at its level (a sublayer beside a current sublayer); `top`: above all layers (Ctrl-click the button). Options as layer.setProps → {id}",
            has_doc,
            new_layer
        ),
        cmd!(
            "layer.newSublayer",
            "New Sublayer…",
            ["Window", "Layers"],
            None,
            "{parent?: layer id (default: the highlighted row's layer, else the current layer), name?, options as layer.new} a sublayer on top of the parent's contents → {id}",
            has_doc,
            new_sublayer
        ),
        cmd!(
            "layer.delete",
            "Delete Selection",
            ["Window", "Layers"],
            None,
            "{ids?|id?} delete rows: layers, sublayers, groups or objects with what they hold (default: the highlighted rows, else the current layer). The last layer stays → {deleted}",
            has_doc,
            delete_rows
        ),
        cmd!(
            "layer.duplicate",
            "Duplicate Selection",
            ["Window", "Layers"],
            None,
            "{ids?|id?} copy rows (default: the highlighted rows, else the current layer), each just above its original; a layer's copy is named `<name> copy`. The copies are highlighted → {ids}",
            has_doc,
            duplicate_rows
        ),
        cmd!(
            "layer.setCurrent",
            "Set Current Layer",
            [],
            None,
            "{id} click a row: it alone is highlighted, and the layer it is (or the layer it is in) becomes the current layer, where new art goes. Not an undo step → {id, layer}",
            has_doc,
            set_current
        ),
        cmd!(
            "layer.highlight",
            "Highlight Rows",
            [],
            None,
            "{ids, mode?: \"set\"|\"add\"|\"toggle\"} highlight Layers panel rows (Shift-click: a range, Ctrl-click: toggle one); the last one's layer becomes current. Not an undo step → {rows}",
            has_doc,
            highlight
        ),
        cmd!(
            "layer.setProps",
            "Layer Options…",
            ["Window", "Layers"],
            None,
            "{ids?|id? (default: the highlighted rows, else the current layer), name?, visible?, locked?, template?, printable?, preview?, dimImages?: 0..100|false, color?: index 0..26|\"#rrggbb\"|preset name} Layer Options; rows that are not layers take name, visible and locked. Template on also locks the layer and dims its images to 50% (unless given); off unlocks it and stops dimming. One undo step",
            has_doc,
            set_props
        ),
        cmd!(
            "layer.collectInNew",
            "Collect in New Layer",
            ["Window", "Layers"],
            None,
            "{ids?} move rows (default: the highlighted rows, else the selected objects) into a new layer where the topmost of them was (a sublayer when they are in a layer) → {id}",
            has_doc,
            collect
        ),
        cmd!(
            "layer.selectAll",
            "Select Art",
            [],
            None,
            "{id, add?: bool} the Layers panel's selection column: select the visible, unlocked art of a layer (its sublayers' too), or the object itself; `add` (Shift-click) adds it to the selection, or removes it when it is all selected already → {selected: [..]}",
            has_doc,
            select_art
        ),
        cmd!(
            "layer.move",
            "Move Rows",
            [],
            None,
            "{ids, target, place?: \"above\"|\"below\"|\"inside\" (default inside), copy?: bool} drag rows in the Layers panel: above or below the target row (beside it, in its layer or group), or inside a layer or group (on top of its contents). Layers go only in layers or at the top level, objects in layers and groups; an object placed beside a top-level layer goes inside it. A row never goes into itself; a locked layer or group takes nothing. `copy` (Alt-drag) moves copies. One undo step → {ids}",
            has_doc,
            move_rows
        ),
        cmd!(
            "node.move",
            "Reorder",
            [],
            None,
            "{id, parent?: id (null = top level), index} put a row at `index` (0 = bottom) of `parent` (layers only at the top level or in layers)",
            has_doc,
            move_node
        ),
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
            "{index, name?, x?, y?, width?, height?, scaleArt?: bool (Scale Artwork with Artboard: resized, the artboard takes the art fully inside it (locked and hidden art only with lockedAndHidden?: bool, default prefs moveLockedWithArtboard) and its guides from its old rectangle onto the new one, each side by its own ratio; strokes/corners/patterns as object.scale)} → null, or with scaleArt {scaled: the art scaled}",
            has_doc,
            artboard_set
        ),
        cmd!("artboard.fitToArt", "Fit to Artwork Bounds", ["Object", "Artboards"], None, "{index?}", has_doc, artboard_fit_art),
        cmd!("artboard.fitToSelection", "Fit to Selected Art", ["Object", "Artboards"], None, "{index?}", has_selection, artboard_fit_sel),
        cmd!(
            "layer.clippingMask.toggle",
            "Make/Release Clipping Mask",
            ["Window", "Layers"],
            None,
            "{id?: layer or group (default: the one highlighted layer or group row, else the one selected group, else the current layer)} make: its top object (a path, compound path or text, which loses its paint) clips the rest and moves to the bottom (new art added on top is clipped); release: it stops clipping, the clipping path stays unpainted → {clip}",
            has_doc,
            clip_toggle
        ),
        cmd!(
            "layer.target",
            "Target",
            [],
            None,
            "{id} target a layer, group or object as clicking its target circle in the Layers panel does: a layer gets its visible, unlocked art (its sublayers' too) selected and becomes the current layer, and appearance.*, effect.*, transparency.* and the opacity-mask commands without `ids` then act on the layer itself; anything else is selected. Any other selection change ends the targeting (`document.inspect` → target) → {id, selected: [..]}",
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

/// The rows a panel command acts on: `ids` or `id`, else the highlighted rows, else the current
/// layer (capped at [`MAX_ROWS`]).
pub(crate) fn rows_param(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let mut ids = match (ids_param(p, "ids"), id_param(p, "id")) {
        (Some(ids), _) => ids,
        (None, Some(id)) => vec![id],
        (None, None) => {
            let st = s.doc()?;
            let rows = st.highlighted_rows();
            if rows.is_empty() { st.current_layer().into_iter().collect() } else { rows }
        }
    };
    ids.truncate(MAX_ROWS);
    Ok(ids)
}

/// `ids` that exist, without repeats or rows inside another listed row, in paint order (bottom
/// first).
pub(crate) fn row_roots(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let set: HashSet<NodeId> = ids.iter().copied().filter(|i| d.node(*i).is_some()).collect();
    let keep: Vec<NodeId> = set
        .iter()
        .copied()
        .filter(|id| d.ancestry(*id).is_some_and(|a| a.split_last().is_none_or(|(_, above)| !above.iter().any(|x| set.contains(x)))))
        .collect();
    d.paint_order(keep)
}

/// Whether `parent` (None: the top level) can hold `n`: layers go at the top level or in layers,
/// anything else in layers and groups.
pub(crate) fn accepts(d: &Document, parent: Option<NodeId>, n: &Node) -> bool {
    match parent {
        None => n.is_layer(),
        Some(p) => match d.node(p).map(|x| &x.kind) {
            Some(NodeKind::Layer { .. }) => true,
            Some(NodeKind::Group { .. }) => !n.is_layer(),
            // A compound shape takes what covers a region.
            Some(NodeKind::CompoundShape { .. }) => {
                !matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Image(_) | NodeKind::SymbolInstance { .. } | NodeKind::PlacedDocument(_))
            }
            _ => false,
        },
    }
}

/// Whether `id` or a container around it is locked.
fn locked_within(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).unwrap_or_default().iter().any(|a| d.node(*a).is_some_and(|n| n.locked))
}

/// Every layer and sublayer, depth first.
fn all_layers(d: &Document) -> Vec<&Node> {
    let mut out = vec![];
    d.walk(|n| {
        if n.is_layer() {
            out.push(n);
        }
    });
    out
}

/// A new layer named `name` (default: the next `Layer N`) in the next layer colour.
pub(crate) fn make_layer(d: &mut Document, name: Option<&str>) -> Node {
    let n = all_layers(d).len();
    let name = name.map(str::to_string).unwrap_or_else(|| d.next_layer_name());
    let id = d.alloc_id();
    Node::layer(id, &name, LayerColor::Preset((n % LAYER_COLORS.len()) as u8))
}

/// A layer colour: a preset index (0–26), `#rrggbb` or a preset's name (any case).
fn layer_color_param(v: &Value) -> std::result::Result<LayerColor, String> {
    match v {
        Value::Number(n) => match n.as_u64() {
            Some(i) if (i as usize) < LAYER_COLORS.len() => Ok(LayerColor::Preset(i as u8)),
            _ => Err(format!("a colour index is 0 to {}, not {n}", LAYER_COLORS.len() - 1)),
        },
        Value::String(s) => {
            if let Some(i) = LAYER_COLORS.iter().position(|(name, _)| name.eq_ignore_ascii_case(s.trim())) {
                return Ok(LayerColor::Preset(i as u8));
            }
            let c = vectorcraft_color::Color::from_hex(s).ok_or_else(|| format!("`{s}` is not a colour (#rrggbb or a preset name)"))?;
            let [r, g, b, _] = c.to_rgba8(1.0);
            Ok(LayerColor::Custom([r, g, b]))
        }
        _ => Err(format!("a colour is an index, #rrggbb or a preset name, not {v}")),
    }
}

/// Layer Options in `p` applied to layer or row `n` (validated first by [`check_options`]).
fn apply_options(n: &mut Node, p: &Value) {
    if let Some(v) = str_param(p, "name") {
        if v.trim().is_empty() {
            // A layer keeps a name; an object goes back to its generated `<Kind>` one.
            if !n.is_layer() {
                n.name = None;
            }
        } else {
            n.name = Some(v.chars().take(512).collect());
        }
    }
    if let Some(v) = p.get("visible").and_then(Value::as_bool) {
        n.visible = v;
    }
    let locked = p.get("locked").and_then(Value::as_bool);
    if let Some(v) = locked {
        n.locked = v;
    }
    let mut lock = None;
    if let NodeKind::Layer { template, printable, color, preview, dim_images, .. } = &mut n.kind {
        let dim_given = p.get("dimImages").is_some();
        if let Some(v) = p.get("template").and_then(Value::as_bool)
            && v != *template
        {
            *template = v;
            // A template is locked and its images dimmed; it stops being either with it.
            lock = Some(v);
            if !dim_given {
                *dim_images = v.then_some(50);
            }
        }
        if let Some(v) = p.get("printable").and_then(Value::as_bool) {
            *printable = v;
        }
        if let Some(v) = p.get("preview").and_then(Value::as_bool) {
            *preview = v;
        }
        match p.get("dimImages") {
            Some(Value::Bool(false) | Value::Null) => *dim_images = None,
            Some(Value::Bool(true)) => *dim_images = Some(50),
            Some(v) => {
                if let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                    *dim_images = Some(x.clamp(0.0, 100.0).round() as u8);
                }
            }
            None => {}
        }
        if let Some(c) = p.get("color").and_then(|v| layer_color_param(v).ok()) {
            *color = c;
        }
    }
    if let (Some(v), None) = (lock, locked) {
        n.locked = v;
    }
}

/// Bad values in Layer Options params, as an error for `cmd`.
fn check_options(cmd: &str, p: &Value) -> Result<()> {
    if let Some(v) = p.get("color") {
        layer_color_param(v).map_err(|e| bad(cmd, e))?;
    }
    for k in ["visible", "locked", "template", "printable", "preview"] {
        if p.get(k).is_some_and(|v| !v.is_boolean()) {
            return Err(bad(cmd, format!("`{k}` is true or false")));
        }
    }
    if let Some(v) = p.get("dimImages")
        && !(v.is_boolean() || v.is_null() || v.as_f64().is_some_and(f64::is_finite))
    {
        return Err(bad(cmd, "`dimImages` is a percentage or false"));
    }
    Ok(())
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
    let rows = st.layer_rows.clone();
    s.select(|d, sel| sel.set_target(d, id))?;
    let st = s.doc_mut()?;
    // Targeting doesn't change which rows are highlighted.
    st.layer_rows = rows;
    if layer {
        st.active_layer = Some(id);
    }
    Ok(json!({ "id": id.0, "selected": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The Layers panel's clipping mask button: the top object of a layer (or group) clips the rest,
/// or the clipping mask is released.
fn clip_toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let container = |id: &NodeId| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }));
    let row = match st.highlighted_rows().as_slice() {
        [id] if container(id) => Some(*id),
        _ => None,
    };
    let group = match st.selection.objects.as_slice() {
        [id] if matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Group { .. })) => Some(*id),
        _ => None,
    };
    let id = id_param(p, "id").or(row).or(group).or(st.current_layer()).ok_or_else(|| bad("layer.clippingMask.toggle", "no layer"))?;
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

/// A new layer with the options in `p`, at `parent`/`index`; it becomes current and highlighted.
fn add_layer(s: &mut Session, p: &Value, cmd: &str, label: &str, place: impl FnOnce(&Document) -> Result<(Option<NodeId>, usize)>) -> Result<Value> {
    check_options(cmd, p)?;
    let name = str_param(p, "name").filter(|n| !n.trim().is_empty()).map(str::to_string);
    let id = s.edit(label, |d, _| {
        let (parent, index) = place(d)?;
        let mut n = make_layer(d, name.as_deref());
        apply_options(&mut n, p);
        Ok(d.insert(parent, index, n)?)
    })?;
    let st = s.doc_mut()?;
    st.active_layer = Some(id);
    st.layer_rows = vec![id];
    Ok(json!({ "id": id.0 }))
}

fn new_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    // Above the current layer, at its level; on top of everything with `top`.
    let cur = st.current_layer().filter(|_| !bool_or(p, "top", false));
    add_layer(s, p, "layer.new", "New Layer", |d| match cur.and_then(|c| d.position(c)) {
        Some((parent, i, _)) => Ok((parent, i + 1)),
        None => Ok((None, usize::MAX)),
    })
}

fn new_sublayer(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.newSublayer";
    let st = s.doc()?;
    let row = st.highlighted_rows().last().and_then(|r| st.doc.layer_containing(*r));
    let parent = id_param(p, "parent").or(row).or(st.current_layer()).ok_or_else(|| bad(C, "no parent layer"))?;
    if !st.doc.node(parent).is_some_and(Node::is_layer) {
        return Err(bad(C, "the parent must be a layer or sublayer"));
    }
    add_layer(s, p, C, "New Sublayer", |_| Ok((Some(parent), usize::MAX)))
}

fn delete_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = rows_param(s, p)?;
    let st = s.doc()?;
    let ids = row_roots(&st.doc, &ids);
    if ids.is_empty() {
        return Err(bad("layer.delete", "no such rows"));
    }
    if st.doc.layers.iter().all(|l| ids.contains(&l.id)) {
        return Err(EngineError::Other("a document needs at least one layer".into()));
    }
    s.edit("Delete Selection", |d, sel| {
        for id in &ids {
            d.remove(*id)?;
        }
        sel.prune(d);
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.layer_rows.clear();
    if st.current_layer().is_none() {
        st.active_layer = st.doc.default_layer();
    }
    Ok(json!({ "deleted": ids.len() }))
}

fn duplicate_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = rows_param(s, p)?;
    let ids = row_roots(&s.doc()?.doc, &ids);
    if ids.is_empty() {
        return Err(bad("layer.duplicate", "no such rows"));
    }
    let copies = s.edit("Duplicate Selection", |d, _| {
        let mut out = vec![];
        for id in &ids {
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
            let mut c = d.reid(&n);
            if n.is_layer() {
                c.name = Some(format!("{} copy", n.display_name()));
            }
            out.push(d.insert(par, idx + 1, c)?);
        }
        Ok(out)
    })?;
    s.doc_mut()?.layer_rows = copies.clone();
    Ok(json!({ "ids": copies.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn set_current(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.setCurrent";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let st = s.doc_mut()?;
    let layer = st.doc.layer_containing(id).ok_or(EngineError::NoNode(id))?;
    st.active_layer = Some(layer);
    st.layer_rows = vec![id];
    st.revision += 1;
    Ok(json!({ "id": id.0, "layer": layer.0 }))
}

fn highlight(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.highlight";
    let mut ids = ids_param(p, "ids").ok_or_else(|| bad(C, "missing ids"))?;
    ids.truncate(MAX_ROWS);
    let mode = str_param(p, "mode").unwrap_or("set");
    let st = s.doc_mut()?;
    ids.retain(|id| st.doc.node(*id).is_some());
    let mut rows = st.highlighted_rows();
    match mode {
        "set" => rows.clear(),
        "add" | "toggle" => {}
        other => return Err(bad(C, format!("mode is set, add or toggle, not `{other}`"))),
    }
    let mut last = None;
    for id in ids {
        if mode == "toggle" && rows.contains(&id) {
            rows.retain(|r| *r != id);
        } else if !rows.contains(&id) {
            rows.push(id);
            last = Some(id);
        }
    }
    if let Some(layer) = last.or(rows.last().copied()).and_then(|id| st.doc.layer_containing(id)) {
        st.active_layer = Some(layer);
    }
    st.layer_rows = rows;
    st.revision += 1;
    Ok(json!({ "rows": st.layer_rows.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.setProps";
    check_options(C, p)?;
    let ids = rows_param(s, p)?;
    if ids.is_empty() {
        return Err(bad(C, "no rows"));
    }
    s.edit("Layer Options", |d, sel| {
        for id in &ids {
            apply_options(d.node_mut(*id).ok_or(EngineError::NoNode(*id))?, p);
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

fn collect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.collectInNew";
    let ids = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => {
            let rows = s.doc()?.highlighted_rows();
            if rows.is_empty() { super::edit::selected_roots(s)? } else { rows }
        }
    };
    let ids = row_roots(&s.doc()?.doc, &ids);
    let Some(top) = ids.last().copied() else { return Err(bad(C, "select rows or objects to collect")) };
    let lid = s.edit("Collect in New Layer", |d, sel| {
        // Where the topmost of them is: beside it in its layer, or on top of the layer holding
        // its group.
        let (parent, index) = match d.position(top) {
            Some((par, i, _)) if par.is_none_or(|p| d.node(p).is_some_and(Node::is_layer)) => (par, i + 1),
            _ => (d.layer_containing(top), usize::MAX),
        };
        let layer = make_layer(d, None);
        let lid = d.insert(parent, index, layer)?;
        for id in &ids {
            d.move_node(*id, Some(lid), usize::MAX)?;
        }
        sel.prune(d);
        Ok(lid)
    })?;
    let st = s.doc_mut()?;
    st.active_layer = Some(lid);
    st.layer_rows = vec![lid];
    Ok(json!({ "id": lid.0 }))
}

/// The selection column: select a row's art.
fn select_art(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.selectAll";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let add = bool_or(p, "add", false);
    let st = s.doc()?;
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    let ids: Vec<NodeId> = if !st.doc.is_editable(id) {
        vec![]
    } else if n.is_layer() {
        n.layer_art(true)
    } else {
        vec![id]
    };
    let all_selected = !ids.is_empty() && ids.iter().all(|i| st.selection.contains(*i));
    let rows = st.layer_rows.clone();
    s.select(|_, sel| {
        if !add {
            sel.set(ids.iter().copied());
        } else if all_selected {
            for i in &ids {
                sel.remove(*i);
            }
        } else {
            for i in &ids {
                sel.add(*i);
            }
        }
    })?;
    let st = s.doc_mut()?;
    // The selection column leaves the highlighted rows as they are.
    st.layer_rows = rows;
    Ok(json!({ "selected": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Above,
    Below,
    Inside,
}

/// Drag rows in the Layers panel.
fn move_rows(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.move";
    let mut ids = ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])).ok_or_else(|| bad(C, "missing ids"))?;
    ids.truncate(MAX_ROWS);
    let target = id_param(p, "target").ok_or_else(|| bad(C, "missing target"))?;
    let place = match str_param(p, "place").unwrap_or("inside") {
        "above" => Place::Above,
        "below" => Place::Below,
        "inside" | "into" => Place::Inside,
        other => return Err(bad(C, format!("place is above, below or inside, not `{other}`"))),
    };
    let copy = bool_or(p, "copy", false);
    let st = s.doc()?;
    let d = &st.doc;
    d.node(target).ok_or(EngineError::NoNode(target))?;
    let ids = row_roots(d, &ids);
    if ids.is_empty() {
        return Err(bad(C, "no such rows"));
    }
    // Never into itself or a row inside it.
    let around = d.ancestry(target).unwrap_or_default();
    if ids.iter().any(|i| around.contains(i)) {
        return Err(bad(C, "a row can't go into itself"));
    }
    let nodes: Vec<&Node> = ids.iter().filter_map(|i| d.node(*i)).collect();
    let (parent, anchor) = match place {
        Place::Inside => (Some(target), None),
        _ => match d.parent_of(target) {
            // Objects can't be at the top level: beside a top-level layer means in it.
            None if nodes.iter().any(|n| !n.is_layer()) => (Some(target), None),
            parent => (parent, Some(target)),
        },
    };
    if let Some(n) = nodes.iter().find(|n| !accepts(d, parent, n)) {
        let into = parent.and_then(|p| d.node(p)).map_or("the top level", |p| if p.is_layer() { "a layer" } else { p.kind_label() });
        return Err(bad(C, format!("{} can't go into {into}", n.display_name())));
    }
    if parent.is_some_and(|p| locked_within(d, p)) {
        return Err(EngineError::Other("the layer or group is locked".into()));
    }
    let label = if copy { "Duplicate Rows" } else { "Move Rows" };
    let moved = s.edit(label, |d, sel| {
        let mut nodes = vec![];
        for id in &ids {
            // An object joining a compound shape (or leaving one) takes the Add mode.
            let stays = d.parent_of(*id) == parent;
            let mut n = if copy {
                let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
                d.reid(&n)
            } else {
                Arc::unwrap_or_clone(d.remove(*id)?)
            };
            if !stays {
                n.shape_mode = vectorcraft_doc::ShapeMode::Add;
            }
            nodes.push(n);
        }
        let count = d.children(parent).map_or(0, Vec::len);
        let mut index = match anchor {
            None => count,
            Some(a) => {
                let (_, i, _) = d.position(a).ok_or(EngineError::NoNode(a))?;
                if place == Place::Above { i + 1 } else { i }
            }
        };
        // The clipping path stays at the bottom of a clip group or a clipping layer.
        if count > 0 && parent.and_then(|p| d.node(p)).is_some_and(Node::clips) {
            index = index.max(1);
        }
        let mut out = vec![];
        for n in nodes {
            out.push(d.insert(parent, index, n)?);
            index += 1;
        }
        sel.prune(d);
        Ok(out)
    })?;
    let st = s.doc_mut()?;
    st.layer_rows = moved.clone();
    Ok(json!({ "ids": moved.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn move_node(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "node.move";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let parent = id_param(p, "parent");
    let index = p.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX).min(usize::MAX as u64) as usize;
    let d = &s.doc()?.doc;
    let n = d.node(id).ok_or(EngineError::NoNode(id))?;
    if !accepts(d, parent, n) {
        return Err(bad(C, "layers go only at the top level or in layers, objects in layers and groups"));
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
        d.artboards.push(Artboard { id, name, rect: r, show_center_mark: false, show_cross_hairs: false });
        Ok(d.artboards.len() - 1)
    })?;
    Ok(json!({ "index": idx }))
}

fn artboard_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("artboard.delete", "missing index"))? as usize;
    s.edit("Delete Artboard", |d, sel| {
        if d.artboards.len() <= 1 {
            return Err(EngineError::Other("a document needs at least one artboard".into()));
        }
        if i >= d.artboards.len() {
            return Err(EngineError::Other("no such artboard".into()));
        }
        // Its guides go with it.
        let id = d.artboards.remove(i).id;
        d.retain_guides(sel, |_, g| g.artboard != Some(id));
        Ok(())
    })?;
    ok()
}

fn artboard_set(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let was = s.doc()?.doc.artboards.get(i).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    let now = rect_from(p, was);
    let resized = (now.width() - was.width()).abs() > 1e-6 || (now.height() - was.height()).abs() > 1e-6;
    // Scale Artwork with Artboard: resized, it takes the art fully inside it (as Move Artwork with
    // Artboard gathers it) and its guides from the old rectangle onto the new one.
    let scale_art = bool_or(p, "scaleArt", false);
    let fit = if scale_art && resized { rect_map(was, now) } else { None };
    let art = if fit.is_some() {
        // Locked and hidden art too? Noted in the journal, so a replay scales the same objects
        // whatever the preference is then.
        let all = bool_or(p, "lockedAndHidden", s.prefs.move_locked_with_artboard);
        s.note_journal("lockedAndHidden", json!(all));
        s.doc()?.doc.art_on_artboard(was, all)
    } else {
        vec![]
    };
    let mut sc = match fit {
        Some(xf) if Scaling::factor(xf).is_some() => super::object::scaling(s, p),
        _ => Scaling::default(),
    };
    sc.patterns = super::object::transform_patterns(s, p, &art)?;
    s.edit("Artboard Options", |d, _| {
        let a = d.artboards.get_mut(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        a.rect = now;
        if let Some(n) = str_param(p, "name") {
            a.name = n.to_string();
        }
        let id = a.id;
        if let Some(xf) = fit {
            d.map_artboard_guides(id, xf);
            for id in &art {
                if let Some(n) = d.node_mut(*id) {
                    n.transform(xf, sc);
                }
            }
        } else if !resized {
            // Moved (not resized), it takes its guides along.
            d.move_artboard_guides(id, now.origin() - was.origin());
        }
        Ok(())
    })?;
    if !scale_art {
        return ok();
    }
    Ok(json!({ "scaled": art.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The map taking artboard rectangle `from` onto `to`, each side scaled by its own ratio. None
/// when `from` has no area or the map isn't finite (a size out of range).
fn rect_map(from: Rect, to: Rect) -> Option<Affine> {
    if from.width() <= 1e-9 || from.height() <= 1e-9 {
        return None;
    }
    let (sx, sy) = (to.width() / from.width(), to.height() / from.height());
    let xf = Affine::translate(to.origin().to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-from.origin().to_vec2());
    xf.is_finite().then_some(xf)
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
