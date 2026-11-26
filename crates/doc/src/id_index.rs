//! A cache of every node's index path, so a lookup by id checks one path instead of walking the
//! tree.
//!
//! The tree is edited in many places through public fields, so the cache can't be kept in step
//! with it. It is checked instead: a cached path is used only while the node at that path still
//! has the id (a check as deep as the tree). A path that no longer checks out rebuilds the cache,
//! and an id the cache doesn't have (added since, or removed) is looked up by walking, as before.
//! An id that appears more than once (a damaged file) is never cached, so lookups keep returning
//! its first place in tree order.
//!
//! [`crate::Document::remove_many`] keeps the cache and notes the nodes it removes, so the cache
//! places each node after a removed sibling one lower and finds no path for a removed node or one
//! inside it. Once the removed nodes are half the cache's entries, the cache is let go. The first
//! lookup with no cache builds it and answers from it.
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

/// The children removed from each parent since the table was built: the parent's entry (`u32::MAX`
/// for the top-level layers) and the removed children's indices in the table, sorted.
type Gaps = HashMap<u32, Vec<u32>>;

/// The cache as a document reads it: the table, and the nodes removed from the tree since it was
/// built. A node's index among its siblings is its index in the table less the removed siblings
/// before it.
#[derive(Clone)]
pub(crate) struct Paths {
    table: Arc<Table>,
    gaps: Arc<Gaps>,
}

/// Where each node was when the cache was built: its parent's entry (`u32::MAX` for a top-level
/// layer) and its index among its siblings. One flat table, so building it allocates nothing per
/// node.
#[derive(Default)]
struct Table {
    /// Each id's entry in `at`; `None` for an id that appears more than once.
    ids: IdMap<Option<u32>>,
    at: Vec<(u32, u32)>,
}

impl Paths {
    /// The index path of `id`: `Some(None)` when the id appears more than once, `None` when it
    /// isn't in the table or it, or a node it is in, was removed.
    pub(crate) fn get(&self, id: &NodeId) -> Option<Option<Vec<usize>>> {
        let e = (*self.table.ids.get(id)?)?;
        let mut path = Vec::new();
        let mut cur = e;
        // Each step goes to a parent entry, made before its children: at most `at.len()` steps.
        for _ in 0..=self.table.at.len() {
            let Some(&(parent, i)) = self.table.at.get(cur as usize) else { return Some(None) };
            path.push(self.shifted(parent, i)? as usize);
            if parent == u32::MAX {
                path.reverse();
                return Some(Some(path));
            }
            cur = parent;
        }
        Some(None)
    }
    /// Where the table has `id` as it was built: its parent's entry and its index among its
    /// siblings then; `None` when the id isn't in the table or appears more than once.
    pub(crate) fn table_place(&self, id: &NodeId) -> Option<(u32, u32)> {
        let e = (*self.table.ids.get(id)?)?;
        self.table.at.get(e as usize).copied()
    }
    pub(crate) fn contains_key(&self, id: &NodeId) -> bool {
        self.table.ids.contains_key(id)
    }
    pub(crate) fn len(&self) -> usize {
        self.table.ids.len()
    }
    /// The entry of `id` in the table: `Some(None)` when the id appears more than once, `None`
    /// when it isn't in the table.
    pub(crate) fn entry(&self, id: &NodeId) -> Option<Option<u32>> {
        self.table.ids.get(id).copied()
    }
    /// Entry `e`'s parent entry (`u32::MAX` for a top-level layer) and its index among its
    /// siblings; `None` when the node at `e` was removed.
    pub(crate) fn place(&self, e: u32) -> Option<(u32, u32)> {
        let (parent, i) = self.table.at.get(e as usize).copied()?;
        Some((parent, self.shifted(parent, i)?))
    }
    /// The index among its siblings of the child of `parent` at index `i` in the table: `i` less
    /// the removed children before it, or `None` when that child was removed.
    fn shifted(&self, parent: u32, i: u32) -> Option<u32> {
        let Some(gone) = self.gaps.get(&parent) else { return Some(i) };
        match gone.binary_search(&i) {
            Ok(_) => None,
            Err(k) => Some(i - k as u32),
        }
    }
}

/// Lookups that had to walk (the cache didn't have the id, or its path no longer checks out)
/// before the cache is rebuilt: a rebuild costs a few walks, so rebuilding only after a few keeps
/// a lookup's cost within a small factor of a walk whatever the edit pattern (say, delete an
/// object and look the next one up, again and again).
const MISSES_PER_REBUILD: u32 = 4;

thread_local! {
    /// [`tree_walks`] of this thread.
    static TREE_WALKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times lookups by id on this thread have walked a document's tree, reading up to all of
/// its nodes: one count per search of the tree and per rebuild of the cache. An edit that looks its
/// objects up one at a time counts about one walk per object; one that looks them all up at once
/// counts a few.
pub fn tree_walks() -> u64 {
    TREE_WALKS.get()
}

/// Adds one to [`tree_walks`].
pub(crate) fn note_tree_walk() {
    TREE_WALKS.set(TREE_WALKS.get().saturating_add(1));
}

thread_local! {
    /// [`path_lookups`] of this thread.
    static PATH_LOOKUPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times [`crate::Document::index_path`] has run on this thread, the lookups that
/// [`crate::Document::node`] and [`crate::Document::ancestry`] make through it included.
pub fn path_lookups() -> u64 {
    PATH_LOOKUPS.get()
}

/// Adds one to [`path_lookups`].
pub(crate) fn note_path_lookup() {
    PATH_LOOKUPS.set(PATH_LOOKUPS.get().saturating_add(1));
}

/// The cache, shared by clones of a document (each checks every path it uses against its own tree),
/// the lookups that missed it, and the nodes removed from the tree since it was built.
#[derive(Default)]
pub struct IdIndex(RwLock<Option<Arc<Paths>>>, std::sync::atomic::AtomicU32, std::sync::atomic::AtomicUsize);

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
        self.2.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Note the nodes removed from the tree while the cache is kept (their entries go stale); true
    /// when the stale entries are half the cache or more, and it's time to let it go. `count(limit)`
    /// counts the removed nodes, and may stop once it reaches `limit`, the number that makes half.
    /// With no cache, `count` isn't called.
    pub(crate) fn removed(&self, count: impl FnOnce(usize) -> usize) -> bool {
        let Some(paths) = self.get() else { return false };
        let half = paths.len().div_ceil(2);
        let n = count(half.saturating_sub(self.2.load(std::sync::atomic::Ordering::Relaxed)));
        self.2.fetch_add(n, std::sync::atomic::Ordering::Relaxed).saturating_add(n) >= half
    }

    /// Whether a cache was built (and not cleared since).
    pub(crate) fn is_built(&self) -> bool {
        self.0.read().is_ok_and(|g| g.is_some())
    }

    /// Note the nodes that [`crate::Document::remove_many`] took out of the tree, each as its place
    /// in the table ([`Paths::table_place`]): the cache then places their later siblings one lower
    /// for each, and has no path for them or the nodes in them. With no cache, it does nothing.
    pub(crate) fn take_out(&self, gone: &[(u32, u32)]) {
        let Ok(mut g) = self.0.write() else { return };
        let Some(paths) = g.as_mut() else { return };
        // Clones of the document keep the cache as it was.
        let gaps = Arc::make_mut(&mut Arc::make_mut(paths).gaps);
        let mut gone = gone.to_vec();
        gone.sort_unstable();
        // One merge of sorted lists per parent, so a removal of many children costs one pass.
        for run in gone.chunk_by(|a, b| a.0 == b.0) {
            let Some(&(parent, _)) = run.first() else { continue };
            let list = gaps.entry(parent).or_default();
            let old = std::mem::take(list);
            list.reserve(old.len() + run.len());
            let (mut a, mut b) = (old.into_iter().peekable(), run.iter().map(|&(_, i)| i).peekable());
            loop {
                let next = match (a.peek(), b.peek()) {
                    (Some(x), Some(y)) if x <= y => a.next(),
                    (Some(_), Some(_)) | (None, Some(_)) => b.next(),
                    (Some(_), None) => a.next(),
                    (None, None) => break,
                };
                if let Some(i) = next
                    && list.last() != Some(&i)
                {
                    list.push(i);
                }
            }
        }
    }

    /// Rebuild the cache from `layers` (one walk) and return it.
    pub(crate) fn rebuild(&self, layers: &[Arc<Node>]) -> Arc<Paths> {
        fn walk(nodes: &[Arc<Node>], parent: u32, out: &mut Table) {
            for (i, n) in nodes.iter().enumerate() {
                let e = out.at.len() as u32;
                out.at.push((parent, i as u32));
                out.ids.entry(n.id).and_modify(|x| *x = None).or_insert(Some(e));
                if let Some(ch) = n.children() {
                    walk(ch, e, out);
                }
            }
        }
        note_tree_walk();
        let mut table = Table::default();
        walk(layers, u32::MAX, &mut table);
        let out = Arc::new(Paths { table: Arc::new(table), gaps: Arc::default() });
        if let Ok(mut g) = self.0.write() {
            *g = Some(out.clone());
        }
        self.1.store(0, std::sync::atomic::Ordering::Relaxed);
        self.2.store(0, std::sync::atomic::Ordering::Relaxed);
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
        IdIndex(RwLock::new(self.get()), Default::default(), self.2.load(std::sync::atomic::Ordering::Relaxed).into())
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
