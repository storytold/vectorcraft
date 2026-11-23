//! The nearest node of a kind at or above each of many objects, found along one index path per
//! object.

use std::sync::Arc;

use crate::id_index::Paths;
use crate::{Document, Node, NodeId};

/// Finds, for one object after another, the nearest node that passes a test: the object itself,
/// then its parent, and so on up to its layer. Made by [`Document::nearest_finder`].
///
/// Each node on the way is tested as [`Document::node`] gives its id, so the answer is the one that
/// `doc.ancestry(id)` read from the end, with `doc.node` for each id, gives. While the lookup cache
/// places the object and each id on its path where they are, a lookup reads that one path; an id
/// the cache does not place there (one that appears more than once, or a cache older than the
/// tree) is looked up with [`Document::node`].
pub struct NearestFinder<'a> {
    doc: &'a Document,
    cache: Option<Arc<Paths>>,
    /// The cache entries from the top-level layer down to the object.
    entries: Vec<u32>,
    /// The nodes at those entries.
    nodes: Vec<&'a Node>,
}

impl Document {
    /// A [`NearestFinder`] for this document.
    pub fn nearest_finder(&self) -> NearestFinder<'_> {
        NearestFinder { doc: self, cache: self.index.get(), entries: vec![], nodes: vec![] }
    }
}

impl<'a> NearestFinder<'a> {
    /// The node nearest `id` (itself, then its parent, and so on up to its layer) for which `pred`
    /// holds; `None` when `id` is not in the tree or no node on its path passes.
    pub fn nearest(&mut self, id: NodeId, mut pred: impl FnMut(&Node) -> bool) -> Option<&'a Node> {
        let doc = self.doc;
        let ids: Vec<NodeId> = if self.cached_path(id) {
            let cache = self.cache.as_deref()?;
            // The depth of the first id from the object up that the cache does not place there.
            let mut unplaced = None;
            for (k, (&node, &e)) in self.nodes.iter().zip(&self.entries).enumerate().rev() {
                if cache.entry(&node.id) != Some(Some(e)) {
                    unplaced = Some(k);
                    break;
                }
                if pred(node) {
                    return Some(node);
                }
            }
            let k = unplaced?;
            self.nodes[..=k].iter().map(|n| n.id).collect()
        } else {
            doc.ancestry(id).unwrap_or_default()
        };
        let found = ids.into_iter().rev().find_map(|a| doc.node(a).filter(|n| pred(n)));
        // Those lookups may have built the cache again.
        self.cache = doc.index.get();
        found
    }

    /// Fill `entries` and `nodes` with the cached path to `id`; false when the cache has no path for
    /// it or the path no longer leads to it.
    fn cached_path(&mut self, id: NodeId) -> bool {
        self.entries.clear();
        self.nodes.clear();
        let Some(cache) = self.cache.as_deref() else { return false };
        let Some(Some(mut e)) = cache.entry(&id) else { return false };
        loop {
            let Some((parent, _)) = cache.place(e) else { return false };
            self.entries.push(e);
            if parent == u32::MAX {
                break;
            }
            // A parent's entry is made before its children's, so the walk up ends.
            if parent >= e {
                return false;
            }
            e = parent;
        }
        self.entries.reverse();
        let mut siblings: &'a [Arc<Node>] = &self.doc.layers;
        for &e in &self.entries {
            let Some(n) = cache.place(e).and_then(|(_, i)| siblings.get(i as usize)) else { return false };
            self.nodes.push(&**n);
            siblings = n.children().map_or(&[], |c| c.as_slice());
        }
        self.nodes.last().is_some_and(|n| n.id == id)
    }
}
