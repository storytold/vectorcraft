//! Opacity masks (Transparency panel): make, release, enable/disable, link/unlink, clip, invert,
//! and the panel's state (`transparency.info`).
//!
//! The mask art is stored on the masked object ([`vectorcraft_doc::OpacityMask`]), outside the
//! layer tree, so it is never hit-tested or selected. Its luminance sets the object's opacity.
//! Every command here (and `transparency.set`) acts on [`transparency_targets`]: explicit ids, else
//! the object whose mask is being edited, else the selection.

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Document, Knockout, Node, NodeId, NodeKind, OpacityMask, Selection};

use super::maskedit::{begin, finish, mask_group, mask_parts, show_editing};
use super::pathops::{Beside, separate};
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, has_doc, ids_param, ok, targets};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "transparency.makeOpacityMask",
            "Make Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{ids?, clip?, invert?} the topmost of the selected objects (or of `ids`) becomes the mask of the others (grouped if several). One object gets an empty mask and enters mask editing: draw the mask, then transparency.stopEditingOpacityMask; one target (an object or a layer) is refused in isolation mode and in pattern editing mode → {id, editing}",
            can_make,
            make
        ),
        cmd!(
            "transparency.releaseOpacityMask",
            "Release Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{id?|ids?} put the mask art back above each masked object (default: the selection, or the object whose mask is being edited, which leaves mask editing)",
            has_doc,
            release
        ),
        cmd!(
            "transparency.disableOpacityMask",
            "Disable Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{id?|ids?} keep the mask but stop applying it",
            has_doc,
            |s, p| set_flags(s, p, "Disable Opacity Mask", &json!({ "disabled": true }))
        ),
        cmd!("transparency.enableOpacityMask", "Enable Opacity Mask", ["Window", "Transparency"], None, "{id?|ids?}", has_doc, |s, p| set_flags(
            s,
            p,
            "Enable Opacity Mask",
            &json!({ "disabled": false })
        )),
        cmd!(
            "transparency.unlinkOpacityMask",
            "Unlink Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{id?|ids?} the object moves without its mask",
            has_doc,
            |s, p| set_flags(s, p, "Unlink Opacity Mask", &json!({ "linked": false }))
        ),
        cmd!("transparency.linkOpacityMask", "Link Opacity Mask", ["Window", "Transparency"], None, "{id?|ids?}", has_doc, |s, p| set_flags(
            s,
            p,
            "Link Opacity Mask",
            &json!({ "linked": true })
        )),
        cmd!(
            "transparency.setOpacityMask",
            "Opacity Mask Options",
            [],
            None,
            "{id?|ids?, clip?, invert?, disabled?, linked?} change the opacity masks of `ids`, the selection, or the object whose mask is being edited",
            has_doc,
            |s, p| set_flags(s, p, "Opacity Mask Options", p)
        ),
        cmd!(
            "transparency.toggleNewMasksClipping",
            "New Opacity Masks Are Clipping",
            ["Window", "Transparency"],
            None,
            "{value?} → {value}",
            always,
            toggle_new_clip
        ),
        cmd!(
            "transparency.toggleNewMasksInverted",
            "New Opacity Masks Are Inverted",
            ["Window", "Transparency"],
            None,
            "{value?} → {value}",
            always,
            toggle_new_invert
        ),
        cmd!(
            query "transparency.opacityMaskInfo",
            "Opacity Mask Info",
            [],
            None,
            "{id?|ids?} → [{id, clip, invert, disabled, linked, art}] the opacity masks of `ids`, the selection, or the object whose mask is being edited",
            has_doc,
            mask_info
        ),
        cmd!(
            query "transparency.info",
            "Transparency Info",
            [],
            None,
            "{id?|ids?} → {ids, opacity: 0..100, blend, isolate, knockout: \"on\"|\"off\"|\"neutral\", knockoutShape, editingMask, pageIsolatedBlending, pageKnockoutGroup} the Transparency panel's values for `ids`, the selection, or the object whose mask is being edited. A value is null where those objects differ (or there are none); editingMask is the id of the object whose mask is being edited, or null; the page values are the document's",
            has_doc,
            info
        ),
        cmd!(
            "transparency.togglePageIsolatedBlending",
            "Page Isolated Blending",
            ["Window", "Transparency"],
            None,
            "{value?} make the page an isolated group (default: toggle): blend modes of top-level objects don't blend with what lies under the page; saved with the document and written to PDF → {value}",
            has_doc,
            |s, p| toggle_page(s, p, false)
        ),
        cmd!(
            "transparency.togglePageKnockoutGroup",
            "Page Knockout Group",
            ["Window", "Transparency"],
            None,
            "{value?} make the page a knockout group (default: toggle): its layers (the contents of neutral layers) hide what they cover instead of showing it through their transparency; saved with the document and written to PDF → {value}",
            has_doc,
            |s, p| toggle_page(s, p, true)
        ),
    ]
}

/// The objects the Transparency panel's commands act on: `ids` / `id`, else the object whose
/// opacity mask is being edited (the selection then is its mask art), else the targeted object,
/// group or layer (`layer.target`), else the selection.
pub(crate) fn transparency_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    if p.get("ids").is_some() || p.get("id").is_some() {
        return targets(s, p);
    }
    Ok(match st.doc.mask_edit {
        Some(me) => vec![me.object],
        None => st.selection.subjects().to_vec(),
    })
}

/// An opacity parameter, in percent (0..100) everywhere, as the model's 0..1.
pub(crate) fn percent(o: f64) -> f32 {
    (o / 100.0).clamp(0.0, 1.0) as f32
}

/// The [`transparency_targets`] that have an opacity mask.
fn masked_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids: Vec<NodeId> = transparency_targets(s, p)?.into_iter().filter(|id| d.node(*id).is_some_and(|n| n.extras().mask.is_some())).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("no target object has an opacity mask: select one or give `id`".into()));
    }
    Ok(ids)
}

/// Make Opacity Mask: a document is open, and one selected object, which [`make`] gives an empty
/// mask and takes into mask editing, is where mask editing can be entered
/// ([`super::maskedit::can_edit_mask`]: not in isolation mode or pattern editing mode). The
/// selection is one object when it has one subject root, which `make` masks without `ids`, and at
/// most one outermost object, which it masks with `ids` (a call's `ids` stand in for the selection
/// here). A targeted layer is one subject root however many objects it selects. `make` itself
/// refuses any single target in those modes, a layer included, so a selection with a target counts
/// as one object.
fn can_make(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let Some(st) = s.active() else { return Ok(()) };
    let roots = super::appearance::subject_roots(s).map_or(0, |r| r.len());
    let outermost = super::edit::outermost(&st.doc, st.selection.in_paint_order(&st.doc)).len();
    let targeted = st.selection.target.is_some();
    if roots == 1 && (outermost <= 1 || targeted) { super::maskedit::can_edit_mask(s) } else { Ok(()) }
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.makeOpacityMask";
    let d = &s.doc()?.doc;
    if d.mask_edit.is_some() {
        return Err(bad(C, "stop editing the opacity mask first"));
    }
    let ids = match ids_param(p, "ids") {
        Some(ids) => {
            if let Some(id) = ids.iter().find(|id| d.node(**id).is_none()) {
                return Err(EngineError::NoNode(*id));
            }
            super::edit::outermost(d, Selection { objects: ids, ..Default::default() }.in_paint_order(d))
        }
        None => super::appearance::subject_roots(s)?,
    };
    let Some((&top, below)) = ids.split_last() else { return Err(bad(C, "select the art and, on top of it, the mask object")) };
    // One target gets an empty mask and enters mask editing, which isolation mode and pattern
    // editing mode do not allow.
    if below.is_empty()
        && let Some(why) = super::maskedit::mask_editing_refused(s.doc()?)
    {
        return Err(bad(C, why));
    }
    let clip = bool_or(p, "clip", !s.menu.new_masks_unclipped);
    let invert = bool_or(p, "invert", s.menu.new_masks_inverted);
    // Several objects: the top one is the mask. One: an empty mask, drawn in mask editing.
    let (art, mask_id) = if below.is_empty() { (&ids[..], None) } else { (below, Some(top)) };
    let (id, layer) = s.edit("Make Opacity Mask", |d, sel| {
        let mask_art = match mask_id {
            Some(m) => (*d.remove(m)?).clone(),
            None => mask_group(vec![]),
        };
        // Several objects are grouped so they share one mask.
        let target = if let [one] = art {
            *one
        } else if separate(d, art) {
            let gid = d.alloc_id();
            d.wrap(art, |children| Node::group(gid, children))?
        } else {
            let last = art[art.len() - 1];
            let (par, idx, _) = d.position(last).ok_or(EngineError::NoNode(last))?;
            let gid = d.alloc_id();
            d.insert(par, idx + 1, Node::group(gid, vec![]))?;
            for id in art {
                d.move_node(*id, Some(gid), usize::MAX)?;
            }
            gid
        };
        let n = d.node_mut(target).ok_or(EngineError::NoNode(target))?;
        if n.extras().mask.is_some() {
            return Err(EngineError::Other("the object already has an opacity mask".into()));
        }
        let mut m = OpacityMask::new(mask_art, clip);
        m.invert = invert;
        n.extras_mut().mask = Some(Box::new(m));
        sel.set_target(d, target);
        let layer = if mask_id.is_none() { Some(begin(d, sel, target)?) } else { None };
        Ok((target, layer))
    })?;
    if layer.is_some() {
        show_editing(s, layer)?;
    }
    Ok(json!({ "id": id.0, "editing": layer.is_some() }))
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = masked_targets(s, p)?;
    let editing = s.doc()?.doc.mask_edit.is_some();
    s.edit("Release Opacity Mask", |d, sel| {
        // Leave mask editing first: its art goes back into the mask, which is released below.
        finish(d, sel);
        let mut out = vec![];
        let mut released = Beside::above(d, &ids);
        for id in &ids {
            let Some(m) = d.node_mut(*id).and_then(|n| n.extras_mut().mask.take()) else { continue };
            out.push(*id);
            // Fresh ids: copies of a masked object share the ids inside their mask art.
            let parts: Vec<Node> = mask_parts(&m.art).iter().map(|part| d.reid(part)).collect();
            out.extend(parts.iter().map(|p| p.id));
            released.put(d, *id, parts)?;
        }
        released.finish(d)?;
        // Copied type whose layout caches `reid` dropped has them again.
        super::typecmd::refresh_bounds_of(d, &out);
        sel.set(out);
        Ok(())
    })?;
    if editing {
        show_editing(s, None)?;
    }
    ok()
}

/// Set the mask flags given in `p` on the masked objects among `target`'s targets.
fn set_flags(s: &mut Session, target: &Value, label: &str, p: &Value) -> Result<Value> {
    let get = |k: &str| p.get(k).and_then(Value::as_bool);
    let (clip, invert, disabled, linked) = (get("clip"), get("invert"), get("disabled"), get("linked"));
    if clip.is_none() && invert.is_none() && disabled.is_none() && linked.is_none() {
        return Err(bad("transparency.setOpacityMask", "give at least one of clip, invert, disabled, linked"));
    }
    let ids = masked_targets(s, target)?;
    // When every mask already has the flags given, the document and its undo history stay as they
    // are.
    let has_flags = |m: &OpacityMask| {
        clip.is_none_or(|v| m.clip == v)
            && invert.is_none_or(|v| m.invert == v)
            && disabled.is_none_or(|v| m.disabled == v)
            && linked.is_none_or(|v| m.linked == v)
    };
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).and_then(|n| n.extras().mask.as_deref()).is_none_or(has_flags)) {
        return ok();
    }
    s.edit(label, |d, _| {
        for id in &ids {
            if let Some(m) = d.node_mut(*id).and_then(|n| n.extras_if_any().and_then(|e| e.mask.as_mut())) {
                m.clip = clip.unwrap_or(m.clip);
                m.invert = invert.unwrap_or(m.invert);
                m.disabled = disabled.unwrap_or(m.disabled);
                m.linked = linked.unwrap_or(m.linked);
            }
        }
        Ok(())
    })?;
    ok()
}

fn toggle_new_clip(s: &mut Session, p: &Value) -> Result<Value> {
    let v = bool_or(p, "value", s.menu.new_masks_unclipped);
    s.menu.new_masks_unclipped = !v;
    Ok(json!({ "value": v }))
}

/// Set (or toggle) the page's knockout group flag (`knockout`) or isolated blending flag, as one
/// undo step; setting the current value changes nothing.
fn toggle_page(s: &mut Session, p: &Value, knockout: bool) -> Result<Value> {
    fn flag(d: &mut Document, knockout: bool) -> &mut bool {
        if knockout { &mut d.page_knockout } else { &mut d.page_isolate }
    }
    let d = &s.doc()?.doc;
    let cur = if knockout { d.page_knockout } else { d.page_isolate };
    let v = bool_or(p, "value", !cur);
    if v != cur {
        s.edit(if knockout { "Page Knockout Group" } else { "Page Isolated Blending" }, |d, _| {
            *flag(d, knockout) = v;
            Ok(())
        })?;
    }
    Ok(json!({ "value": v }))
}

fn toggle_new_invert(s: &mut Session, p: &Value) -> Result<Value> {
    let v = bool_or(p, "value", !s.menu.new_masks_inverted);
    s.menu.new_masks_inverted = v;
    Ok(json!({ "value": v }))
}

/// The Transparency panel's values for its targets ([`Session::transparency_info`]). A value is
/// `None` where the targets differ, or when there are none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransparencyInfo {
    /// The targets (the selection, in selection order, by default).
    pub ids: Vec<NodeId>,
    /// Opacity in percent (0..100), to two decimals.
    pub opacity: Option<f64>,
    pub blend: Option<BlendMode>,
    pub isolate: Option<bool>,
    pub knockout: Option<Knockout>,
    pub knockout_shape: Option<bool>,
}

impl TransparencyInfo {
    fn of(d: &Document, ids: Vec<NodeId>) -> Self {
        let nodes = nodes_of(d, &ids);
        fn same<T: PartialEq>(nodes: &[&Node], f: impl Fn(&Node) -> T) -> Option<T> {
            let first = f(nodes.first()?);
            nodes[1..].iter().all(|n| f(n) == first).then_some(first)
        }
        Self {
            // Compared in hundredths of a percent, as shown.
            opacity: same(&nodes, |n| (n.opacity as f64 * 10_000.0).round() as i64).map(|o| o as f64 / 100.0),
            blend: same(&nodes, |n| n.blend),
            isolate: same(&nodes, |n| n.isolate),
            knockout: same(&nodes, |n| n.knockout),
            knockout_shape: same(&nodes, |n| n.knockout_shape),
            ids,
        }
    }

    /// The opacity of the picked gradient mesh `points` (see `transparency.set`) instead of their
    /// objects': `None` where they differ.
    fn with_mesh_points(mut self, d: &Document, points: Option<Vec<(NodeId, Vec<usize>)>>) -> Self {
        let Some(points) = points else { return self };
        let mut all = points.iter().flat_map(|(id, v)| {
            let m = match d.node(*id).map(|n| &n.kind) {
                Some(NodeKind::Mesh(m)) => Some(m),
                _ => None,
            };
            v.iter().filter_map(move |i| m?.points.get(*i)).map(|q| (q.opacity as f64 * 10_000.0).round() as i64)
        });
        let first = all.next();
        self.opacity = first.filter(|f| all.all(|o| o == *f)).map(|o| o as f64 / 100.0);
        self
    }
}

/// The nodes of `ids` that exist: looked up one by one when few, else in one walk of the tree.
fn nodes_of<'a>(d: &'a Document, ids: &[NodeId]) -> Vec<&'a Node> {
    if ids.len() <= 8 {
        return ids.iter().filter_map(|id| d.node(*id)).collect();
    }
    let set: HashSet<NodeId> = ids.iter().copied().collect();
    let mut out = Vec::with_capacity(ids.len());
    d.walk(|n| {
        if set.contains(&n.id) {
            out.push(n);
        }
    });
    out
}

impl Session {
    /// Defaults for new opacity masks (clip, invert) from the Transparency panel menu.
    pub fn new_mask_defaults(&self) -> (bool, bool) {
        (!self.menu.new_masks_unclipped, self.menu.new_masks_inverted)
    }

    /// The Transparency panel's values for the selection, or for the object whose mask is being
    /// edited (`transparency.info` without ids).
    pub fn transparency_info(&self) -> TransparencyInfo {
        match (self.active(), transparency_targets(self, &Value::Null)) {
            (Some(st), Ok(ids)) => {
                let points = super::paint::picked_mesh_points_of(self, &ids);
                TransparencyInfo::of(&st.doc, ids).with_mesh_points(&st.doc, points)
            }
            _ => TransparencyInfo::default(),
        }
    }
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = transparency_targets(s, p)?;
    let points = super::paint::picked_mesh_points_of(s, &ids);
    let i = TransparencyInfo::of(&st.doc, ids).with_mesh_points(&st.doc, points);
    Ok(json!({
        "ids": i.ids.iter().map(|id| id.0).collect::<Vec<_>>(),
        "opacity": i.opacity,
        "blend": i.blend.map(BlendMode::label),
        "isolate": i.isolate,
        "knockout": i.knockout.map(Knockout::label),
        "knockoutShape": i.knockout_shape,
        "editingMask": st.doc.mask_edit.map(|me| me.object.0),
        "pageIsolatedBlending": st.doc.page_isolate,
        "pageKnockoutGroup": st.doc.page_knockout,
    }))
}

fn mask_info(s: &mut Session, p: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let v: Vec<Value> = nodes_of(d, &transparency_targets(s, p)?)
        .into_iter()
        .filter_map(|n| {
            let m = n.extras().mask.as_deref()?;
            Some(json!({ "id": n.id.0, "clip": m.clip, "invert": m.invert, "disabled": m.disabled, "linked": m.linked, "art": m.art.id.0 }))
        })
        .collect();
    Ok(json!(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session_with_two() -> (Session, u64, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
        (s, a, b)
    }

    fn select(s: &mut Session, ids: &[u64]) {
        s.execute("select.set", &json!({ "ids": ids })).unwrap();
    }

    #[test]
    fn make_moves_top_object_into_mask() {
        let (mut s, a, b) = session_with_two();
        select(&mut s, &[a, b]);
        let r = s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        assert_eq!(r["id"].as_u64(), Some(a));
        assert_eq!(r["editing"], false);
        let d = &s.doc().unwrap().doc;
        assert!(d.node(NodeId(b)).is_none(), "mask art left the layer tree");
        let m = d.node(NodeId(a)).unwrap().extras().mask.as_deref().unwrap();
        assert!(m.clip && m.linked && !m.invert && !m.disabled);
        assert_eq!(m.art.id, NodeId(b));
    }

    #[test]
    fn make_groups_several_objects() {
        let (mut s, a, b) = session_with_two();
        let c = s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        select(&mut s, &[a, b, c]);
        let g = s.execute("transparency.makeOpacityMask", &json!({"clip": false})).unwrap()["id"].as_u64().unwrap();
        let d = &s.doc().unwrap().doc;
        let n = d.node(NodeId(g)).unwrap();
        assert_eq!(n.children().unwrap().len(), 2);
        assert!(!n.extras().mask.as_ref().unwrap().clip);
    }

    #[test]
    fn release_restores_mask_art_and_undo_restores_mask() {
        let (mut s, a, b) = session_with_two();
        select(&mut s, &[a, b]);
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        s.execute("transparency.releaseOpacityMask", &json!({})).unwrap();
        let st = s.doc().unwrap();
        assert!(st.doc.node(NodeId(a)).unwrap().extras().mask.is_none());
        assert_eq!(st.selection.len(), 2);
        assert_eq!(st.doc.layers[0].children().unwrap().len(), 2);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().mask.is_some());
    }

    #[test]
    fn flags_and_linking() {
        let (mut s, a, b) = session_with_two();
        select(&mut s, &[a, b]);
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        s.execute("transparency.setOpacityMask", &json!({"invert": true, "clip": false})).unwrap();
        s.execute("transparency.disableOpacityMask", &json!({})).unwrap();
        s.execute("transparency.unlinkOpacityMask", &json!({})).unwrap();
        let info = s.execute("transparency.opacityMaskInfo", &json!({})).unwrap();
        assert_eq!(info[0]["invert"], true);
        assert_eq!(info[0]["clip"], false);
        assert_eq!(info[0]["disabled"], true);
        assert_eq!(info[0]["linked"], false);
        // Unlinked: moving the object leaves the mask art where it was.
        let before = s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().mask.as_ref().unwrap().art.clone();
        s.execute("object.move", &json!({"dx": 30, "dy": 0})).unwrap();
        let after = s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().mask.as_ref().unwrap().art.clone();
        assert_eq!(before.geometric_bounds(), after.geometric_bounds());
        s.execute("transparency.linkOpacityMask", &json!({})).unwrap();
        s.execute("object.move", &json!({"dx": 30, "dy": 0})).unwrap();
        let moved = s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().mask.as_ref().unwrap().art.clone();
        assert!((moved.geometric_bounds().unwrap().x0 - after.geometric_bounds().unwrap().x0 - 30.0).abs() < 1e-9);
    }

    #[test]
    fn new_masks_clipping_toggle() {
        let (mut s, a, b) = session_with_two();
        assert_eq!(s.execute("transparency.toggleNewMasksClipping", &json!({})).unwrap()["value"], false);
        select(&mut s, &[a, b]);
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        assert!(!s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().mask.as_ref().unwrap().clip);
        s.execute("transparency.toggleNewMasksInverted", &json!({})).unwrap();
        assert_eq!(s.new_mask_defaults(), (false, true));
    }

    #[test]
    fn masks_survive_native_round_trip() {
        let (mut s, a, b) = session_with_two();
        select(&mut s, &[a, b]);
        s.execute("transparency.makeOpacityMask", &json!({"invert": true})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let bytes = vectorcraft_format::save(&d, false);
        let back = vectorcraft_format::load(&bytes).unwrap();
        assert_eq!(back.node(NodeId(a)).unwrap().extras().mask, d.node(NodeId(a)).unwrap().extras().mask);
    }
}
