//! Opacity-mask editing mode (click the mask thumbnail in the Transparency panel).
//!
//! The mask art moves onto a temporary isolated layer where every tool and command can edit it.
//! After each edit [`sync`] writes the layer's art back into the object's mask, so the masked
//! object updates live; the renderer doesn't paint the editing layer (the mask is seen through its
//! effect, as in the reference app). Leaving the mode removes the layer, and saving or exporting
//! never writes it ([`Document::without_edit_modes`]). Meanwhile the Transparency panel's commands
//! act on the masked object (see `opacitymask::transparency_targets`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, MaskEdit, Node, NodeId, NodeKind, Selection};

use super::*;

/// Name of the temporary mask-editing layer.
pub const MASK_EDIT_LAYER: &str = "Opacity Mask Editing";

/// Name of the group that holds several mask objects (or none) as one mask art node.
const MASK_GROUP: &str = "Opacity Mask";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "transparency.editOpacityMask",
            "Edit Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{id?} edit the mask art of `id` (default: the first selected object with a mask) in place, as clicking the mask thumbnail in the Transparency panel does → {layer}",
            has_doc,
            enter
        ),
        cmd!(
            "transparency.stopEditingOpacityMask",
            "Stop Editing Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{} leave mask editing (the object thumbnail) → {id}",
            has_doc,
            leave
        ),
        cmd!(
            "transparency.viewOpacityMask",
            "View Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{on?: bool (default: toggle), id?} show only the opacity mask of `id` (default: the object whose mask is edited, else the first selected object with a mask) on the canvas, as greyscale coverage, and edit it, as Alt-clicking the mask thumbnail does; off shows the artwork again (still editing) → {on, id}",
            has_doc,
            view_mask
        ),
    ]
}

/// Mask art made of `children`: a plain group, so the mask keeps one art node. Editing spreads it
/// back out onto the editing layer.
pub(crate) fn mask_group(children: Vec<Arc<Node>>) -> Node {
    let mut g = Node::group(NodeId(u64::MAX - 1), children);
    g.name = Some(MASK_GROUP.into());
    g
}

/// A group [`mask_group`] made (named so, with nothing of its own that its members would lose).
fn is_mask_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(MASK_GROUP)
        && n.has_default_transparency()
        && n.appearance.items.is_empty()
        && n.appearance.effects.is_empty()
}

/// The objects mask art is made of: the members of a [`mask_group`], else the art itself.
pub(crate) fn mask_parts(art: &Arc<Node>) -> Vec<Arc<Node>> {
    match art.children() {
        Some(ch) if is_mask_group(art) => ch.clone(),
        _ => vec![art.clone()],
    }
}

fn enter(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.editOpacityMask";
    let st = s.doc()?;
    if st.doc.mask_edit.is_some() {
        return Err(bad(C, "already editing an opacity mask"));
    }
    let id = id_param(p, "id").or_else(|| first_masked(st)).ok_or_else(|| bad(C, "select an object with an opacity mask"))?;
    let layer = start(s, id)?;
    Ok(json!({ "layer": layer.0 }))
}

/// The first selected object with an opacity mask.
fn first_masked(st: &crate::DocState) -> Option<NodeId> {
    st.selection.subjects().iter().copied().find(|i| st.doc.node(*i).is_some_and(|n| n.extras().mask.is_some()))
}

/// Enter mask editing for `id` (one undo step) and isolate the editing layer. Returns the layer.
fn start(s: &mut Session, id: NodeId) -> Result<NodeId> {
    let layer = s.edit("Edit Opacity Mask", |d, sel| begin(d, sel, id))?;
    show_editing(s, Some(layer))?;
    Ok(layer)
}

fn view_mask(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.viewOpacityMask";
    let st = s.doc()?;
    let editing = st.doc.mask_edit.map(|m| m.object);
    if !p.get("on").and_then(Value::as_bool).unwrap_or(st.shown_mask().is_none()) {
        let st = s.doc_mut()?;
        let id = st.mask_view.take();
        st.revision += 1;
        return Ok(json!({ "on": false, "id": id.map(|i| i.0) }));
    }
    let id = id_param(p, "id").or(editing).or_else(|| first_masked(st)).ok_or_else(|| bad(C, "select an object with an opacity mask"))?;
    match editing {
        Some(e) if e != id => return Err(bad(C, "already editing another object's opacity mask")),
        Some(_) => {}
        None => {
            start(s, id)?;
        }
    }
    let st = s.doc_mut()?;
    st.mask_view = Some(id);
    st.revision += 1;
    Ok(json!({ "on": true, "id": id.0 }))
}

/// Enter mask editing for `id` inside an edit: copy its mask art onto a new editing layer and
/// select it. Returns the layer.
pub(crate) fn begin(d: &mut Document, sel: &mut Selection, id: NodeId) -> Result<NodeId> {
    let art = d
        .node(id)
        .and_then(|n| n.extras().mask.as_ref())
        .map(|m| m.art.clone())
        .ok_or_else(|| EngineError::Other("the object has no opacity mask".into()))?;
    let layer = d.add_layer(Some(MASK_EDIT_LAYER));
    let parts = mask_parts(&art);
    let mut ids = Vec::with_capacity(parts.len());
    for part in &parts {
        // Fresh ids: copies of a masked object share the ids inside their mask art.
        let copy = d.reid(part);
        ids.push(d.insert(Some(layer), usize::MAX, copy)?);
    }
    // Copied type whose layout caches `reid` dropped has them again.
    super::typecmd::refresh_bounds_of(d, &ids);
    d.mask_edit = Some(MaskEdit { object: id, layer });
    sel.set(ids);
    Ok(layer)
}

fn leave(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(me) = s.doc()?.doc.mask_edit else { return Err(bad("transparency.stopEditingOpacityMask", "not editing an opacity mask")) };
    s.edit("Stop Editing Opacity Mask", |d, sel| {
        finish(d, sel);
        Ok(())
    })?;
    show_editing(s, None)?;
    Ok(json!({ "id": me.object.0 }))
}

/// Leave mask editing inside an edit (no-op when not editing): write the art back into the mask,
/// drop the editing layer and select the masked object.
pub(crate) fn finish(d: &mut Document, sel: &mut Selection) {
    let Some(me) = d.mask_edit else { return };
    sync(d);
    d.drop_edit_modes();
    if d.node(me.object).is_some() {
        sel.set_target(d, me.object);
    } else {
        sel.clear();
    }
}

/// Isolate the editing layer and draw into it (`Some`), or return to the document (`None`, which
/// also ends View Opacity Mask).
pub(crate) fn show_editing(s: &mut Session, layer: Option<NodeId>) -> Result<()> {
    let st = s.doc_mut()?;
    st.isolation = layer;
    st.active_layer = layer.or_else(|| st.doc.default_layer());
    if layer.is_none() {
        st.mask_view = None;
    }
    st.revision += 1;
    Ok(())
}

/// After undo or redo, isolation follows the restored document into or out of mask editing
/// (`was` is the editing layer before, if any), so it never names a layer that is gone (whose id
/// a later object could reuse). Out of mask editing, View Opacity Mask ends.
pub(crate) fn follow_history(st: &mut crate::DocState, was: Option<NodeId>) {
    match st.doc.mask_edit {
        Some(me) => {
            st.isolation = Some(me.layer);
            st.active_layer = Some(me.layer);
        }
        None => {
            st.mask_view = None;
            if was.is_some() && st.isolation == was {
                st.isolation = None;
                st.active_layer = st.doc.default_layer();
            }
        }
    }
}

/// Write the editing layer's art into the object's mask (called after every edit). Leaves the
/// mode when the object or the layer is gone (e.g. deleted).
pub(crate) fn sync(d: &mut Document) {
    let Some(me) = d.mask_edit else { return };
    let art: Option<Vec<Arc<Node>>> = d.node(me.layer).and_then(|l| l.children().cloned());
    let (Some(art), true) = (art, d.node(me.object).is_some_and(|n| n.extras().mask.is_some())) else {
        d.mask_edit = None;
        let _ = d.remove(me.layer);
        return;
    };
    let new_art = match art.as_slice() {
        [one] => one.clone(),
        // Several objects (or none): a group, so the mask keeps one art node.
        many => Arc::new(mask_group(many.to_vec())),
    };
    if let Some(m) = d.node(me.object).and_then(|n| n.extras().mask.as_ref())
        && Arc::ptr_eq(&m.art, &new_art)
    {
        return;
    }
    if let Some(m) = d.node_mut(me.object).and_then(|n| n.extras_if_any().and_then(|e| e.mask.as_mut())) {
        m.art = new_art;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{NodeId, Session};

    #[test]
    fn edit_mask_art_in_place() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
        let obj = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap()["id"].as_u64().unwrap();
        let m = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"color": "#ffffff", "ids": [m]})).unwrap();
        s.execute("select.set", &json!({"ids": [obj, m]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        let layer = s.execute("transparency.editOpacityMask", &json!({})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(s.doc().unwrap().isolation, Some(NodeId(layer)));
        // Edit the mask art with ordinary commands: widen it; the object's mask follows live.
        s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
        let mask_bounds =
            |s: &Session| s.doc().unwrap().doc.node(NodeId(obj)).unwrap().extras().mask.as_ref().unwrap().art.geometric_bounds().unwrap();
        assert!((mask_bounds(&s).x0 - 50.0).abs() < 1e-6);
        // Drawing while editing adds to the mask.
        s.execute("shape.ellipse", &json!({"x": 180, "y": 150, "width": 40, "height": 40})).unwrap();
        assert!(mask_bounds(&s).x1 > 185.0);
        // Rendered: the object shows through the moved mask; the mask art itself isn't painted
        // (the ellipse pokes out of the object at x = 210).
        let doc = s.doc().unwrap().doc.clone();
        let img = vectorcraft_render::Renderer::new().render(&doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert!(img.pixel(100, 100)[3] > 200);
        assert_eq!(img.pixel(25, 100)[3], 0);
        assert_eq!(img.pixel(212, 170)[3], 0);
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        let st = s.doc().unwrap();
        assert!(st.doc.mask_edit.is_none() && st.isolation.is_none());
        assert!(!st.doc.layers.iter().any(|l| l.name.as_deref() == Some(super::MASK_EDIT_LAYER)));
        assert_eq!(st.selection.objects, vec![NodeId(obj)]);
        // Two mask objects went back as one group; editing again spreads them out on the layer.
        let layer = s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(s.doc().unwrap().doc.node(NodeId(layer)).unwrap().children().unwrap().len(), 2);
        assert_eq!(s.doc().unwrap().selection.len(), 2);
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        // Undo returns to editing mode's last state, then to before.
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.mask_edit.is_some());
    }
}
