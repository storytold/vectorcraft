//! Selection state (objects and, for direct selection, anchors and segments; ruler guides).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::{Document, NodeId, NodeKind};

/// (subpath index, anchor index) inside a path.
pub type AnchorRef = (usize, usize);
/// (subpath index, segment index) inside a path.
pub type SegmentRef = (usize, usize);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// Selected objects in selection order.
    pub objects: Vec<NodeId>,
    /// Direct-selected anchors, or a selected segment's endpoints used for movement. A path in
    /// `objects` with no entry here is fully selected; `segments` distinguishes segment picks.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anchors: BTreeMap<NodeId, BTreeSet<AnchorRef>>,
    /// Explicitly selected segments. Their endpoints in `anchors` move with them, but are not
    /// selected anchor points: Delete cuts connections and their anchor squares stay hollow.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub segments: BTreeMap<NodeId, BTreeSet<SegmentRef>>,
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
        self.segments.clear();
        self.key = None;
        self.target = None;
        self.slices.clear();
        self.guides.clear();
    }
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.clear();
        // What `add` per id does after a clear, without its linear duplicate check per id.
        let mut seen = std::collections::HashSet::new();
        self.objects.extend(ids.into_iter().filter(|id| seen.insert(*id)));
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
    /// The objects selected whole: those with no direct-selected anchors.
    pub fn whole(&self) -> Vec<NodeId> {
        self.objects.iter().copied().filter(|id| !self.anchors.contains_key(id)).collect()
    }
    /// With direct-selected anchors, what a transform of them measures from: the box around those
    /// anchors and the objects selected whole (`visual`: with their strokes). `None` without any.
    pub fn anchor_bounds(&self, doc: &Document, visual: bool) -> Option<vectorcraft_geom::Rect> {
        if self.anchors.is_empty() {
            return None;
        }
        vectorcraft_geom::union_opt(doc.bounds_of(&self.whole(), visual), self.anchors_box(doc))
    }
    /// The box around the anchors selected with Direct Selection. `None` without any.
    pub fn anchors_box(&self, doc: &Document) -> Option<vectorcraft_geom::Rect> {
        let mut acc = None;
        for (id, set) in &self.anchors {
            let Some(NodeKind::Path { path, .. }) = doc.node(*id).map(|n| &n.kind) else { continue };
            for a in set.iter().filter_map(|&(si, ai)| path.subpaths.get(si)?.anchors.get(ai)) {
                acc = vectorcraft_geom::union_opt(acc, Some(vectorcraft_geom::Rect::from_points(a.p, a.p)));
            }
        }
        acc
    }
    /// Which direction handles belong to selected segments; both for an anchor selection.
    pub fn handle_sides(&self, id: NodeId, si: usize, ai: usize, sp: &vectorcraft_geom::SubPath) -> (bool, bool) {
        let Some(segments) = self.segments.get(&id) else { return (true, true) };
        let previous = ai.checked_sub(1).or_else(|| sp.closed.then_some(sp.segment_count()).and_then(|n| n.checked_sub(1)));
        (previous.is_some_and(|i| segments.contains(&(si, i))), segments.contains(&(si, ai)))
    }
    /// Drop ids that no longer exist or are no longer editable.
    pub fn prune(&mut self, doc: &Document) {
        // Every id this looks up, found in one walk of the tree.
        let ids = self.objects.iter().chain(self.anchors.keys()).chain(self.segments.keys()).copied().chain(self.key).chain(self.target);
        let found = doc.index_paths(ids);
        self.objects.retain(|id| found.contains_key(id));
        self.anchors.retain(|id, _| found.contains_key(id));
        self.segments.retain(|id, refs| {
            let Some(path) = found.get(id).and_then(|p| doc.node_at(p)).and_then(|n| n.path_data()) else { return false };
            refs.retain(|&(si, seg)| path.subpaths.get(si).is_some_and(|sp| seg < sp.segment_count()));
            !refs.is_empty() && self.objects.contains(id) && self.anchors.contains_key(id)
        });
        // The key object stays while another selected object lies outside it.
        let keep = self.key.and_then(|k| found.get(&k)).and_then(|p| doc.node_at(p)).is_some_and(|n| {
            let mut inside = BTreeSet::new();
            n.walk(&mut |c| {
                inside.insert(c.id);
            });
            self.objects.iter().any(|o| !inside.contains(o))
        });
        if !keep {
            self.key = None;
        }
        if self.target.is_some_and(|t| !found.contains_key(&t)) {
            self.target = None;
        }
        self.slices.retain(|id| doc.is_slice(*id));
        // Guides on a hidden or locked layer can't stay selected.
        self.guides.retain(|i| doc.guides.get(*i).is_some_and(|g| doc.guide_editable(g)));
    }
    /// Deselect the objects that are hidden or locked, themselves or through a group or layer
    /// around them (and so the key object when it is one): hidden or locked art can't stay
    /// selected.
    pub fn deselect_uneditable(&mut self, doc: &Document) {
        let uneditable: HashSet<NodeId> = self.objects.iter().copied().filter(|o| !doc.is_editable(*o)).collect();
        if uneditable.is_empty() {
            return;
        }
        self.target = None;
        self.slices.clear();
        self.objects.retain(|o| !uneditable.contains(o));
        self.anchors.retain(|id, _| !uneditable.contains(id));
        self.segments.retain(|id, _| !uneditable.contains(id));
        if self.key.is_some_and(|k| uneditable.contains(&k)) {
            self.key = None;
        }
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
mod tests;
