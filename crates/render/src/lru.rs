//! Caches bounded by frames and by bytes.
//!
//! A renderer keeps what it works out per object between frames (glyph outlines, clip regions,
//! brush art, evaluated blends…). Each entry records the frame that last used it and roughly how
//! many bytes it holds. [`FrameCache::sweep`] (at the end of every frame) drops entries unused for
//! a few frames once the cache holds more than a handful, so the nodes they keep alive go with
//! them; [`FrameCache::insert`] keeps the total under a byte budget by dropping the least recently
//! used entries of earlier frames.
//!
//! Entries used in the frame being drawn are never dropped for a new one: when they alone fill the
//! budget the new entry isn't kept (it is worked out again when next needed). A document needing
//! more than the budget then keeps the same entries frame after frame and works out only the rest
//! again, where dropping everything (or the oldest entry, in a least-recently-used order) whenever
//! the cache filled worked out every entry again every frame.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::mem::size_of;

use vectorcraft_doc::Node;
use vectorcraft_geom::{Anchor, BezPath, SubPath};

use crate::PtrHasher;

/// Glyph outlines per text object (`Renderer::text_geom_of`).
pub(crate) const TEXT_BUDGET: usize = 128 << 20;
/// Clip regions per clipping path (`Renderer::clip_of`), and separately what clipping paths
/// paint (`Renderer::clip_paint_of`).
pub(crate) const CLIP_BUDGET: usize = 32 << 20;
/// The art drawn for objects with effects that apply through it (`Renderer::fx_art`).
pub(crate) const FX_ART_BUDGET: usize = 64 << 20;
/// Symbol art placed for instances and inline graphics (`Renderer::placed_art`).
pub(crate) const PLACED_ART_BUDGET: usize = 64 << 20;
/// Brush art per brushed stroke (`Renderer::draw_brush`).
pub(crate) const BRUSH_BUDGET: usize = 64 << 20;
/// Evaluated blends, envelopes and repeats (`Renderer::live_expanded`).
pub(crate) const LIVE_BUDGET: usize = 64 << 20;
/// Tessellated gradient meshes (`Renderer::mesh_quads`).
pub(crate) const MESH_BUDGET: usize = 32 << 20;
/// Pattern tile rasters, per thread and ink plane (`pattern::set_pattern_paint`): a tile is at
/// most 4096² pixels, 64 MiB.
pub(crate) const PATTERN_BUDGET: usize = 128 << 20;
/// Freeform gradient grids, per thread (`freeform::set_freeform_paint`).
pub(crate) const FREEFORM_BUDGET: usize = 32 << 20;

/// Frames an entry may go unused before [`FrameCache::sweep`] drops it.
const AGE: u64 = 3;

/// Bookkeeping of a node beyond its own struct: its allocation, `Arc` counts, appearance.
const NODE_OVERHEAD: usize = 128;

pub(crate) type PtrBuild = std::hash::BuildHasherDefault<PtrHasher>;

struct Slot<V> {
    value: V,
    bytes: usize,
    /// Frame (or, for caches without frames, request) that last used it.
    stamp: u64,
    /// [`FrameCache::clock`] when last used: the order entries are dropped in.
    used: u64,
}

/// Entries swept by frame and kept within a byte budget (see the module docs).
pub(crate) struct FrameCache<K, V, S = PtrBuild> {
    map: HashMap<K, Slot<V>, S>,
    /// Sum of the entries' sizes.
    bytes: usize,
    budget: usize,
    /// Entries kept whatever their age.
    soft: usize,
    /// Request counter.
    clock: u64,
    /// The frame whose own entries were found to fill the budget: nothing more is kept in it.
    full: Option<u64>,
}

impl<K: Hash + Eq + Clone, V, S: BuildHasher + Default> FrameCache<K, V, S> {
    /// A cache holding at most `budget` bytes, sweeping entries of past frames beyond `soft` entries.
    pub(crate) fn new(budget: usize, soft: usize) -> Self {
        Self { map: HashMap::default(), bytes: 0, budget, soft, clock: 0, full: None }
    }

    /// The entry for `k`, marked as used in frame `stamp`.
    pub(crate) fn get<Q>(&mut self, k: &Q, stamp: u64) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.clock += 1;
        let s = self.map.get_mut(k)?;
        (s.stamp, s.used) = (stamp, self.clock);
        Some(&s.value)
    }

    /// Keep `value` (about `bytes` bytes) as `k`, used in frame `stamp`, replacing any entry for
    /// `k`. Least recently used entries of earlier frames make room; returns false when they can't
    /// (the value isn't kept then).
    pub(crate) fn insert(&mut self, k: K, value: V, bytes: usize, stamp: u64) -> bool {
        self.clock += 1;
        if let Some(old) = self.map.remove(&k) {
            self.bytes -= old.bytes;
        }
        if self.bytes + bytes > self.budget {
            if bytes > self.budget || self.full == Some(stamp) {
                return false;
            }
            // Make room down to seven eighths of the budget, so the next few don't search again.
            let target = self.budget - self.budget / 8;
            let mut old: Vec<(u64, K)> = self.map.iter().filter(|(_, s)| s.stamp < stamp).map(|(k, s)| (s.used, k.clone())).collect();
            old.sort_unstable_by_key(|(used, _)| *used);
            for (_, key) in old {
                if self.bytes + bytes <= target {
                    break;
                }
                if let Some(s) = self.map.remove(&key) {
                    self.bytes -= s.bytes;
                }
            }
            if self.bytes + bytes > self.budget {
                self.full = Some(stamp);
                return false;
            }
        }
        self.bytes += bytes;
        self.map.insert(k, Slot { value, bytes, stamp, used: self.clock });
        true
    }

    /// End of frame `stamp`: past [`Self::new`]'s `soft` entries, drop those unused for a few frames.
    pub(crate) fn sweep(&mut self, stamp: u64) {
        if self.map.len() > self.soft {
            let bytes = &mut self.bytes;
            self.map.retain(|_, s| {
                let keep = stamp.saturating_sub(s.stamp) <= AGE;
                if !keep {
                    *bytes -= s.bytes;
                }
                keep
            });
        }
    }

    /// Drop the entries whose values `keep` returns false for.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&V) -> bool) {
        let bytes = &mut self.bytes;
        self.map.retain(|_, s| {
            let kept = keep(&s.value);
            if !kept {
                *bytes -= s.bytes;
            }
            kept
        });
    }

    /// A new stamp for caches without frames: each request its own, so every other entry may make
    /// room for it (least recently used first).
    pub(crate) fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
        self.full = None;
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The bytes the entries hold (as estimated when they were kept).
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn contains_key<Q>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(k)
    }
}

/// Heap bytes of a path's elements.
pub(crate) fn path_bytes(p: &BezPath) -> usize {
    size_of_val(p.elements())
}

/// Approximate bytes of node `n` and its members: the nodes and their path anchors.
pub(crate) fn node_bytes(n: &Node) -> usize {
    let mut b = 0;
    n.walk(&mut |c| {
        b += size_of::<Node>() + NODE_OVERHEAD;
        if let Some(p) = c.path_data() {
            b += p.subpaths.iter().map(|s| size_of::<SubPath>() + s.anchors.len() * size_of::<Anchor>()).sum::<usize>();
        }
    });
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    type Cache = FrameCache<usize, &'static str>;

    #[test]
    fn over_budget_drops_the_least_recently_used_entries_of_past_frames() {
        let mut c = Cache::new(100, 0);
        for k in 0..10 {
            assert!(c.insert(k, "x", 10, 1));
        }
        assert_eq!(c.bytes(), 100);
        // Frame 2 uses 0..5 (so 5..10 are now the least recently used), then needs room.
        for k in 0..5 {
            assert!(c.get(&k, 2).is_some());
        }
        assert!(c.insert(10, "y", 10, 2));
        assert!((0..5).all(|k| c.contains_key(&k)) && c.contains_key(&10), "recently used entries stay");
        assert!(c.bytes() <= 100 && c.bytes() < 110, "bytes went down: {}", c.bytes());
        assert!(!c.contains_key(&5), "the least recently used entry went first");
    }

    #[test]
    fn entries_of_the_frame_being_drawn_stay() {
        let mut c = Cache::new(100, 0);
        // More than the budget, every frame: the first entries stay and only the rest are redone.
        for frame in 1..4 {
            let mut hits = 0;
            for k in 0..20 {
                if c.get(&k, frame).is_some() {
                    hits += 1;
                } else {
                    c.insert(k, "x", 10, frame);
                }
            }
            assert_eq!(c.len(), 10);
            assert_eq!(hits, if frame == 1 { 0 } else { 10 }, "frame {frame}");
        }
        // Too large for the budget: not kept.
        assert!(!c.insert(99, "big", 1000, 5));
    }

    #[test]
    fn sweep_drops_old_entries_past_the_soft_limit() {
        let mut c = Cache::new(1000, 2);
        for k in 0..4 {
            c.insert(k, "x", 10, 1);
        }
        assert!(c.get(&0, 5).is_some());
        c.sweep(5);
        assert_eq!((c.len(), c.bytes()), (1, 10));
        c.sweep(100);
        assert_eq!(c.len(), 1, "within the soft limit nothing goes");
        let t = c.tick();
        assert!(c.insert(7, "y", 10, t));
        c.clear();
        assert!(c.is_empty() && c.bytes() == 0);
    }
}
