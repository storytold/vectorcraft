//! How much memory a document keeps that another doesn't share: what an undo step costs.
//!
//! Documents share structure (`Arc<Node>` children, [`crate::Shared`] resources), so an old
//! version kept in the undo history holds only what the edits after it replaced. The estimate
//! walks two versions side by side and visits only what isn't shared: a subtree whose `Arc` both
//! hold costs nothing and isn't entered, so the walk is about as long as the edit was (plus a pass
//! over each changed child list). One walk gives both directions: what the older version keeps
//! that the newer one doesn't (the undo step's cost) and the other way round (the cost of the redo
//! step that undoing it makes), so undo and redo needn't walk again. It counts the heap the values
//! own (struct sizes, vector capacities, strings, JSON) plus a typical allocator overhead per
//! allocation, without following every last field: an estimate in the right ballpark, for a
//! memory budget, not an exact figure.

use std::collections::HashSet;
use std::mem::size_of;
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_color::{Paint, Swatch, SwatchGroup};
use vectorcraft_geom::{Affine, Anchor, PathData, SubPath};

use crate::live::{BlendSpec, EnvelopeKind};
use crate::text::{ParaStyle, TextObject, TextRun, TextStyleDef};
use crate::{Appearance, AppearanceItem, Document, Effect, GraphicStyle, ImageObject, Node, NodeExtras, NodeKind, SavedSelection, Shared};

/// Bookkeeping and rounding a typical allocator adds to each allocation.
const ALLOC: usize = 16;
/// An `Arc`'s two reference counts.
const ARC: usize = 16;
/// The kinds of node this doesn't break down (meshes, repeats' settings, placed documents…): a
/// typical size of what they own besides the node.
const OTHER_KIND: usize = 256;

impl Document {
    /// Roughly how many bytes of heap this document keeps that `newer` doesn't share with it: what
    /// keeping this version in the undo history costs while `newer` (a later version) is kept too.
    /// Visits only the parts that aren't shared (see the [module docs](self)).
    pub fn retained_bytes(&self, newer: &Document) -> usize {
        self.retained_bytes_each(newer).0
    }

    /// [`Document::retained_bytes`] both ways in one walk: what this document keeps that `other`
    /// doesn't share, and what `other` keeps that this one doesn't.
    pub fn retained_bytes_each(&self, other: &Document) -> (usize, usize) {
        if std::ptr::eq(self, other) {
            return (0, 0);
        }
        let own = size_of::<Document>() + ALLOC + ARC;
        let mut w = Walk { docs: [self, other], seen: [None, None], bytes: [own, own] };
        w.list(&self.layers, &other.layers);
        for i in 0..self.symbols.len().max(other.symbols.len()) {
            match (self.symbols.get(i), other.symbols.get(i)) {
                (Some(a), Some(b)) => w.pair(&a.art, &b.art),
                (a, b) => {
                    a.into_iter().for_each(|a| w.alone(0, &a.art));
                    b.into_iter().for_each(|b| w.alone(1, &b.art));
                }
            }
        }
        for i in 0..self.patterns.len().max(other.patterns.len()) {
            match (self.patterns.get(i), other.patterns.get(i)) {
                (Some(a), Some(b)) => w.list(&a.art, &b.art),
                (a, b) => {
                    a.into_iter().flat_map(|a| &a.art).for_each(|n| w.alone(0, n));
                    b.into_iter().flat_map(|b| &b.art).for_each(|n| w.alone(1, n));
                }
            }
        }
        let [mut a, mut b] = w.bytes;
        a += self.lists_bytes() + self.resources_bytes(other);
        b += other.lists_bytes() + other.resources_bytes(self);
        (a, b)
    }

    /// The symbol and pattern lists themselves (their art is counted as nodes).
    fn lists_bytes(&self) -> usize {
        vec_bytes(&self.symbols)
            + self.symbols.iter().map(|s| s.name.capacity()).sum::<usize>()
            + vec_bytes(&self.patterns)
            + self.patterns.iter().map(|p| p.name.capacity()).sum::<usize>()
    }

    /// The document-wide lists and settings: a copy of the document copies those that aren't
    /// [`Shared`]; the shared ones count only when `newer` has its own.
    fn resources_bytes(&self, newer: &Document) -> usize {
        fn shared<T>(a: &Shared<T>, b: &Shared<T>, f: impl FnOnce(&T) -> usize) -> usize {
            if Shared::ptr_eq(a, b) { 0 } else { ALLOC + ARC + f(a) }
        }
        let mut n = 0;
        n += vec_bytes(&self.artboards) + self.artboards.iter().map(|a| a.name.capacity()).sum::<usize>();
        n += vec_bytes(&self.guides) + vec_bytes(&self.views) + vec_bytes(&self.slices) + vec_bytes(&self.assets);
        n += vec_bytes(&self.text_threads) + self.text_threads.iter().map(vec_bytes).sum::<usize>();
        n += self.images.iter().map(|(k, b)| 4 * size_of::<usize>() + k.capacity() + b.mime.capacity() + size_of_val(b)).sum::<usize>();
        n += map_bytes(&self.export_settings) + map_bytes(&self.extra) + self.print_setup.as_ref().map_or(0, json_bytes);
        n += self.title.capacity();
        n += shared(&self.swatches, &newer.swatches, |s| swatches_bytes(s));
        n += shared(&self.swatch_groups, &newer.swatch_groups, |g| {
            vec_bytes(g) + g.iter().map(|g: &SwatchGroup| g.name.capacity() + swatches_bytes(&g.swatches)).sum::<usize>()
        });
        n += shared(&self.graphic_styles, &newer.graphic_styles, |g| {
            vec_bytes(g) + g.iter().map(|g: &GraphicStyle| g.name.capacity() + appearance_heap(&g.appearance)).sum::<usize>()
        });
        let styles = |s: &Vec<TextStyleDef>| vec_bytes(s) + s.iter().map(|s| s.name.capacity() + map_bytes(&s.attrs)).sum::<usize>();
        n += shared(&self.char_styles, &newer.char_styles, styles);
        n += shared(&self.para_styles, &newer.para_styles, styles);
        n += shared(&self.saved_selections, &newer.saved_selections, |s| {
            vec_bytes(s) + s.iter().map(|s: &SavedSelection| s.name.capacity() + vec_bytes(&s.objects)).sum::<usize>()
        });
        n += shared(&self.variables, &newer.variables, |v| {
            // Map entries: the key and value plus a share of the tree's nodes.
            let entry = 2 * size_of::<usize>();
            vec_bytes(&v.variables)
                + v.variables.iter().map(|v| v.name.capacity()).sum::<usize>()
                + vec_bytes(&v.datasets)
                + v.datasets.iter().map(|d| d.name.capacity() + d.values.keys().map(|k| entry + k.capacity() + 32).sum::<usize>()).sum::<usize>()
                + v.bindings.values().map(|b| entry + 8 + b.iter().map(|s| entry + s.capacity()).sum::<usize>()).sum::<usize>()
        });
        n += shared(&self.unknown, &newer.unknown, map_bytes);
        n
    }
}

/// One estimate in progress: two documents walked side by side.
struct Walk<'a> {
    docs: [&'a Document; 2],
    /// For each side, every node the other document holds (by address), built the first time a
    /// node of that side that the other may share has no counterpart in place: then a node moved
    /// elsewhere (grouped, say) is still found shared.
    seen: [Option<HashSet<usize>>; 2],
    /// For each side, what it keeps that the other doesn't.
    bytes: [usize; 2],
}

impl Walk<'_> {
    /// Two versions of a child list: the nodes in the same place on both sides (the same `Arc`, or
    /// versions of the same object) are compared in place; the others (inserted, deleted, moved)
    /// may still be shared from elsewhere.
    fn list(&mut self, a: &[Arc<Node>], b: &[Arc<Node>]) {
        let same = |x: &Arc<Node>, y: &Arc<Node>| Arc::ptr_eq(x, y) || x.id == y.id;
        // In place from the start and from the end: an insertion or deletion shifts only what is
        // between.
        let head = a.iter().zip(b).take_while(|(x, y)| same(x, y)).count();
        let (a_rest, b_rest) = (a.get(head..).unwrap_or_default(), b.get(head..).unwrap_or_default());
        let tail = a_rest.iter().rev().zip(b_rest.iter().rev()).take_while(|(x, y)| same(x, y)).count();
        for (x, y) in a.iter().zip(b).take(head).chain(a_rest.iter().rev().zip(b_rest.iter().rev()).take(tail)) {
            self.pair(x, y);
        }
        let a_mid = a_rest.get(..a_rest.len().saturating_sub(tail)).unwrap_or_default();
        let b_mid = b_rest.get(..b_rest.len().saturating_sub(tail)).unwrap_or_default();
        for i in 0..a_mid.len().max(b_mid.len()) {
            match (a_mid.get(i), b_mid.get(i)) {
                (Some(x), Some(y)) if same(x, y) => self.pair(x, y),
                (x, y) => {
                    x.into_iter().for_each(|x| self.alone(0, x));
                    y.into_iter().for_each(|y| self.alone(1, y));
                }
            }
        }
    }

    /// Node `a` and `b` in the same place on the two sides: nothing when they are the same `Arc`,
    /// both nodes and what under them isn't shared when they are versions of the same object.
    fn pair(&mut self, a: &Arc<Node>, b: &Arc<Node>) {
        if Arc::ptr_eq(a, b) {
            return;
        }
        if a.id != b.id {
            self.alone(0, a);
            self.alone(1, b);
            return;
        }
        self.bytes[0] += node_bytes(a, Some(b));
        self.bytes[1] += node_bytes(b, Some(a));
        match (a.children(), b.children()) {
            (Some(x), Some(y)) => self.list(x, y),
            (x, y) => {
                x.into_iter().flatten().for_each(|c| self.alone(0, c));
                y.into_iter().flatten().for_each(|c| self.alone(1, c));
            }
        }
        match (&a.extras().mask, &b.extras().mask) {
            (Some(x), Some(y)) => self.pair(&x.art, &y.art),
            (x, y) => {
                x.iter().for_each(|m| self.alone(0, &m.art));
                y.iter().for_each(|m| self.alone(1, &m.art));
            }
        }
    }

    /// Node `n` of side `side` without a counterpart in place, and what under it the other side
    /// doesn't hold.
    fn alone(&mut self, side: usize, n: &Arc<Node>) {
        if self.other_holds(side, n) {
            return;
        }
        if let Some(b) = self.bytes.get_mut(side) {
            *b += node_bytes(n, None);
        }
        for c in n.children().into_iter().flatten() {
            self.alone(side, c);
        }
        if let Some(m) = &n.extras().mask {
            self.alone(side, &m.art);
        }
    }

    /// Whether the document across from `side` holds node `n` somewhere.
    fn other_holds(&mut self, side: usize, n: &Arc<Node>) -> bool {
        // Held only where it was found: not in the other document (which would hold it too). So
        // new art inserted by the edit is told apart without looking.
        if Arc::strong_count(n) == 1 {
            return false;
        }
        let (Some(other), Some(seen)) = (self.docs.get(1 - side.min(1)), self.seen.get_mut(side)) else { return false };
        let seen = seen.get_or_insert_with(|| {
            fn add(n: &Arc<Node>, out: &mut HashSet<usize>) {
                // A node already seen was entered already (a node shared within the document).
                if !out.insert(Arc::as_ptr(n) as usize) {
                    return;
                }
                for c in n.children().into_iter().flatten() {
                    add(c, out);
                }
                if let Some(m) = &n.extras().mask {
                    add(&m.art, out);
                }
            }
            let mut out = HashSet::new();
            let roots = other.layers.iter().chain(other.symbols.iter().map(|s| &s.art)).chain(other.patterns.iter().flat_map(|p| &p.art));
            for n in roots {
                add(n, &mut out);
            }
            out
        });
        seen.contains(&(Arc::as_ptr(n) as usize))
    }
}

/// One node's own heap: the node, and what it owns besides its children and mask art (counted
/// as nodes of their own). Its appearance counts unless `other`, its other version, shares it.
fn node_bytes(n: &Node, other: Option<&Node>) -> usize {
    fn boxed<T>(b: &Option<Box<T>>) -> usize {
        b.as_ref().map_or(0, |_| ALLOC + size_of::<T>())
    }
    let mut b = ALLOC + ARC + size_of::<Node>();
    b += n.name.as_ref().map_or(0, String::capacity);
    if !other.is_some_and(|m| Shared::ptr_eq(&m.appearance, &n.appearance)) {
        b += ALLOC + ARC + appearance_heap(&n.appearance);
    }
    if let Some(e) = n.extras_box() {
        b += ALLOC + size_of::<NodeExtras>();
        b += boxed(&e.mask) + boxed(&e.graph) + boxed(&e.shaper) + boxed(&e.attrs) + boxed(&e.slice) + boxed(&e.perspective);
        b += boxed(&e.dimension) + boxed(&e.live_paint_gaps);
        b += e.trace.as_ref().map_or(0, |t| ALLOC + size_of::<Value>() + json_bytes(t));
        b += e.intertwine.as_deref().map_or(0, |a| ALLOC + size_of::<PathData>() + path_bytes(a));
        b += e.objects_on_path.as_deref().map_or(0, |o| {
            ALLOC + size_of::<crate::objects_on_path::ObjectsOnPath>() + o.members.capacity() * size_of::<crate::objects_on_path::Member>()
        });
    }
    b += match &n.kind {
        NodeKind::Path { path, live, .. } => path_bytes(path) + boxed(live),
        NodeKind::Layer { children, .. }
        | NodeKind::Group { children, .. }
        | NodeKind::Compound { children, .. }
        | NodeKind::Intertwine { children } => vec_bytes(children),
        NodeKind::Blend { children, .. } => vec_bytes(children) + ALLOC + size_of::<BlendSpec>(),
        NodeKind::Envelope { content, .. } => vec_bytes(content) + 2 * ALLOC + size_of::<EnvelopeKind>() + size_of::<Affine>(),
        NodeKind::Repeat(r) => vec_bytes(&r.source) + OTHER_KIND,
        NodeKind::Text(t) => ALLOC + text_bytes(t),
        NodeKind::Image(im) => ALLOC + size_of::<ImageObject>() + im.key.capacity(),
        NodeKind::SymbolInstance { symbol, .. } => symbol.capacity(),
        _ => OTHER_KIND,
    };
    b
}

fn path_bytes(p: &PathData) -> usize {
    let anchors: usize =
        p.subpaths.iter().map(|s| if s.anchors.capacity() == 0 { 0 } else { ALLOC + s.anchors.capacity() * size_of::<Anchor>() }).sum();
    anchors + if p.subpaths.capacity() == 0 { 0 } else { ALLOC + p.subpaths.capacity() * size_of::<SubPath>() }
}

fn text_bytes(t: &TextObject) -> usize {
    size_of::<TextObject>()
        + vec_bytes(&t.runs)
        + t.runs.iter().map(|r: &TextRun| r.text.capacity()).sum::<usize>()
        + t.paras.capacity() * size_of::<ParaStyle>()
        + vec_bytes(&t.wrap)
        + vec_bytes(&t.cached_baselines)
}

/// The heap an appearance owns (not the appearance itself).
fn appearance_heap(a: &Appearance) -> usize {
    size_of::<Appearance>()
        + vec_bytes(&a.items)
        + a.items
            .iter()
            .map(|i| match i {
                AppearanceItem::Fill(f) => paint_heap(&f.paint),
                AppearanceItem::Stroke(s) => paint_heap(&s.paint),
            })
            .sum::<usize>()
        + vec_bytes(&a.effects)
        + a.effects.iter().map(|e: &Effect| e.id.capacity() + json_bytes(&e.params)).sum::<usize>()
}

fn paint_heap(p: &Paint) -> usize {
    match p {
        Paint::Solid { swatch, .. } => swatch.as_ref().map_or(0, String::capacity),
        Paint::Gradient(g) => ALLOC + size_of_val(&**g) + vec_bytes(&g.gradient.stops),
        Paint::Pattern { pattern, .. } => pattern.capacity(),
        Paint::None => 0,
    }
}

fn swatches_bytes(s: &[Swatch]) -> usize {
    ALLOC + std::mem::size_of_val(s) + s.iter().map(|w| w.name.capacity() + paint_heap(&w.paint)).sum::<usize>()
}

/// A vector's buffer (not what its elements own).
fn vec_bytes<T>(v: &Vec<T>) -> usize {
    if v.capacity() == 0 { 0 } else { ALLOC + v.capacity() * size_of::<T>() }
}

fn map_bytes(m: &serde_json::Map<String, Value>) -> usize {
    m.iter().map(|(k, v)| ALLOC + 4 * size_of::<usize>() + k.capacity() + size_of::<Value>() + json_bytes(v)).sum()
}

/// The heap a JSON value owns.
fn json_bytes(v: &Value) -> usize {
    match v {
        Value::String(s) => s.capacity(),
        Value::Array(a) => vec_bytes(a) + a.iter().map(json_bytes).sum::<usize>(),
        Value::Object(m) => map_bytes(m),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::shapes;

    fn doc_with_rects(n: usize) -> Document {
        let mut d = Document::new(1000.0, 1000.0);
        let layer = d.layers[0].id;
        for i in 0..n {
            let id = d.alloc_id();
            let r = shapes::rectangle(kurbo::Rect::new(i as f64, 0.0, i as f64 + 10.0, 10.0));
            d.insert(Some(layer), usize::MAX, Node::path(id, r, Appearance::default_art())).unwrap();
        }
        d
    }

    #[test]
    fn an_unchanged_copy_keeps_nothing_of_its_own() {
        let d = doc_with_rects(10);
        let copy = d.clone();
        // Only the document itself and its own lists: no node, no shared resource.
        assert!(d.retained_bytes(&copy) < 2048, "{}", d.retained_bytes(&copy));
        assert_eq!(d.retained_bytes(&d), 0);
    }

    /// Moving every rectangle: each keeps its old node and anchors (its appearance stays shared).
    /// The process grew by about 775 bytes per moved rectangle when measured (#1244), about 490
    /// once nodes kept their rarely set parts out of line.
    #[test]
    fn moving_rectangles_costs_their_nodes() {
        let n = 1000;
        let before = doc_with_rects(n);
        let mut after = before.clone();
        let ids: Vec<_> = after.layers[0].children().unwrap().iter().map(|c| c.id).collect();
        for id in ids {
            let node = after.node_mut(id).unwrap();
            if let NodeKind::Path { path, .. } = &mut node.kind {
                for a in path.subpaths.iter_mut().flat_map(|s| s.anchors.iter_mut()) {
                    a.p.x += 5.0;
                }
            }
        }
        let per = before.retained_bytes(&after) / n;
        assert!((400..1200).contains(&per), "{per} bytes per moved rectangle");
    }

    #[test]
    fn a_grouped_object_is_found_shared() {
        let before = doc_with_rects(100);
        let mut after = before.clone();
        let ids: Vec<_> = after.layers[0].children().unwrap().iter().map(|c| c.id).collect();
        // Put the rectangles in a group (the same Arcs, elsewhere).
        let gid = after.alloc_id();
        let layer = Arc::make_mut(&mut after.layers[0]);
        let kids = std::mem::take(layer.children_mut().unwrap());
        layer.children_mut().unwrap().push(Arc::new(Node::group(gid, kids)));
        assert_eq!(ids.len(), 100);
        let bytes = before.retained_bytes(&after);
        assert!(bytes < 8192, "only the old layer and its child list: {bytes}");
    }

    /// One walk gives both ways, each what a walk the other way gives; an insertion and a deletion
    /// cost the side that has the objects.
    #[test]
    fn both_ways_at_once() {
        let before = doc_with_rects(100);
        let mut after = before.clone();
        let layer = after.layers[0].id;
        for at in [0, 50, usize::MAX] {
            let id = after.alloc_id();
            let r = shapes::rectangle(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0));
            after.insert(Some(layer), at, Node::path(id, r, Appearance::default_art())).unwrap();
        }
        let (kept, added) = before.retained_bytes_each(&after);
        assert_eq!((kept, added), (before.retained_bytes(&after), after.retained_bytes(&before)));
        assert_eq!(after.retained_bytes_each(&before), (added, kept));
        // The old layer and its list are the old version's own; the new one holds three new nodes.
        assert!(kept < 4096, "{kept}");
        assert!(added > kept + 3 * 300, "{added} for three new rectangles");
    }

    #[test]
    fn a_resource_counts_only_when_changed() {
        let before = doc_with_rects(1);
        let mut after = before.clone();
        let same = before.retained_bytes(&after);
        after.swatches.push(before.swatches[1].clone());
        assert!(before.retained_bytes(&after) > same + 1000, "the old swatch list is the old version's own");
    }
}
