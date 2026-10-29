//! A cache of every node's index path, so a lookup by id checks one path instead of walking the
//! tree.
//!
//! The tree is edited in many places through public fields, so the cache can't be kept in step
//! with it. It is checked instead: a cached path is used only while the node at that path still
//! has the id (a check as deep as the tree). A path that no longer checks out rebuilds the cache,
//! and an id the cache doesn't have (deleted, or added since) is looked up by walking, as before.
//! An id that appears more than once (a damaged file) is never cached, so lookups keep returning
//! its first place in tree order.
//!
//! The one case a cached lookup can answer differently from a walk: a second node with an
//! existing id put in the tree *after* the cache was built, earlier in tree order than the first.
//! The lookup then returns the original node (also one with that id) instead of the new one. The
//! app never makes such a node (paste and duplicate give new ids, undo restores whole trees);
//! duplicated ids come from damaged files, which are cached afresh when opened.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{Arc, RwLock};

use crate::{Node, NodeId};

/// Hasher for node ids: one multiply, by a number chosen at random once per run (a document's ids
/// are its author's, and a fixed multiplier lets them be picked to collide). The default SipHash
/// dominated the cost of building the cache.
#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(multiplier());
    }
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 29)
    }
}

/// The odd multiplier of [`IdHasher`], random per run.
fn multiplier() -> u64 {
    static KEY: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *KEY.get_or_init(|| std::hash::BuildHasher::hash_one(&std::hash::RandomState::new(), 0x9e37_79b9_7f4a_7c15_u64) | 1)
}

/// A map keyed by node id with [`IdHasher`].
pub type IdMap<V> = HashMap<NodeId, V, BuildHasherDefault<IdHasher>>;
/// Where each node is: its parent's entry (`u32::MAX` for a top-level layer) and its index among
/// its siblings. One flat table, so building it allocates nothing per node.
#[derive(Default)]
pub(crate) struct Paths {
    /// Each id's entry in `at`; `None` for an id that appears more than once.
    ids: IdMap<Option<u32>>,
    at: Vec<(u32, u32)>,
}

impl Paths {
    /// The index path of `id`: `Some(None)` when the id appears more than once, `None` when it
    /// isn't in the table.
    pub(crate) fn get(&self, id: &NodeId) -> Option<Option<Vec<usize>>> {
        let e = (*self.ids.get(id)?)?;
        let mut path = Vec::new();
        let mut cur = e;
        // Each step goes to a parent entry, made before its children: at most `at.len()` steps.
        for _ in 0..=self.at.len() {
            let Some(&(parent, i)) = self.at.get(cur as usize) else { return Some(None) };
            path.push(i as usize);
            if parent == u32::MAX {
                path.reverse();
                return Some(Some(path));
            }
            cur = parent;
        }
        Some(None)
    }
    pub(crate) fn contains_key(&self, id: &NodeId) -> bool {
        self.ids.contains_key(id)
    }
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Lookups that had to walk (the cache didn't have the id, or its path no longer checks out)
/// before the cache is rebuilt: a rebuild costs a few walks, so rebuilding only after a few keeps
/// a lookup's cost within a small factor of a walk whatever the edit pattern (say, delete an
/// object and look the next one up, again and again).
const MISSES_PER_REBUILD: u32 = 4;

/// The cache, shared by clones of a document (each checks every path it uses against its own tree).
#[derive(Default)]
pub struct IdIndex(RwLock<Option<Arc<Paths>>>, std::sync::atomic::AtomicU32);

impl IdIndex {
    /// The current cache, if one was built.
    pub(crate) fn get(&self) -> Option<Arc<Paths>> {
        self.0.read().ok().and_then(|g| g.clone())
    }

    /// Let go of the cache (a document kept in the undo history): the next lookup builds it again.
    pub(crate) fn clear(&self) {
        if let Ok(mut g) = self.0.write() {
            *g = None;
        }
        self.1.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether a cache was built (and not cleared since).
    pub(crate) fn is_built(&self) -> bool {
        self.0.read().is_ok_and(|g| g.is_some())
    }

    /// Rebuild the cache from `layers` (one walk) and return it.
    pub(crate) fn rebuild(&self, layers: &[Arc<Node>]) -> Arc<Paths> {
        fn walk(nodes: &[Arc<Node>], parent: u32, out: &mut Paths) {
            for (i, n) in nodes.iter().enumerate() {
                let e = out.at.len() as u32;
                out.at.push((parent, i as u32));
                out.ids.entry(n.id).and_modify(|x| *x = None).or_insert(Some(e));
                if let Some(ch) = n.children() {
                    walk(ch, e, out);
                }
            }
        }
        let mut out = Paths::default();
        walk(layers, u32::MAX, &mut out);
        let out = Arc::new(out);
        if let Ok(mut g) = self.0.write() {
            *g = Some(out.clone());
        }
        self.1.store(0, std::sync::atomic::Ordering::Relaxed);
        out
    }

    /// Note a lookup that had to walk (an id the cache didn't have, or a path that no longer
    /// checks out); true when it's time to rebuild.
    pub(crate) fn missed(&self) -> bool {
        self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= MISSES_PER_REBUILD
    }
}

impl Clone for IdIndex {
    fn clone(&self) -> Self {
        IdIndex(RwLock::new(self.get()), Default::default())
    }
}

/// A cache, not document content: two documents are equal whatever their caches hold.
impl PartialEq for IdIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for IdIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdIndex")
    }
}
