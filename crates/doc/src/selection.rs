//! Selection state (objects and, for direct selection, individual anchors and segments).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Document, NodeId};

/// (subpath index, anchor index) inside a path.
pub type AnchorRef = (usize, usize);

/// A segment picked by the Direct Selection tool: (subpath index, index of its first anchor, the
/// subpath's anchor count when it was picked). It counts only while that count holds and both of
/// its anchors are selected (see [`Selection::segments_of`]).
pub type SegmentRef = (usize, usize, usize);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// Selected objects in selection order.
    pub objects: Vec<NodeId>,
    /// Direct-selected anchors per path. A path in `objects` with no entry here is fully selected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anchors: BTreeMap<NodeId, BTreeSet<AnchorRef>>,
    /// Direct-selected segments per path (clicked, or cut through by a marquee): Delete removes
    /// these instead of their anchors.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub segments: BTreeMap<NodeId, BTreeSet<SegmentRef>>,
    /// Key object for Align.
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
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
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
        self.segments.clear();
        self.key = None;
        self.target = None;
        self.slices.clear();
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
        self.segments.remove(&id);
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
    pub fn toggle(&mut self, id: NodeId) {
        if self.contains(id) { self.remove(id) } else { self.add(id) }
    }
    /// Are some (not all) anchors of `id` direct-selected?
    pub fn partial(&self, id: NodeId) -> Option<&BTreeSet<AnchorRef>> {
        self.anchors.get(&id)
    }
    /// The segments of path `id` still picked: (subpath, first anchor, second anchor), for those
    /// whose subpath kept its anchor count and whose two anchors are both still selected.
    pub fn segments_of(&self, doc: &Document, id: NodeId) -> Vec<(usize, usize, usize)> {
        let (Some(segs), Some(anchors)) = (self.segments.get(&id), self.anchors.get(&id)) else { return vec![] };
        let Some(path) = doc.node(id).and_then(|n| n.path_data()) else { return vec![] };
        segs.iter()
            .filter_map(|&(si, a0, count)| {
                let sp = path.subpaths.get(si)?;
                let n = sp.anchors.len();
                let a1 = if a0 + 1 < n {
                    a0 + 1
                } else if sp.closed && a0 + 1 == n {
                    0
                } else {
                    return None;
                };
                (n == count && n >= 2 && anchors.contains(&(si, a0)) && anchors.contains(&(si, a1))).then_some((si, a0, a1))
            })
            .collect()
    }
    /// Drop ids that no longer exist or are no longer editable.
    pub fn prune(&mut self, doc: &Document) {
        self.objects.retain(|id| doc.node(*id).is_some());
        self.anchors.retain(|id, _| doc.node(*id).is_some());
        self.segments.retain(|id, _| doc.node(*id).is_some());
        if self.key.is_some_and(|k| doc.node(k).is_none()) {
            self.key = None;
        }
        if self.target.is_some_and(|t| doc.node(t).is_none()) {
            self.target = None;
        }
        self.slices.retain(|id| doc.is_slice(*id));
    }
    /// Target `id` (see [`Selection::target`]): a layer gets its visible, unlocked art selected,
    /// anything else is selected itself.
    pub fn set_target(&mut self, doc: &Document, id: NodeId) {
        match doc.node(id) {
            Some(n) if n.is_layer() => self.set(n.children().into_iter().flatten().filter(|c| c.visible && !c.locked).map(|c| c.id)),
            _ => self.set([id]),
        }
        self.target = Some(id);
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
}
