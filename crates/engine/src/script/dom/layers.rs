//! Layers, groups, compound paths and artboards in the script object model, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Layer/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Layers/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/GroupItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/GroupItems/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/CompoundPathItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Artboard/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Artboards/> describe them.
//!
//! A layer is named by its id and an artboard by its artboard id, so a handle keeps naming the same
//! one after others are added, moved or removed, and throws once it is gone.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `layers` lists layers top first, as the Layers panel shows them; `groupItems`,
//!   `compoundPathItems` and a layer's or group's `pageItems` list art in paint order, bottom first,
//!   as `pageItems` does. The document's `groupItems` and `compoundPathItems` hold every group or
//!   compound path at any depth; a layer's or group's hold the ones directly in it.
//! - `layers.add()` on the document makes a layer above every top-level layer; on a layer it makes
//!   a sublayer on top of that layer's contents. The new layer becomes the current one.
//! - `locked` true means the layer is locked.
//! - `clipped = true` makes the group's topmost object, which must be a path, a compound path or
//!   type, its clipping path, as the Layers panel's clipping mask button does; `false` releases it.
//! - A layer's `color` is an object `{red, green, blue}` with values from 0 to 255.
//! - `CompoundPathItems.add()` throws: a compound path holds at least one path.
//! - `artboards.add` and `artboards.insert` stop at [`super::MAX_ARTBOARDS`] artboards in a
//!   document, the limit `Documents.add` uses. Artboard names are unique: a name another artboard
//!   has throws.
//! - `Artboard.rulerOrigin` is `[x, y]` from the artboard's top-left corner with Y down; with global
//!   rulers, setting it moves the document's zero point.
//! - `setActiveArtboardIndex` changes the view's active artboard: no undo step, and the run's report
//!   lists it in `activeArtboards` for the app to show.

use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_js::{Data, HostClass, HostError, HostMember, host_ref, host_ref_parts};

use super::{
    Answer, Doc, Item, MAX_LISTED, MAX_TEXT_BYTES, NOT_FOUND, ON_LOCKED_ARTBOARD, Placement, STALE_DOCUMENT, STALE_ITEM, active_artboard, arg,
    boolean, command, doc_of, doc_ref, editable, find_doc, float, frame, get, handle_of, id_of, id_parts, item_of, item_ref, later, method, named,
    num, number, on_doc, page_items, placement_of, property, text, typename, whole, with_doc,
};
use crate::script::{Host, Link};
use crate::{DocState, Document, NodeId, NodeKind};

/// The error for a layer that was deleted, or that came from an earlier run.
pub const STALE_LAYER: &str = "Stale layer";
/// The error for an artboard that was deleted, or that came from an earlier run.
pub const STALE_ARTBOARD: &str = "Stale artboard";
/// The longest name a layer takes, in characters, as Layer Options keeps it.
pub const MAX_LAYER_NAME: usize = 512;

type Result<T> = std::result::Result<T, HostError>;

/// What a collection of art lists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Every page item.
    Any,
    /// Groups (`GroupItem`).
    Group,
    /// Compound paths (`CompoundPathItem`).
    Compound,
}

impl Kind {
    fn class(self) -> &'static str {
        match self {
            Kind::Any => "PageItems",
            Kind::Group => "GroupItems",
            Kind::Compound => "CompoundPathItems",
        }
    }

    fn holds(self, n: &crate::doc::Node) -> bool {
        match self {
            Kind::Any => typename(n).is_some(),
            Kind::Group => typename(n) == Some("GroupItem"),
            Kind::Compound => typename(n) == Some("CompoundPathItem"),
        }
    }
}

fn type_error(what: &str) -> HostError {
    HostError::Type(format!("this is not a {what}"))
}

/// The id a command's result gives under `key`.
fn result_id(d: &Data, key: &str) -> Option<u64> {
    let Data::Object(o) = d else { return None };
    match o.iter().find(|(k, _)| k == key).map(|(_, v)| v) {
        Some(Data::Number(n)) => whole(*n),
        _ => None,
    }
}

// ---- Layer ----

/// The layer `d` names, which must still be a layer of its document.
pub(super) fn layer_of(h: &Host, d: &Data) -> Result<(Doc, NodeId)> {
    let Some(handle) = handle_of(d, "Layer") else { return Err(type_error("Layer")) };
    let [run, uid, high, low] = handle.as_slice() else { return Err(type_error("Layer")) };
    let doc = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_LAYER.into()))?;
    let id = id_of(*high, *low).ok_or_else(|| HostError::Error(STALE_LAYER.into()))?;
    let ok = with_doc(h, doc, |_, st| st.doc.node(id).is_some_and(crate::doc::Node::is_layer));
    if !ok {
        return Err(HostError::Error(STALE_LAYER.into()));
    }
    Ok((doc, id))
}

fn this_layer(h: &Host, a: &[Data]) -> Result<(Doc, NodeId)> {
    layer_of(h, arg(a, 0))
}

fn layer_ref(h: &Host, uid: u64, id: NodeId) -> Data {
    let [high, low] = id_parts(id);
    host_ref("Layer", &[float(h.run), float(uid), high, low])
}

/// Reads the layer `this` names with `f`.
fn read_layer(h: &Host, a: &[Data], f: impl FnOnce(&DocState, &crate::doc::Node) -> Answer) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    with_doc(h, doc, |_, st| match st.doc.node(id) {
        Some(n) => f(st, n),
        None => Err(HostError::Error(STALE_LAYER.into())),
    })
}

/// Runs `id` with `params` and the layer's id on its document, unless `unchanged` says the layer
/// already has what they set.
fn set_layer(h: &Host, a: &[Data], command_id: &str, unchanged: impl FnOnce(&crate::doc::Node) -> bool, params: Value) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    let same = with_doc(h, doc, |_, st| st.doc.node(id).is_some_and(unchanged));
    if !same {
        let mut p = params;
        if let Some(o) = p.as_object_mut() {
            o.insert("ids".into(), json!([id.0]));
        }
        on_doc(h, doc.uid, true, |h| command(h, command_id, &p))?;
    }
    Ok(Data::Undefined)
}

fn layer_flags(n: &crate::doc::Node) -> (bool, bool, Option<u8>, crate::doc::LayerColor) {
    match &n.kind {
        NodeKind::Layer { printable, preview, dim_images, color, .. } => (*printable, *preview, *dim_images, *color),
        _ => (true, true, None, crate::doc::LayerColor::Preset(0)),
    }
}

fn layer_name(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(text(n.name.clone().unwrap_or_default())))
}

fn layer_set_name(h: &Host, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("name must be a string".into())) };
    if name.trim().is_empty() {
        return Err(HostError::Range("a layer's name cannot be empty".into()));
    }
    if name.chars().count() > MAX_LAYER_NAME {
        return Err(HostError::Range(format!("a layer's name has at most {MAX_LAYER_NAME} characters")));
    }
    set_layer(h, a, "layer.setProps", |n| n.name.as_deref() == Some(name.as_str()), json!({"name": name}))
}

fn layer_visible(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(Data::Bool(n.visible)))
}

fn layer_set_visible(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "visible")?;
    set_layer(h, a, "layer.setProps", |n| n.visible == v, json!({"visible": v}))
}

fn layer_locked(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(Data::Bool(n.locked)))
}

fn layer_set_locked(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "locked")?;
    set_layer(h, a, "layer.setProps", |n| n.locked == v, json!({"locked": v}))
}

fn layer_printable(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(Data::Bool(layer_flags(n).0)))
}

fn layer_set_printable(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "printable")?;
    set_layer(h, a, "layer.setProps", |n| layer_flags(n).0 == v, json!({"printable": v}))
}

fn layer_preview(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(Data::Bool(layer_flags(n).1)))
}

fn layer_set_preview(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "preview")?;
    set_layer(h, a, "layer.setProps", |n| layer_flags(n).1 == v, json!({"preview": v}))
}

fn layer_dim(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(Data::Bool(layer_flags(n).2.is_some())))
}

fn layer_set_dim(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "dimPlacedImages")?;
    set_layer(h, a, "layer.setProps", |n| layer_flags(n).2.is_some() == v, json!({"dimImages": v}))
}

fn layer_opacity(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| Ok(num((f64::from(n.opacity) * 100.0 * 1e4).round() / 1e4)))
}

fn layer_set_opacity(h: &Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "opacity")?;
    if !(0.0..=100.0).contains(&v) {
        return Err(HostError::Range("opacity must be from 0 to 100".into()));
    }
    set_layer(h, a, "object.setProps", |n| (f64::from(n.opacity) * 100.0 - v).abs() < 1e-4, json!({"opacity": v}))
}

fn layer_color(h: &Host, a: &[Data]) -> Answer {
    read_layer(h, a, |_, n| {
        let [r, g, b] = layer_flags(n).3.rgb();
        let item = |k: &str, v: u8| (k.to_string(), num(f64::from(v)));
        Ok(Data::Object(vec![item("red", r), item("green", g), item("blue", b)]))
    })
}

/// `color = {red, green, blue}`: each a whole number from 0 to 255.
fn layer_set_color(h: &Host, a: &[Data]) -> Answer {
    let Data::Object(o) = arg(a, 1) else { return Err(HostError::Type("color must be an object with red, green and blue".into())) };
    let mut rgb = [0u8; 3];
    for (slot, key) in rgb.iter_mut().zip(["red", "green", "blue"]) {
        let v = o.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or(&Data::Undefined);
        let n = number(v, &format!("color.{key}"))?;
        if n.fract() != 0.0 || !(0.0..=255.0).contains(&n) {
            return Err(HostError::Range(format!("color.{key} must be a whole number from 0 to 255")));
        }
        *slot = u8::try_from(whole(n).unwrap_or(0)).unwrap_or(0);
    }
    let hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    set_layer(h, a, "layer.setProps", |n| layer_flags(n).3.rgb() == rgb, json!({"color": hex}))
}

fn layer_parent(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    Ok(with_doc(h, doc, |_, st| match st.doc.parent_of(id) {
        Some(p) => layer_ref(h, st.uid, p),
        None => doc_ref(h, st.uid),
    }))
}

fn layer_typename(h: &Host, a: &[Data]) -> Answer {
    this_layer(h, a)?;
    Ok(text("Layer"))
}

/// The layer's place among the layers beside it, from 1 at the bottom. Art beside a sublayer is
/// not counted.
fn layer_z_position(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    let at = with_doc(h, doc, |_, st| {
        let list = layer_list(&st.doc, st.doc.parent_of(id));
        list.iter().position(|l| *l == id).map(|i| list.len() - i)
    });
    at.map(|i| num(float(i as u64))).ok_or_else(|| HostError::Error(STALE_LAYER.into()))
}

fn layer_has_selected(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    Ok(Data::Bool(with_doc(h, doc, |_, st| st.selection.objects.iter().any(|o| st.doc.ancestry(*o).is_some_and(|a| a.contains(&id))))))
}

/// `hasSelectedArtwork = false` deselects the layer's art with `select.toggle`; `true` adds its
/// visible, unlocked art with `select.add`. The rest of the selection, ruler guides included, stays,
/// and no command runs when there is nothing to deselect or add.
fn layer_set_has_selected(h: &Host, a: &[Data]) -> Answer {
    let on = boolean(arg(a, 1), "hasSelectedArtwork")?;
    let (doc, id) = this_layer(h, a)?;
    let (ids, total) = with_doc(h, doc, |_, st| {
        let inside = |o: &NodeId| st.doc.ancestry(*o).is_some_and(|a| a.contains(&id));
        let selected = &st.selection.objects;
        if !on {
            return (selected.iter().copied().filter(inside).collect::<Vec<_>>(), 0);
        }
        let locks = st.doc.locked_artboards();
        let art = st.doc.node(id).map(|n| n.layer_art(true)).unwrap_or_default();
        let mut have: std::collections::HashSet<NodeId> = selected.iter().copied().collect();
        let mut ids = Vec::new();
        for o in art {
            if selected.len() + ids.len() > MAX_LISTED {
                break;
            }
            if !have.contains(&o) && crate::cmd::select::is_selectable(&st.doc, st.isolation, &locks, o) {
                have.insert(o);
                ids.push(o);
            }
        }
        let total = selected.len() + ids.len();
        (ids, total)
    });
    if total > MAX_LISTED {
        return Err(HostError::Range(format!("a selection takes at most {MAX_LISTED} objects")));
    }
    if ids.is_empty() {
        return Ok(Data::Undefined);
    }
    let command_id = if on { "select.add" } else { "select.toggle" };
    on_doc(h, doc.uid, false, |h| command(h, command_id, &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    Ok(Data::Undefined)
}

fn layer_is_isolated(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    Ok(Data::Bool(with_doc(h, doc, |_, st| st.isolation == Some(id))))
}

fn layer_remove(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    on_doc(h, doc.uid, true, |h| command(h, "layer.delete", &json!({"ids": [id.0]})))?;
    Ok(Data::Undefined)
}

/// The layers of `parent` (`None`: the top level), without its ruler guides and art.
fn art_count(d: &Document, parent: Option<NodeId>) -> usize {
    d.children(parent).map_or(0, |c| crate::doc::Node::guide_run_start(c))
}

/// Moves the layer `id` to `index` of `parent` with `node.move`, unless it is there already.
fn put_layer(h: &Host, doc: Doc, id: NodeId, parent: Option<NodeId>, index: usize) -> Result<()> {
    let now = with_doc(h, doc, |_, st| st.doc.position(id));
    if now.is_some_and(|(p, i, _)| p == parent && i == index) {
        return Ok(());
    }
    let parent = parent.map_or(Value::Null, |p| json!(p.0));
    on_doc(h, doc.uid, true, |h| command(h, "node.move", &json!({"id": id.0, "parent": parent, "index": index})))?;
    Ok(())
}

/// `zOrder(method)`: the layer goes to the top or the bottom of the layers beside it, or one step.
fn layer_z_order(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    let method = match arg(a, 1) {
        Data::String(s) => s.as_str(),
        _ => "",
    };
    let (parent, index) = with_doc(h, doc, |_, st| {
        let d = &st.doc;
        let (parent, i, _) = d.position(id).ok_or_else(|| HostError::Error(STALE_LAYER.into()))?;
        let floor = usize::from(parent.and_then(|p| d.node(p)).is_some_and(crate::doc::Node::clips));
        let count = art_count(d, parent);
        let last = count.saturating_sub(1);
        // One step passes the next layer beside it, and the art between them.
        let layer_at = |j: &usize| d.children(parent).and_then(|c| c.get(*j)).is_some_and(|n| n.is_layer());
        let to = match method {
            "ZOrderMethod.BRINGTOFRONT" => last,
            "ZOrderMethod.BRINGFORWARD" => (i + 1..count).find(layer_at).unwrap_or(i),
            "ZOrderMethod.SENDBACKWARD" => (floor.min(i)..i).rev().find(layer_at).unwrap_or(i),
            "ZOrderMethod.SENDTOBACK" => floor.min(i),
            _ => return Err(HostError::Type("zOrder needs a ZOrderMethod value".into())),
        };
        Ok((parent, to))
    })?;
    put_layer(h, doc, id, parent, index)?;
    Ok(Data::Undefined)
}

/// `move(relativeObject, insertionLocation?)`: the layer goes into another layer (on top of its
/// contents, or at the bottom with `PLACEATEND`), to the top level of a document, or directly
/// above or below another layer.
fn layer_move(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    let how = placement_of(arg(a, 2))?;
    let target = arg(a, 1);
    let parent = if let Some(handle) = handle_of(target, "Document") {
        let [run, uid] = handle.as_slice() else { return Err(type_error("Document")) };
        let to = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
        if to.uid != doc.uid {
            return Err(HostError::Error(super::CROSS_DOCUMENT.into()));
        }
        if matches!(how, Placement::Above | Placement::Below) {
            return Err(HostError::Error("a layer goes above or below another layer, not a document".into()));
        }
        None
    } else if handle_of(target, "Layer").is_some() {
        let (to, t) = layer_of(h, target)?;
        if to.uid != doc.uid {
            return Err(HostError::Error(super::CROSS_DOCUMENT.into()));
        }
        Some(t)
    } else if matches!(target, Data::Undefined) {
        return Err(HostError::Type("move needs a relative object".into()));
    } else {
        return Err(HostError::Type("a layer's relative object must be a layer or a document".into()));
    };
    let (into, index) = with_doc(h, doc, |_, st| {
        let d = &st.doc;
        if let Some(t) = parent
            && d.ancestry(t).is_some_and(|a| a.contains(&id))
            && !(t == id && matches!(how, Placement::Above | Placement::Below))
        {
            return Err(HostError::Error("a layer cannot go into itself".into()));
        }
        let (into, beside) = match (how, parent) {
            (Placement::Above | Placement::Below, Some(t)) => (d.parent_of(t), Some(t)),
            (_, p) => (p, None),
        };
        if into.is_some_and(|p| !editable(d, p)) {
            return Err(HostError::Error("the layer is hidden or locked".into()));
        }
        let from = d.position(id);
        let here = |p: Option<NodeId>| from.is_some_and(|(q, ..)| q == p);
        let count = art_count(d, into) - usize::from(here(into));
        let floor = usize::from(into.and_then(|p| d.node(p)).is_some_and(crate::doc::Node::clips) && count > 0);
        let index = match (how, beside) {
            (Placement::Above | Placement::Below, Some(b)) => {
                if b == id {
                    return Ok((into, from.map_or(0, |(_, i, _)| i)));
                }
                let (_, mut i, _) = d.position(b).ok_or_else(|| HostError::Error(STALE_LAYER.into()))?;
                if let Some((_, j, _)) = from
                    && here(into)
                    && j < i
                {
                    i -= 1;
                }
                if how == Placement::Above { i + 1 } else { i }
            }
            (Placement::Bottom, _) => 0,
            _ => count,
        };
        Ok((into, index.max(floor)))
    })?;
    put_layer(h, doc, id, into, index)?;
    Ok(arg(a, 0).clone())
}

// ---- Layers ----

/// The document and the layer (`None`: the top level) a `Layers` object lists.
fn layers_scope(h: &Host, d: &Data) -> Result<(Doc, Option<NodeId>)> {
    let Some(handle) = handle_of(d, "Layers") else { return Err(type_error("Layers")) };
    match handle.as_slice() {
        [run, uid] => Ok((find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?, None)),
        [run, uid, high, low] => {
            let (doc, id) = layer_of(h, &host_ref("Layer", &[*run, *uid, *high, *low]))?;
            Ok((doc, Some(id)))
        }
        _ => Err(type_error("Layers")),
    }
}

/// The layers of `parent` (`None`: the top level), top first.
fn layer_list(d: &Document, parent: Option<NodeId>) -> Vec<NodeId> {
    let list = match parent {
        None => Some(&d.layers),
        Some(p) => d.children(Some(p)),
    };
    list.into_iter().flatten().rev().filter(|n| n.is_layer()).map(|n| n.id).collect()
}

fn layers_of(h: &Host, a: &[Data]) -> Answer {
    let (doc, id) = this_layer(h, a)?;
    let [high, low] = id_parts(id);
    Ok(host_ref("Layers", &[float(h.run), float(doc.uid), high, low]))
}

fn layers_length(h: &Host, a: &[Data]) -> Answer {
    let (doc, parent) = layers_scope(h, arg(a, 0))?;
    Ok(num(float(with_doc(h, doc, |_, st| layer_list(&st.doc, parent).len()) as u64)))
}

fn layers_parent(h: &Host, a: &[Data]) -> Answer {
    let (doc, parent) = layers_scope(h, arg(a, 0))?;
    Ok(parent.map_or_else(|| doc_ref(h, doc.uid), |p| layer_ref(h, doc.uid, p)))
}

fn layers_typename(h: &Host, a: &[Data]) -> Answer {
    layers_scope(h, arg(a, 0))?;
    Ok(text("Layers"))
}

fn layers_at(h: &Host, a: &[Data]) -> Answer {
    let (doc, parent) = layers_scope(h, arg(a, 0))?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    Ok(with_doc(h, doc, |_, st| layer_list(&st.doc, parent).get(i).map_or(Data::Undefined, |id| layer_ref(h, st.uid, *id))))
}

fn layers_by_name(h: &Host, a: &[Data]) -> Answer {
    let (doc, parent) = layers_scope(h, arg(a, 0))?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        let ids = layer_list(&st.doc, parent);
        h.charge_call(ids.len() as u64)?;
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| layer_ref(h, st.uid, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn layers_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => layers_by_name(h, a),
        Data::Number(_) => match layers_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `layers.add()`: a new layer above every top-level layer, or a sublayer on top of the layer's
/// contents. → the new layer, which is the current one.
fn layers_add(h: &Host, a: &[Data]) -> Answer {
    let (doc, parent) = layers_scope(h, arg(a, 0))?;
    if let Some(p) = parent {
        let ok = with_doc(h, doc, |_, st| editable(&st.doc, p));
        if !ok {
            return Err(HostError::Error("the layer is hidden or locked".into()));
        }
    }
    let r = on_doc(h, doc.uid, true, |h| match parent {
        None => command(h, "layer.new", &json!({"top": true})),
        Some(p) => command(h, "layer.newSublayer", &json!({"parent": p.0})),
    })?;
    let id = result_id(&r, "id").ok_or_else(|| HostError::Error("the layer was not made".into()))?;
    Ok(layer_ref(h, doc.uid, NodeId(id)))
}

/// `layers.removeAll()`: deletes every layer of the collection in one command. A document keeps a
/// layer, so on the top-level layers it throws before anything is deleted.
fn layers_remove_all(h: &Host, a: &[Data]) -> Answer {
    let (doc, parent) = layers_scope(h, arg(a, 0))?;
    if parent.is_none() {
        return Err(HostError::Error("a document needs at least one layer".into()));
    }
    let ids = with_doc(h, doc, |_, st| layer_list(&st.doc, parent));
    if !ids.is_empty() {
        on_doc(h, doc.uid, true, |h| command(h, "layer.delete", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    }
    Ok(Data::Undefined)
}

// ---- GroupItems, CompoundPathItems and a container's PageItems ----

/// The document and the layer, group or compound path (`None`: the whole document) an art
/// collection of `class` lists.
fn art_scope(h: &Host, d: &Data, class: &str) -> Result<(Doc, Option<NodeId>)> {
    let Some(handle) = handle_of(d, class) else { return Err(type_error(class)) };
    match handle.as_slice() {
        [run, uid] => Ok((find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?, None)),
        [run, uid, high, low] => {
            let doc = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
            let id = id_of(*high, *low).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
            let holds = with_doc(h, doc, |_, st| {
                st.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. }))
            });
            if !holds {
                return Err(HostError::Error(STALE_ITEM.into()));
            }
            Ok((doc, Some(id)))
        }
        _ => Err(type_error(class)),
    }
}

/// A document's uid, the document as it was, the [`Kind`] and the container of an art collection,
/// and the ids it listed then.
pub(super) type ArtList = (u64, Arc<Document>, u8, Option<NodeId>, Rc<Vec<NodeId>>);

/// The objects of `kind` in `container` (directly), or in the whole document at any depth, in
/// paint order. The last list is kept while the document is unchanged, and each new one is charged
/// by the objects it looks at.
fn art_list(h: &Host, st: &DocState, kind: Kind, container: Option<NodeId>) -> Result<Rc<Vec<NodeId>>> {
    if let Some((uid, doc, k, c, ids)) = &*h.dom.art.borrow()
        && *uid == st.uid
        && Arc::ptr_eq(doc, &st.doc)
        && *k == kind as u8
        && *c == container
    {
        return Ok(ids.clone());
    }
    let ids: Vec<NodeId> = match container {
        None => {
            let all = page_items(h, st);
            h.charge_call(all.len() as u64)?;
            all.iter().copied().filter(|i| st.doc.node(*i).is_some_and(|n| kind.holds(n))).collect()
        }
        Some(c) => {
            let children: &[Arc<crate::doc::Node>] = st.doc.node(c).and_then(|n| n.children()).map_or(&[], Vec::as_slice);
            h.charge_call(children.len() as u64)?;
            children.iter().filter(|n| kind.holds(n)).map(|n| n.id).collect()
        }
    };
    let ids = Rc::new(ids);
    *h.dom.art.borrow_mut() = Some((st.uid, st.doc.clone(), kind as u8, container, ids.clone()));
    Ok(ids)
}

/// The `PageItems` of a layer, group or compound path: a [`super`] `PageItems` object whose handle
/// names the container.
pub(super) fn is_scoped(a: &[Data]) -> bool {
    host_ref_parts(arg(a, 0)).is_some_and(|(c, handle)| c == "PageItems" && handle.len() == 4)
}

fn collection(h: &Host, kind: Kind, container: Option<NodeId>, uid: u64) -> Data {
    match container {
        Some(c) => host_ref(kind.class(), &[float(h.run), float(uid), id_parts(c)[0], id_parts(c)[1]]),
        None => host_ref(kind.class(), &[float(h.run), float(uid)]),
    }
}

fn art_length(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), kind.class())?;
    let n = with_doc(h, doc, |_, st| art_list(h, st, kind, c).map(|l| l.len()))?;
    Ok(num(float(n as u64)))
}

fn art_parent(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), kind.class())?;
    Ok(with_doc(h, doc, |_, st| c.map_or_else(|| doc_ref(h, st.uid), |c| item_ref(h, st, c))))
}

fn art_at(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), kind.class())?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    with_doc(h, doc, |_, st| Ok(art_list(h, st, kind, c)?.get(i).map_or(Data::Undefined, |id| item_ref(h, st, *id))))
}

fn art_by_name(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), kind.class())?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        let ids = art_list(h, st, kind, c)?;
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| item_ref(h, st, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn art_index(h: &Host, a: &[Data], kind: Kind) -> Answer {
    match arg(a, 1) {
        Data::String(_) => art_by_name(h, a, kind),
        Data::Number(_) => match art_at(h, a, kind)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `removeAll()`: deletes every object the collection lists (those inside another listed one go
/// with it) in one command. A hidden or locked one, one in a hidden or locked layer or group, or
/// one on a locked artboard stops it before anything is deleted, as the document's
/// `pageItems.removeAll()` is stopped.
fn art_remove_all(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), kind.class())?;
    let (ids, refused, on_locked) = with_doc(h, doc, |_, st| {
        let all = art_list(h, st, kind, c)?;
        let d = &st.doc;
        let listed: std::collections::HashSet<NodeId> = all.iter().copied().collect();
        let ids: Vec<NodeId> =
            all.iter().copied().filter(|i| !d.ancestry(*i).is_some_and(|a| a.iter().any(|x| *x != *i && listed.contains(x)))).collect();
        let whole = |id: &NodeId| editable(d, *id) && d.node(*id).is_some_and(super::all_editable);
        let refused: Vec<u64> = ids.iter().filter(|i| !whole(i)).map(|i| i.0).take(20).collect();
        let locks = d.locked_artboards();
        let on_locked: Vec<u64> = ids.iter().filter(|i| locks.holds_id(**i)).map(|i| i.0).take(20).collect();
        Ok::<_, HostError>((ids, refused, on_locked))
    })?;
    if !refused.is_empty() {
        let list = refused.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("cannot delete hidden or locked objects: {list}")));
    }
    if !on_locked.is_empty() {
        let list = on_locked.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("{ON_LOCKED_ARTBOARD}: {list}")));
    }
    if !ids.is_empty() {
        on_doc(h, doc.uid, true, |h| command(h, "edit.clear", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    }
    Ok(Data::Undefined)
}

/// The scoped `PageItems` of a container, which `PageItems`' members hand over to when their
/// handle names one ([`is_scoped`]).
pub(super) fn scoped_length(h: &Host, a: &[Data]) -> Answer {
    art_length(h, a, Kind::Any)
}

pub(super) fn scoped_parent(h: &Host, a: &[Data]) -> Answer {
    art_parent(h, a, Kind::Any)
}

pub(super) fn scoped_at(h: &Host, a: &[Data]) -> Answer {
    art_at(h, a, Kind::Any)
}

pub(super) fn scoped_by_name(h: &Host, a: &[Data]) -> Answer {
    art_by_name(h, a, Kind::Any)
}

pub(super) fn scoped_remove_all(h: &Host, a: &[Data]) -> Answer {
    art_remove_all(h, a, Kind::Any)
}

fn groups_length(h: &Host, a: &[Data]) -> Answer {
    art_length(h, a, Kind::Group)
}
fn groups_parent(h: &Host, a: &[Data]) -> Answer {
    art_parent(h, a, Kind::Group)
}
fn groups_at(h: &Host, a: &[Data]) -> Answer {
    art_at(h, a, Kind::Group)
}
fn groups_by_name(h: &Host, a: &[Data]) -> Answer {
    art_by_name(h, a, Kind::Group)
}
fn groups_index(h: &Host, a: &[Data]) -> Answer {
    art_index(h, a, Kind::Group)
}
fn groups_remove_all(h: &Host, a: &[Data]) -> Answer {
    art_remove_all(h, a, Kind::Group)
}
fn compounds_length(h: &Host, a: &[Data]) -> Answer {
    art_length(h, a, Kind::Compound)
}
fn compounds_parent(h: &Host, a: &[Data]) -> Answer {
    art_parent(h, a, Kind::Compound)
}
fn compounds_at(h: &Host, a: &[Data]) -> Answer {
    art_at(h, a, Kind::Compound)
}
fn compounds_by_name(h: &Host, a: &[Data]) -> Answer {
    art_by_name(h, a, Kind::Compound)
}
fn compounds_index(h: &Host, a: &[Data]) -> Answer {
    art_index(h, a, Kind::Compound)
}
fn compounds_remove_all(h: &Host, a: &[Data]) -> Answer {
    art_remove_all(h, a, Kind::Compound)
}

fn art_typename(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), kind.class())?;
    // A compound path's `pathItems` are its `PageItems`: it holds only paths.
    let compound = c.is_some_and(|c| with_doc(h, doc, |_, st| st.doc.node(c).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))));
    Ok(text(if compound && kind == Kind::Any { "PathItems" } else { kind.class() }))
}

pub(super) fn scoped_typename(h: &Host, a: &[Data]) -> Answer {
    art_typename(h, a, Kind::Any)
}
fn groups_typename(h: &Host, a: &[Data]) -> Answer {
    art_typename(h, a, Kind::Group)
}
fn compounds_typename(h: &Host, a: &[Data]) -> Answer {
    art_typename(h, a, Kind::Compound)
}

/// `groupItems.add()`: an empty group on top of the layer's or group's contents (the document's:
/// the current layer's), with `object.newGroup`. → the group.
fn groups_add(h: &Host, a: &[Data]) -> Answer {
    let (doc, c) = art_scope(h, arg(a, 0), "GroupItems")?;
    if let Some(c) = c {
        let compound = with_doc(h, doc, |_, st| st.doc.node(c).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })));
        if compound {
            return Err(HostError::Error("a compound path holds only paths".into()));
        }
    }
    let params = c.map_or_else(|| json!({}), |c| json!({"parent": c.0}));
    let r = on_doc(h, doc.uid, true, |h| command(h, "object.newGroup", &params))?;
    let id = result_id(&r, "id").ok_or_else(|| HostError::Error("the group was not made".into()))?;
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == doc.uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
    Ok(item_ref(h, st, NodeId(id)))
}

fn compounds_add(_: &Host, _: &[Data]) -> Answer {
    Err(HostError::Error("CompoundPathItems.add cannot make an empty compound path: a compound path holds at least one path".into()))
}

// ---- The art collections of a document, a layer, a group and a compound path ----

/// The collection of `kind` the object `this` holds: a document's (`Document`), a layer's
/// (`Layer`) or a page item's.
fn held(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let this = arg(a, 0);
    if handle_of(this, "Document").is_some() {
        let doc = doc_of(h, this, "Document")?;
        return Ok(collection(h, kind, None, doc.uid));
    }
    if handle_of(this, "Layer").is_some() {
        let (doc, id) = layer_of(h, this)?;
        return Ok(collection(h, kind, Some(id), doc.uid));
    }
    let item = item_of(h, this)?;
    Ok(collection(h, kind, Some(item.id), item.doc.uid))
}

fn held_page_items(h: &Host, a: &[Data]) -> Answer {
    held(h, a, Kind::Any)
}

fn held_groups(h: &Host, a: &[Data]) -> Answer {
    held(h, a, Kind::Group)
}

fn held_compounds(h: &Host, a: &[Data]) -> Answer {
    held(h, a, Kind::Compound)
}

// ---- GroupItem and CompoundPathItem ----

fn this_item_of(h: &Host, a: &[Data], what: &str) -> Result<Item> {
    let item = item_of(h, arg(a, 0))?;
    let ok = with_doc(h, item.doc, |_, st| st.doc.node(item.id).and_then(typename) == Some(what));
    if !ok {
        return Err(type_error(what));
    }
    Ok(item)
}

/// The layer the object is in.
fn item_layer(h: &Host, a: &[Data]) -> Answer {
    let item = item_of(h, arg(a, 0))?;
    Ok(with_doc(h, item.doc, |_, st| st.doc.layer_containing(item.id).map_or(Data::Null, |l| layer_ref(h, st.uid, l))))
}

fn group_clipped(h: &Host, a: &[Data]) -> Answer {
    let item = this_item_of(h, a, "GroupItem")?;
    Ok(Data::Bool(with_doc(h, item.doc, |_, st| st.doc.node(item.id).is_some_and(crate::doc::Node::clips))))
}

/// `clipped = true`: the group's topmost object becomes its clipping path; `false` releases the
/// clipping mask. The group keeps its id either way.
fn group_set_clipped(h: &Host, a: &[Data]) -> Answer {
    let on = boolean(arg(a, 1), "clipped")?;
    let item = this_item_of(h, a, "GroupItem")?;
    let (now, ok) = with_doc(h, item.doc, |_, st| (st.doc.node(item.id).is_some_and(crate::doc::Node::clips), editable(&st.doc, item.id)));
    if now == on {
        return Ok(Data::Undefined);
    }
    if !ok {
        return Err(HostError::Error(format!("Cannot change GroupItem {}: it is hidden or locked", item.id.0)));
    }
    on_doc(h, item.doc.uid, true, |h| command(h, "layer.clippingMask.toggle", &json!({"id": item.id.0})))?;
    Ok(Data::Undefined)
}

/// The members `layers.rs` adds to `PageItem`'s.
pub(super) fn page_item_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("layer", get(l, item_layer))])
}

/// The members a page item class adds to `PageItem`'s.
pub(super) fn members(class: &str, l: &Link) -> Vec<(String, HostMember)> {
    match class {
        "GroupItem" => {
            let mut m = named(vec![
                ("clipped", property(l, group_clipped, group_set_clipped)),
                ("pageItems", get(l, held_page_items)),
                ("groupItems", get(l, held_groups)),
                ("compoundPathItems", get(l, held_compounds)),
            ]);
            m.extend(["pathItems", "textFrames", "symbolItems", "placedItems", "rasterItems"].map(|m| later(l, "GroupItem", m)));
            m
        }
        "CompoundPathItem" => named(vec![("pathItems", get(l, held_page_items))]),
        _ => Vec::new(),
    }
}

// ---- Document ----

fn doc_layers(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("Layers", &[float(h.run), float(doc.uid)]))
}

fn doc_active_layer(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(with_doc(h, doc, |_, st| st.current_layer().or_else(|| st.doc.default_layer()).map_or(Data::Null, |l| layer_ref(h, st.uid, l))))
}

/// `activeLayer = layer`: the layer becomes the current one, where new art goes.
fn doc_set_active_layer(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    let (to, id) = layer_of(h, arg(a, 1))?;
    if to.uid != doc.uid {
        return Err(HostError::Error("the layer is in another document".into()));
    }
    let now = with_doc(h, doc, |_, st| st.current_layer());
    if now != Some(id) {
        on_doc(h, doc.uid, true, |h| command(h, "layer.setCurrent", &json!({"id": id.0})))?;
    }
    Ok(Data::Undefined)
}

fn doc_artboards(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("Artboards", &[float(h.run), float(doc.uid)]))
}

/// The members `layers.rs` adds to `Document`'s.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![
        ("layers", get(l, doc_layers)),
        ("activeLayer", property(l, doc_active_layer, doc_set_active_layer)),
        ("groupItems", get(l, held_groups)),
        ("compoundPathItems", get(l, held_compounds)),
        ("artboards", get(l, doc_artboards)),
    ])
}

// ---- Artboard and Artboards ----

/// The document of an artboard.
#[derive(Clone, Copy)]
struct Board {
    doc: Doc,
}

fn board_index(st: &DocState, id: u32) -> Option<usize> {
    st.doc.artboards.iter().position(|a| a.id == id)
}

fn board_of(h: &Host, d: &Data) -> Result<(Board, usize)> {
    let Some(handle) = handle_of(d, "Artboard") else { return Err(type_error("Artboard")) };
    let [run, uid, id] = handle.as_slice() else { return Err(type_error("Artboard")) };
    let doc = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_ARTBOARD.into()))?;
    let id = whole(*id).and_then(|i| u32::try_from(i).ok()).ok_or_else(|| HostError::Error(STALE_ARTBOARD.into()))?;
    let index = with_doc(h, doc, |_, st| board_index(st, id)).ok_or_else(|| HostError::Error(STALE_ARTBOARD.into()))?;
    Ok((Board { doc }, index))
}

fn board_ref(h: &Host, uid: u64, id: u32) -> Data {
    host_ref("Artboard", &[float(h.run), float(uid), f64::from(id)])
}

fn this_board(h: &Host, a: &[Data]) -> Result<(Board, usize)> {
    board_of(h, arg(a, 0))
}

/// Reads the artboard `this` names with `f`.
fn read_board(h: &Host, a: &[Data], f: impl FnOnce(&DocState, &crate::doc::Artboard, usize) -> Answer) -> Answer {
    let (b, i) = this_board(h, a)?;
    with_doc(h, b.doc, |_, st| match st.doc.artboards.get(i) {
        Some(ab) => f(st, ab, i),
        None => Err(HostError::Error(STALE_ARTBOARD.into())),
    })
}

/// Runs `artboard.setProps` with `props` for the artboard, unless `unchanged` says it has them.
fn set_board(h: &Host, a: &[Data], unchanged: impl FnOnce(&crate::doc::Artboard) -> bool, props: Value) -> Answer {
    let (b, i) = this_board(h, a)?;
    let same = with_doc(h, b.doc, |_, st| st.doc.artboards.get(i).is_some_and(unchanged));
    if !same {
        let mut p = props;
        if let Some(o) = p.as_object_mut() {
            o.insert("index".into(), json!(i));
        }
        on_doc(h, b.doc.uid, true, |h| command(h, "artboard.setProps", &p))?;
    }
    Ok(Data::Undefined)
}

fn board_rect(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |st, ab, _| Ok(frame(st)?.bounds(h.dom.coords.get(), ab.rect)))
}

/// A rectangle `[left, top, right, bottom]` a script gave, in document coordinates, checked: finite,
/// inside the canvas, and from 1 to [`super::MAX_SIZE`] points on each side, the sides the artboard
/// commands keep.
fn rect_arg(h: &Host, st: &DocState, d: &Data, what: &str) -> Result<vectorcraft_geom::Rect> {
    let Data::Array(v) = d else { return Err(HostError::Type(format!("{what} must be an array of four numbers"))) };
    let [l, t, r, b] = v.as_slice() else { return Err(HostError::Type(format!("{what} must be an array of four numbers"))) };
    let n = |d: &Data| super::coordinate(d, what);
    let (l, t, r, b) = (n(l)?, n(t)?, n(r)?, n(b)?);
    let f = frame(st)?;
    let c = h.dom.coords.get();
    let p = f.to_document(c, [l, t]);
    let q = f.to_document(c, [r, b]);
    let rect = vectorcraft_geom::Rect::new(p.x.min(q.x), p.y.min(q.y), p.x.max(q.x), p.y.max(q.y));
    if q.x <= p.x || q.y <= p.y {
        return Err(HostError::Range(format!("{what} must have its right side right of its left side and its bottom below its top")));
    }
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return Err(HostError::Range(format!("{what} must be at least 1 point wide and tall")));
    }
    if rect.width() > super::MAX_SIZE || rect.height() > super::MAX_SIZE {
        return Err(HostError::Range(format!("{what} must be at most 3999856 points wide and tall")));
    }
    if [rect.x0, rect.y0, rect.x1, rect.y1].iter().any(|v| v.abs() > crate::MAX_COORD) {
        return Err(HostError::Range(format!("{what} is outside the canvas")));
    }
    Ok(rect)
}

fn board_set_rect(h: &Host, a: &[Data]) -> Answer {
    let (b, _) = this_board(h, a)?;
    let r = with_doc(h, b.doc, |_, st| rect_arg(h, st, arg(a, 1), "artboardRect"))?;
    let close = |x: f64, y: f64| (x - y).abs() < 1e-9;
    set_board(
        h,
        a,
        |ab| close(ab.rect.x0, r.x0) && close(ab.rect.y0, r.y0) && close(ab.rect.x1, r.x1) && close(ab.rect.y1, r.y1),
        json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()}),
    )
}

fn board_name(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |_, ab, _| Ok(text(ab.name.clone())))
}

/// A new artboard name: not empty, at most [`MAX_TEXT_BYTES`] bytes, and no other artboard's
/// (`except`: the artboard being renamed). → the name, trimmed as `artboard.rename` trims it.
fn board_name_arg(st: &DocState, d: &Data, except: Option<usize>) -> Result<String> {
    let Data::String(name) = d else { return Err(HostError::Type("name must be a string".into())) };
    if name.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the name is too long".into()));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(HostError::Range("an artboard's name cannot be empty".into()));
    }
    if st.doc.artboards.iter().enumerate().any(|(i, ab)| Some(i) != except && ab.name == name) {
        return Err(HostError::Error(format!("another artboard is named {name}")));
    }
    Ok(name.to_string())
}

fn board_set_name(h: &Host, a: &[Data]) -> Answer {
    let (b, i) = this_board(h, a)?;
    let name = with_doc(h, b.doc, |_, st| board_name_arg(st, arg(a, 1), Some(i)))?;
    set_board(h, a, |ab| ab.name == name, json!({"name": name}))
}

fn board_parent(h: &Host, a: &[Data]) -> Answer {
    let (b, _) = this_board(h, a)?;
    Ok(doc_ref(h, b.doc.uid))
}

fn board_typename(h: &Host, a: &[Data]) -> Answer {
    this_board(h, a)?;
    Ok(text("Artboard"))
}

/// `rulerOrigin`: where the rulers measure from with this artboard active, from its top-left corner
/// with Y down.
fn board_ruler_origin(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |st, ab, i| {
        let o = st.doc.ruler_origin_for(i);
        Ok(super::pair([o.x - ab.rect.x0, o.y - ab.rect.y0]))
    })
}

fn board_set_ruler_origin(h: &Host, a: &[Data]) -> Answer {
    let p = super::point(arg(a, 1), "rulerOrigin")?;
    let (b, i) = this_board(h, a)?;
    let (to, now) = with_doc(h, b.doc, |_, st| {
        let ab = &st.doc.artboards[i];
        (vectorcraft_geom::Point::new(ab.rect.x0 + p[0], ab.rect.y0 + p[1]), st.doc.ruler_origin_for(i))
    });
    if to.distance(now) > 1e-9 {
        on_doc(h, b.doc.uid, false, |h| command(h, "document.setRulerOrigin", &json!({"x": to.x, "y": to.y, "artboard": i})))?;
    }
    Ok(Data::Undefined)
}

fn board_par(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |_, ab, _| Ok(num(ab.video_ruler_pixel_aspect_ratio)))
}

fn board_set_par(h: &Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "rulerPAR")?;
    if !crate::doc::VIDEO_PIXEL_ASPECT_RANGE.contains(&v) {
        return Err(HostError::Range("rulerPAR must be from 0.1 to 10".into()));
    }
    set_board(h, a, |ab| (ab.video_ruler_pixel_aspect_ratio - v).abs() < 1e-12, json!({"videoRulerPixelAspectRatio": v}))
}

fn board_center(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |_, ab, _| Ok(Data::Bool(ab.show_center_mark)))
}

fn board_set_center(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "showCenter")?;
    set_board(h, a, |ab| ab.show_center_mark == v, json!({"showCenterMark": v}))
}

fn board_cross_hairs(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |_, ab, _| Ok(Data::Bool(ab.show_cross_hairs)))
}

fn board_set_cross_hairs(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "showCrossHairs")?;
    set_board(h, a, |ab| ab.show_cross_hairs == v, json!({"showCrossHairs": v}))
}

fn board_safe_areas(h: &Host, a: &[Data]) -> Answer {
    read_board(h, a, |_, ab, _| Ok(Data::Bool(ab.show_video_safe_areas)))
}

fn board_set_safe_areas(h: &Host, a: &[Data]) -> Answer {
    let v = boolean(arg(a, 1), "showSafeAreas")?;
    set_board(h, a, |ab| ab.show_video_safe_areas == v, json!({"showVideoSafeAreas": v}))
}

fn board_remove(h: &Host, a: &[Data]) -> Answer {
    let (b, i) = this_board(h, a)?;
    on_doc(h, b.doc.uid, true, |h| command(h, "artboard.delete", &json!({"index": i})))?;
    Ok(Data::Undefined)
}

fn boards_doc(h: &Host, a: &[Data]) -> Result<Doc> {
    doc_of(h, arg(a, 0), "Artboards")
}

fn boards_length(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    Ok(num(float(with_doc(h, doc, |_, st| st.doc.artboards.len()) as u64)))
}

fn boards_parent(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    Ok(doc_ref(h, doc.uid))
}

fn boards_typename(h: &Host, a: &[Data]) -> Answer {
    boards_doc(h, a)?;
    Ok(text("Artboards"))
}

/// An artboard index argument: a whole number below `len` (`len + 1` with `insert`).
fn board_index_arg(d: &Data, len: usize, what: &str) -> Result<usize> {
    let n = number(d, what)?;
    whole(n)
        .and_then(|i| usize::try_from(i).ok())
        .filter(|i| *i < len)
        .ok_or_else(|| HostError::Range(format!("{what} must be a whole number from 0 to {}", len.saturating_sub(1))))
}

fn boards_at(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    Ok(with_doc(h, doc, |_, st| st.doc.artboards.get(i).map_or(Data::Undefined, |ab| board_ref(h, st.uid, ab.id))))
}

fn boards_by_name(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        h.charge_call(st.doc.artboards.len() as u64)?;
        let ab = st.doc.artboards.iter().find(|ab| ab.name == *name);
        ab.map(|ab| board_ref(h, st.uid, ab.id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn boards_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => boards_by_name(h, a),
        Data::Number(_) => match boards_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// A new artboard at `rect` with `artboard.new`, moved to `at` (`None`: it stays last) with
/// `artboard.reorder`; when either fails, the document is as it was. → the artboard.
fn add_board(h: &Host, a: &[Data], at: Option<&Data>) -> Answer {
    let doc = boards_doc(h, a)?;
    let (r, len, name) = with_doc(h, doc, |_, st| {
        let boards = &st.doc.artboards;
        h.charge_call(boards.len() as u64)?;
        // The first "Artboard N" from one past the count that no artboard has, so names stay unique.
        let name = (boards.len() + 1..).map(|n| format!("Artboard {n}")).find(|n| !boards.iter().any(|ab| ab.name == *n)).unwrap_or_default();
        Ok::<_, HostError>((rect_arg(h, st, arg(a, 1), "artboardRect")?, boards.len(), name))
    })?;
    let to = at.map(|d| board_index_arg(d, len + 1, "index")).transpose()?;
    let max = whole(super::MAX_ARTBOARDS).and_then(|m| usize::try_from(m).ok()).unwrap_or(0);
    if len >= max {
        return Err(HostError::Range(format!("a document has at most {max} artboards")));
    }
    let id = super::atomically(h, |h| {
        on_doc(h, doc.uid, true, |h| {
            let made = command(h, "artboard.new", &json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height(), "name": name}))?;
            let index = result_id(&made, "index")
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| HostError::Error("the artboard was not made".into()))?;
            if let Some(to) = to
                && to != index
            {
                command(h, "artboard.reorder", &json!({"index": index, "to": to}))?;
            }
            let s = h.session.borrow();
            let st = s.documents().iter().find(|d| d.uid == doc.uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
            st.doc.artboards.get(to.unwrap_or(index)).map(|ab| ab.id).ok_or_else(|| HostError::Error("the artboard was not made".into()))
        })
    })?;
    Ok(board_ref(h, doc.uid, id))
}

/// `artboards.add(artboardRect)`: a new artboard, last in the list.
fn boards_add(h: &Host, a: &[Data]) -> Answer {
    add_board(h, a, None)
}

/// `artboards.insert(artboardRect, index)`: a new artboard at `index`.
fn boards_insert(h: &Host, a: &[Data]) -> Answer {
    if matches!(arg(a, 2), Data::Undefined) {
        return Err(HostError::Type("insert needs an index".into()));
    }
    add_board(h, a, Some(arg(a, 2)))
}

fn boards_remove(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    let len = with_doc(h, doc, |_, st| st.doc.artboards.len());
    let i = board_index_arg(arg(a, 1), len, "index")?;
    on_doc(h, doc.uid, true, |h| command(h, "artboard.delete", &json!({"index": i})))?;
    Ok(Data::Undefined)
}

fn boards_active(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    Ok(num(float(with_doc(h, doc, |_, st| active_artboard(st)) as u64)))
}

/// `setActiveArtboardIndex(index)`: the artboard becomes the view's active one. Nothing in the
/// document changes, and no command runs.
fn boards_set_active(h: &Host, a: &[Data]) -> Answer {
    let doc = boards_doc(h, a)?;
    let (len, now) = with_doc(h, doc, |_, st| (st.doc.artboards.len(), active_artboard(st)));
    let i = board_index_arg(arg(a, 1), len, "index")?;
    if i != now {
        let mut s = h.session.borrow_mut();
        let st = s.document_mut(doc.uid).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into()))?;
        st.active_artboard = st.doc.artboards.get(i).map(|ab| ab.id);
        drop(s);
        let mut moved = h.dom.active_artboards.borrow_mut();
        if !moved.contains(&doc.uid) {
            moved.push(doc.uid);
        }
    }
    Ok(Data::Undefined)
}

/// The documents whose active artboard the run changed, as `script.run`'s report gives them:
/// `[{document, artboard}]`, each an index, with the active artboard each document has at the end
/// of the run, or `None` when there are none.
pub(crate) fn active_artboards(moved: &[u64], docs: &[DocState]) -> Option<Value> {
    let list: Vec<Value> = moved
        .iter()
        .filter_map(|uid| {
            let document = docs.iter().position(|d| d.uid == *uid)?;
            Some(json!({"document": document, "artboard": active_artboard(&docs[document])}))
        })
        .collect();
    (!list.is_empty()).then_some(Value::Array(list))
}

/// The classes of layers, art collections and artboards.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let class = super::class;
    let mut layer = named(vec![
        ("typename", get(l, layer_typename)),
        ("name", property(l, layer_name, layer_set_name)),
        ("visible", property(l, layer_visible, layer_set_visible)),
        ("locked", property(l, layer_locked, layer_set_locked)),
        ("printable", property(l, layer_printable, layer_set_printable)),
        ("preview", property(l, layer_preview, layer_set_preview)),
        ("dimPlacedImages", property(l, layer_dim, layer_set_dim)),
        ("opacity", property(l, layer_opacity, layer_set_opacity)),
        ("color", property(l, layer_color, layer_set_color)),
        ("parent", get(l, layer_parent)),
        ("layers", get(l, layers_of)),
        ("pageItems", get(l, held_page_items)),
        ("groupItems", get(l, held_groups)),
        ("compoundPathItems", get(l, held_compounds)),
        ("zOrderPosition", get(l, layer_z_position)),
        ("hasSelectedArtwork", property(l, layer_has_selected, layer_set_has_selected)),
        ("isIsolated", get(l, layer_is_isolated)),
        ("move", method(l, layer_move)),
        ("remove", method(l, layer_remove)),
        ("zOrder", method(l, layer_z_order)),
    ]);
    layer.extend(
        ["pathItems", "textFrames", "symbolItems", "placedItems", "rasterItems", "blendingMode", "artworkKnockout", "sliced"]
            .map(|m| later(l, "Layer", m)),
    );
    vec![
        class("Layer", None, layer, None),
        class(
            "Layers",
            None,
            named(vec![
                ("typename", get(l, layers_typename)),
                ("length", get(l, layers_length)),
                ("parent", get(l, layers_parent)),
                ("index", method(l, layers_index)),
                ("getByName", method(l, layers_by_name)),
                ("add", method(l, layers_add)),
                ("removeAll", method(l, layers_remove_all)),
            ]),
            Some(super::function(l, layers_at)),
        ),
        class(
            "GroupItems",
            None,
            {
                let mut m = named(vec![
                    ("typename", get(l, groups_typename)),
                    ("length", get(l, groups_length)),
                    ("parent", get(l, groups_parent)),
                    ("index", method(l, groups_index)),
                    ("getByName", method(l, groups_by_name)),
                    ("add", method(l, groups_add)),
                    ("removeAll", method(l, groups_remove_all)),
                ]);
                m.push(later(l, "GroupItems", "createFromFile"));
                m
            },
            Some(super::function(l, groups_at)),
        ),
        class(
            "CompoundPathItems",
            None,
            named(vec![
                ("typename", get(l, compounds_typename)),
                ("length", get(l, compounds_length)),
                ("parent", get(l, compounds_parent)),
                ("index", method(l, compounds_index)),
                ("getByName", method(l, compounds_by_name)),
                ("add", method(l, compounds_add)),
                ("removeAll", method(l, compounds_remove_all)),
            ]),
            Some(super::function(l, compounds_at)),
        ),
        class(
            "Artboard",
            None,
            named(vec![
                ("typename", get(l, board_typename)),
                ("artboardRect", property(l, board_rect, board_set_rect)),
                ("name", property(l, board_name, board_set_name)),
                ("parent", get(l, board_parent)),
                ("rulerOrigin", property(l, board_ruler_origin, board_set_ruler_origin)),
                ("rulerPAR", property(l, board_par, board_set_par)),
                ("showCenter", property(l, board_center, board_set_center)),
                ("showCrossHairs", property(l, board_cross_hairs, board_set_cross_hairs)),
                ("showSafeAreas", property(l, board_safe_areas, board_set_safe_areas)),
                ("remove", method(l, board_remove)),
            ]),
            None,
        ),
        class(
            "Artboards",
            None,
            named(vec![
                ("typename", get(l, boards_typename)),
                ("length", get(l, boards_length)),
                ("parent", get(l, boards_parent)),
                ("index", method(l, boards_index)),
                ("getByName", method(l, boards_by_name)),
                ("add", method(l, boards_add)),
                ("insert", method(l, boards_insert)),
                ("remove", method(l, boards_remove)),
                ("getActiveArtboardIndex", method(l, boards_active)),
                ("setActiveArtboardIndex", method(l, boards_set_active)),
            ]),
            Some(super::function(l, boards_at)),
        ),
    ]
}
