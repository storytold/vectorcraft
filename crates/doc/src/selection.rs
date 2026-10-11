//! Selection state (objects and, for direct selection, individual anchors; ruler guides).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Document, NodeId};

/// (subpath index, anchor index) inside a path.
pub type AnchorRef = (usize, usize);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// Selected objects in selection order.
    pub objects: Vec<NodeId>,
    /// Direct-selected anchors per path. A path in `objects` with no entry here is fully selected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anchors: BTreeMap<NodeId, BTreeSet<AnchorRef>>,
    /// Key object for Align, one object of a selection of two or more. [`Selection::prune`] clears
    /// it when every selected object is the key or lies inside it, as when one object is left
    /// selected or when the key is a group selected with only its own members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<NodeId>,
    /// The object, group or layer targeted through its target circle in the Layers panel, which
    /// appearance, transparency and opacity-mask commands then act on (a targeted layer has its
    /// art selected). Any other selection change clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<NodeId>,
    /// Slices selected with the Slice Selection tool: user slice ids and the ids of objects whose
    /// object slice is selected (see [`crate::slices`]). Any change to the object selection
    /// clears them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<NodeId>,
    /// Ruler guides selected with a selection tool, as indexes into [`Document::guides`]. A new
    /// selection ([`Self::set`]) clears them; Shift-adding or -removing objects keeps them, so
    /// guides and objects can be selected together, in either order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<usize>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
    /// Is anything selected that Delete and the arrow keys act on: objects or ruler guides?
    pub fn has_objects_or_guides(&self) -> bool {
        !self.objects.is_empty() || !self.guides.is_empty()
    }
    pub fn len(&self) -> usize {
        self.objects.len()
    }
    pub fn contains(&self, id: NodeId) -> bool {
        self.objects.contains(&id)
    }
    pub fn clear(&mut self) {
        self.objects.clear();
        self.anchors.clear();
        self.key = None;
        self.target = None;
        self.slices.clear();
        self.guides.clear();
    }
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.clear();
        for id in ids {
            self.add(id);
        }
    }
    pub fn add(&mut self, id: NodeId) {
        self.target = None;
        self.slices.clear();
        if !self.objects.contains(&id) {
            self.objects.push(id);
        }
    }
    pub fn remove(&mut self, id: NodeId) {
        self.target = None;
        self.slices.clear();
        self.objects.retain(|x| *x != id);
        self.anchors.remove(&id);
        if self.key == Some(id) {
            self.key = None;
        }
    }
    /// Select slices `ids` (see [`Selection::slices`]) and nothing else.
    pub fn set_slices(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.clear();
        for id in ids {
            if !self.slices.contains(&id) {
                self.slices.push(id);
            }
        }
    }
    /// Select ruler guides `indexes` (see [`Selection::guides`]) and nothing else.
    pub fn set_guides(&mut self, indexes: impl IntoIterator<Item = usize>) {
        self.clear();
        for i in indexes {
            if !self.guides.contains(&i) {
                self.guides.push(i);
            }
        }
    }
    /// Add ruler guide `i` to the selection, or take it out, keeping the selected objects (a
    /// Shift-click on a guide).
    pub fn toggle_guide(&mut self, i: usize) {
        if let Some(k) = self.guides.iter().position(|g| *g == i) {
            self.guides.remove(k);
        } else {
            self.guides.push(i);
        }
    }
    pub fn toggle(&mut self, id: NodeId) {
        if self.contains(id) { self.remove(id) } else { self.add(id) }
    }
    /// Are some (not all) anchors of `id` direct-selected?
    pub fn partial(&self, id: NodeId) -> Option<&BTreeSet<AnchorRef>> {
        self.anchors.get(&id)
    }
    /// Drop ids that no longer exist or are no longer editable.
    pub fn prune(&mut self, doc: &Document) {
        self.objects.retain(|id| doc.node(*id).is_some());
        self.anchors.retain(|id, _| doc.node(*id).is_some());
        // The key object stays while another selected object lies outside it.
        let keep = self.key.and_then(|k| doc.node(k)).is_some_and(|n| {
            let mut inside = BTreeSet::new();
            n.walk(&mut |c| {
                inside.insert(c.id);
            });
            self.objects.iter().any(|o| !inside.contains(o))
        });
        if !keep {
            self.key = None;
        }
        if self.target.is_some_and(|t| doc.node(t).is_none()) {
            self.target = None;
        }
        self.slices.retain(|id| doc.is_slice(*id));
        // Guides on a hidden or locked layer can't stay selected.
        self.guides.retain(|i| doc.guides.get(*i).is_some_and(|g| doc.guide_editable(g)));
    }
    /// Target `id` (see [`Selection::target`]): a layer gets its visible, unlocked art selected
    /// (the art of its sublayers too), anything else is selected itself.
    pub fn set_target(&mut self, doc: &Document, id: NodeId) {
        match doc.node(id) {
            Some(n) if n.is_layer() => self.set(n.layer_art(true)),
            _ => self.set([id]),
        }
        self.target = Some(id);
    }
    /// Shift-clicking `id`'s target circle: it (a layer: its art) joins the selection, or leaves it
    /// when it is all selected already. Several objects are then selected, none targeted.
    pub fn toggle_target(&mut self, doc: &Document, id: NodeId) {
        let ids = match doc.node(id) {
            Some(n) if n.is_layer() => n.layer_art(true),
            _ => vec![id],
        };
        if ids.iter().all(|i| self.contains(*i)) {
            ids.iter().for_each(|i| self.remove(*i));
        } else {
            ids.into_iter().for_each(|i| self.add(i));
        }
        self.target = None;
    }
    /// What appearance and transparency edits act on: the targeted object, group or layer, else
    /// the selected objects.
    pub fn subjects(&self) -> &[NodeId] {
        match &self.target {
            Some(t) => std::slice::from_ref(t),
            None => &self.objects,
        }
    }
    /// Top-level ordering: selected ids sorted by paint order (bottom first).
    pub fn in_paint_order(&self, doc: &Document) -> Vec<NodeId> {
        doc.paint_order(self.objects.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ops() {
        let mut s = Selection::default();
        s.add(NodeId(1));
        s.add(NodeId(1));
        s.toggle(NodeId(2));
        assert_eq!(s.objects, vec![NodeId(1), NodeId(2)]);
        s.toggle(NodeId(1));
        assert_eq!(s.objects, vec![NodeId(2)]);
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn guides_are_selected_on_their_own() {
        let mut s = Selection::default();
        s.add(NodeId(1));
        s.set_guides([2, 0, 2]);
        assert_eq!((s.objects.clone(), s.guides.clone()), (vec![], vec![2, 0]));
        assert!(s.has_objects_or_guides() && s.is_empty());
        // Shift-adding an object keeps them; a new selection doesn't.
        s.add(NodeId(1));
        assert_eq!(s.guides, vec![2, 0]);
        s.set([NodeId(1)]);
        assert!(s.guides.is_empty());
        // Toggled, a guide joins (or leaves) the selected objects.
        s.toggle_guide(0);
        assert_eq!((s.objects.clone(), s.guides.clone()), (vec![NodeId(1)], vec![0]));
        s.toggle_guide(0);
        assert_eq!((s.objects.clone(), s.guides.clone()), (vec![NodeId(1)], vec![]));
        // Pruning drops guides the document no longer has.
        let mut d = Document::new(100.0, 100.0);
        d.guides.push(crate::Guide::new(true, 10.0));
        s.set_guides([0, 1]);
        s.prune(&d);
        assert_eq!(s.guides, vec![0]);
    }

    #[test]
    fn guides_show_and_lock_with_their_layer() {
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        let g = crate::Guide { layer: Some(layer), ..crate::Guide::new(true, 10.0) };
        assert!(d.guide_shown(&g) && d.guide_editable(&g));
        d.node_mut(layer).unwrap().locked = true;
        assert!(d.guide_shown(&g) && !d.guide_editable(&g), "locked: shown, not editable");
        d.node_mut(layer).unwrap().visible = false;
        assert!(!d.guide_shown(&g) && !d.guide_editable(&g), "hidden");
        // On no layer, or a layer that's gone: always there.
        assert!(d.guide_shown(&crate::Guide::new(false, 5.0)));
        let gone = crate::Guide { layer: Some(NodeId(9999)), ..crate::Guide::new(true, 1.0) };
        assert!(d.guide_shown(&gone) && d.guide_editable(&gone));
        d.guides.push(g);
        assert_eq!(d.guides_on(layer).count(), 1);
        // Guides on no layer, or a gone one, are adopted by the top visible, unlocked layer.
        d.node_mut(layer).unwrap().visible = true;
        d.node_mut(layer).unwrap().locked = false;
        d.guides.push(crate::Guide::new(false, 5.0));
        d.guides.push(gone);
        assert_eq!(d.adopt_guides(), 2);
        assert!(d.guides.iter().all(|g| g.layer == Some(layer)));
        assert_eq!(d.adopt_guides(), 0, "once");
        // A guide saved before guides had layers reads as one on no layer.
        let old: crate::Guide = serde_json::from_str(r#"{"vertical": true, "pos": 5.0}"#).unwrap();
        assert_eq!(old.layer, None);
        assert!(!serde_json::to_string(&crate::Guide::new(true, 5.0)).unwrap().contains("layer"), "nothing new written");
    }

    #[test]
    fn selection_changes_clear_the_target() {
        let mut s = Selection::default();
        s.set([NodeId(2), NodeId(3)]);
        s.target = Some(NodeId(1));
        assert_eq!(s.subjects(), &[NodeId(1)]);
        s.add(NodeId(4));
        assert_eq!(s.target, None);
        assert_eq!(s.subjects(), &[NodeId(2), NodeId(3), NodeId(4)]);
        s.target = Some(NodeId(1));
        s.remove(NodeId(2));
        assert_eq!(s.target, None);
        s.target = Some(NodeId(1));
        s.set([NodeId(3)]);
        assert_eq!(s.target, None);
    }

    #[test]
    fn a_key_object_needs_two_or_more_selected_objects() {
        let mut s = Selection::default();
        s.set([NodeId(1), NodeId(2)]);
        s.key = Some(NodeId(2));
        s.remove(NodeId(1));
        assert_eq!(s.key, Some(NodeId(2)), "another object left: remove keeps the key");
        s.remove(NodeId(2));
        assert_eq!(s.key, None, "the key object itself left");
        // Pruning drops objects the document no longer has and clears the key once no other
        // selected object lies outside it.
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        let mut ids = vec![];
        for _ in 0..4 {
            let id = d.alloc_id();
            ids.push(d.insert(Some(layer), usize::MAX, crate::Node::group(id, vec![])).unwrap());
        }
        s.set(ids.iter().copied());
        s.key = Some(ids[3]);
        d.remove(ids[0]).unwrap();
        s.prune(&d);
        assert_eq!(s.key, Some(ids[3]), "three objects left: the key stays");
        s.toggle(ids[1]);
        s.prune(&d);
        assert_eq!(s.key, Some(ids[3]), "two objects left: the key stays");
        s.toggle(ids[2]);
        s.prune(&d);
        assert_eq!((s.objects.clone(), s.key), (vec![ids[3]], None), "one object left");
        // A group key selected with only its own member is cleared too; another selected object
        // outside the group keeps it.
        let member = d.alloc_id();
        d.insert(Some(ids[3]), usize::MAX, crate::Node::group(member, vec![])).unwrap();
        s.set([member, ids[3]]);
        s.key = Some(ids[3]);
        s.prune(&d);
        assert_eq!(s.key, None, "a group and its own member");
        s.set([member, ids[3], ids[2]]);
        s.key = Some(ids[3]);
        s.prune(&d);
        assert_eq!(s.key, Some(ids[3]), "another object outside the group");
    }
}
