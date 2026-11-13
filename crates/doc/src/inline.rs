//! Inline graphics in text ([`crate::TextRun::inline`]): resolving each inline symbol's art bounds
//! from the document's symbols, which the text layout needs to size and place it.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock, Weak};

use vectorcraft_geom::{Affine, Rect};

use crate::{Document, Node, NodeId, NodeKind, Symbol, TextObject};

/// The document key under which the app records each symbol's natural size (`{name: [w, h]}`).
pub const SYMBOL_SIZES: &str = "symbolSizes";
/// Half the side of the square symbol art is normalised to.
pub const SYMBOL_HALF: f64 = 10.0;

/// Usable art bounds: finite, non-degenerate.
fn usable(r: Option<Rect>) -> Option<Rect> {
    r.filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) && r.width() >= 0.0 && r.height() > 1e-9)
}

impl Document {
    /// Symbol space → the symbol's art at its natural size. Symbols made in the app keep their
    /// art in a 20 × 20 square around the origin and their size as drawn in the document's
    /// `symbolSizes` (an instance is placed with that scale); other symbols are as they are.
    pub fn symbol_natural_xf(&self, name: &str) -> Affine {
        let size = self.unknown.get(SYMBOL_SIZES).and_then(|m| m.get(name)).and_then(|v| Some((v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)));
        match size {
            Some((w, h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => {
                Affine::scale_non_uniform(w / (2.0 * SYMBOL_HALF), h / (2.0 * SYMBOL_HALF))
            }
            _ => Affine::IDENTITY,
        }
    }

    /// Visual bounds of the art of the symbol named `name` at its natural size (see
    /// [`Self::symbol_natural_xf`]), if it exists and has a usable extent.
    pub fn symbol_art_bounds(&self, name: &str) -> Option<Rect> {
        let s = self.symbols.iter().find(|s| s.name == name)?;
        let b = usable(s.art.visual_bounds().or_else(|| s.art.geometric_bounds()))?;
        usable(Some(self.symbol_natural_xf(name).transform_rect_bbox(b)))
    }

    /// The art bounds an inline graphic of the symbol named `name` takes: [`Self::symbol_art_bounds`],
    /// or `None` (an empty slot) when the symbol's art shows the symbol itself inline, directly or
    /// through the inline graphics in other symbols' art, so that its size would depend on itself.
    pub fn inline_art_bounds(&self, name: &str) -> Option<Rect> {
        if self.inline_cycles().contains(name) { None } else { self.symbol_art_bounds(name) }
    }

    /// The names of the symbols that reach themselves through the inline graphics in symbol art,
    /// found once while the symbols stay as they are.
    fn inline_cycles(&self) -> Arc<HashSet<String>> {
        if let Some(found) = self.inline_cycles.get(&self.symbols) {
            return found;
        }
        let found = Arc::new(InlineGraph::of(self).cyclic);
        self.inline_cycles.set(&self.symbols, &found);
        found
    }

    /// Does `t` have inline graphics whose resolved bounds differ from the document's symbols?
    fn inline_stale(&self, t: &TextObject) -> bool {
        stale_by(t, &|n| self.inline_art_bounds(n))
    }

    /// `t` with its inline graphics' art bounds resolved from this document's symbols (borrowed
    /// when nothing changes). Renderers and exporters lay text out through this, so they never
    /// depend on the cached bounds being current.
    pub fn inline_resolved<'a>(&self, t: &'a TextObject) -> Cow<'a, TextObject> {
        if !self.inline_stale(t) {
            return Cow::Borrowed(t);
        }
        let mut t = t.clone();
        self.resolve_text_inline(&mut t);
        Cow::Owned(t)
    }

    /// Fill in the art bounds of `t`'s inline graphics ([`Self::inline_art_bounds`]); returns
    /// whether any changed (the text's cached layout is then cleared).
    pub fn resolve_text_inline(&self, t: &mut TextObject) -> bool {
        resolve_runs(t, &|n| self.inline_art_bounds(n))
    }

    /// Resolve the inline graphics of every text object in the document (after opening a document
    /// or changing its symbols), without laying the type out again: see
    /// [`Self::resolve_inline_art_with`].
    pub fn resolve_inline_art(&mut self) -> Vec<NodeId> {
        self.resolve_inline_art_with(&mut |_| {})
    }

    /// Resolve the inline graphics of every text object in the document: in the layers, the art
    /// of symbols, graph designs and pattern tiles, and the opacity masks and live objects in
    /// them. Each text whose inline graphics changed has its cached layout cleared and is passed
    /// to `layout` to be laid out again. The symbols are resolved first, each after the symbols its
    /// inline graphics show, so a symbol's art is measured with its type already laid out; a
    /// symbol that shows itself inline ([`Self::inline_art_bounds`]) is shown as an empty slot.
    /// Returns the ids of the texts in the layers that changed.
    pub fn resolve_inline_art_with(&mut self, layout: &mut dyn FnMut(&mut TextObject)) -> Vec<NodeId> {
        let mut out = vec![];
        let mut unused = vec![];
        let graph = InlineGraph::of(self);
        let measured = Measured::default();
        for &i in &graph.order {
            let mut art = self.symbols[i].art.clone();
            if resolve_tree(&mut art, &|n| measured.get(n, || graph.bounds(self, n)), layout, &mut unused) {
                self.symbols[i].art = art;
            }
        }
        for i in 0..self.layers.len() {
            let mut layer = self.layers[i].clone();
            if resolve_tree(&mut layer, &|n| measured.get(n, || graph.bounds(self, n)), layout, &mut out) {
                self.layers[i] = layer;
            }
        }
        for i in 0..self.graph_designs.len() {
            let mut art = self.graph_designs[i].art.clone();
            if resolve_tree(&mut art, &|n| measured.get(n, || graph.bounds(self, n)), layout, &mut unused) {
                self.graph_designs[i].art = art;
            }
        }
        for i in 0..self.patterns.len() {
            for j in 0..self.patterns[i].art.len() {
                let mut art = self.patterns[i].art[j].clone();
                if resolve_tree(&mut art, &|n| measured.get(n, || graph.bounds(self, n)), layout, &mut unused) {
                    self.patterns[i].art[j] = art;
                }
            }
        }
        out
    }
}

/// Whether `t` has inline graphics whose bounds differ from what `bounds` gives their symbols.
fn stale_by(t: &TextObject, bounds: &dyn Fn(&str) -> Option<Rect>) -> bool {
    t.runs.iter().filter_map(|r| r.inline.as_ref()).any(|a| a.bounds != bounds(&a.symbol))
}

/// Gives `t`'s inline graphics the bounds `bounds` gives their symbols; returns whether any changed
/// (the text's cached layout is then cleared).
fn resolve_runs(t: &mut TextObject, bounds: &dyn Fn(&str) -> Option<Rect>) -> bool {
    let mut changed = false;
    for a in t.runs.iter_mut().filter_map(|r| r.inline.as_mut()) {
        let b = bounds(&a.symbol);
        if a.bounds != b {
            a.bounds = b;
            changed = true;
        }
    }
    if changed {
        t.cached_bounds = None;
        t.cached_baselines.clear();
        t.cached_ink = None;
    }
    changed
}

/// Resolves the inline graphics of the type in tree `n` (opacity mask art included) with
/// `bounds`, passing each text that changed to `layout` and adding its id to `changed`; returns
/// whether the tree changed. Each node is visited once, and only the nodes on the way to changed
/// type are copied.
fn resolve_tree(
    n: &mut Arc<Node>,
    bounds: &dyn Fn(&str) -> Option<Rect>,
    layout: &mut dyn FnMut(&mut TextObject),
    changed: &mut Vec<NodeId>,
) -> bool {
    let mut copied = false;
    if matches!(&n.kind, NodeKind::Text(t) if stale_by(t, bounds)) {
        let node = Arc::make_mut(n);
        if let NodeKind::Text(t) = &mut node.kind
            && resolve_runs(t, bounds)
        {
            layout(t);
            changed.push(node.id);
        }
        copied = true;
    }
    let mask = n.extras().mask.as_deref().and_then(|m| {
        let mut art = m.art.clone();
        resolve_tree(&mut art, bounds, layout, changed).then_some(art)
    });
    let mut kids = vec![];
    for (i, c) in n.children().into_iter().flatten().enumerate() {
        let mut c = c.clone();
        if resolve_tree(&mut c, bounds, layout, changed) {
            kids.push((i, c));
        }
    }
    if mask.is_none() && kids.is_empty() {
        return copied;
    }
    let node = Arc::make_mut(n);
    if let Some(art) = mask
        && let Some(m) = node.extras_if_any().and_then(|e| e.mask.as_deref_mut())
    {
        m.art = art;
    }
    if let Some(ch) = node.children_mut() {
        for (i, c) in kids {
            ch[i] = c;
        }
    }
    true
}

/// The art bounds of each symbol, measured once in a pass of [`Document::resolve_inline_art_with`].
/// A symbol is first asked for after its own type is resolved (the pass goes in
/// [`InlineGraph::order`]), so its measurement stays current for the rest of the pass.
#[derive(Default)]
struct Measured(RefCell<HashMap<String, Option<Rect>>>);

impl Measured {
    /// The bounds kept for `name`, or `measure`'s, kept for later.
    fn get(&self, name: &str, measure: impl FnOnce() -> Option<Rect>) -> Option<Rect> {
        if let Some(&b) = self.0.borrow().get(name) {
            return b;
        }
        let b = measure();
        self.0.borrow_mut().insert(name.to_owned(), b);
        b
    }
}

/// The names of the symbols the inline graphics in tree `n` show (opacity mask art included).
fn inline_symbols(n: &Node) -> Vec<String> {
    fn collect(n: &Node, out: &mut Vec<String>) {
        if let NodeKind::Text(t) = &n.kind {
            out.extend(t.runs.iter().filter_map(|r| r.inline.as_ref().map(|a| a.symbol.clone())));
        }
        if let Some(m) = n.extras().mask.as_deref() {
            collect(&m.art, out);
        }
        for c in n.children().into_iter().flatten() {
            collect(c, out);
        }
    }
    let mut out = vec![];
    collect(n, &mut out);
    out
}

/// Each symbol name of `d` → the index of the first symbol of that name (the one a name finds).
fn symbol_index(d: &Document) -> HashMap<&str, usize> {
    let mut index = HashMap::with_capacity(d.symbols.len());
    for (i, s) in d.symbols.iter().enumerate() {
        index.entry(s.name.as_str()).or_insert(i);
    }
    index
}

/// The symbols of a document as their inline graphics depend on each other.
struct InlineGraph {
    /// Every symbol index, each after the symbols its art shows inline (except along a cycle).
    order: Vec<usize>,
    /// The names of the symbols that reach themselves through inline graphics.
    cyclic: HashSet<String>,
}

impl InlineGraph {
    /// The graph of `d`'s symbols: each symbol's art is walked once, and the order and cycles are
    /// found in time linear in the symbols and their inline references (Tarjan's strongly
    /// connected components, without recursion).
    fn of(d: &Document) -> Self {
        let index = symbol_index(d);
        let edges: Vec<Vec<usize>> = d
            .symbols
            .iter()
            .map(|s| {
                let mut e: Vec<usize> = inline_symbols(&s.art).iter().filter_map(|n| index.get(n.as_str()).copied()).collect();
                e.sort_unstable();
                e.dedup();
                e
            })
            .collect();
        let (order, cyclic) = components(&edges);
        let cyclic = order.iter().filter(|&&i| cyclic[i]).map(|&i| d.symbols[i].name.clone()).collect();
        Self { order, cyclic }
    }

    /// [`Document::inline_art_bounds`] with the cycles found in this graph.
    fn bounds(&self, d: &Document, name: &str) -> Option<Rect> {
        if self.cyclic.contains(name) { None } else { d.symbol_art_bounds(name) }
    }
}

/// The nodes of the graph `edges` (node → the nodes it depends on), each after the nodes it
/// depends on except within a cycle, and whether each node is on a cycle (a component of more than
/// one node, or an edge to itself).
fn components(edges: &[Vec<usize>]) -> (Vec<usize>, Vec<bool>) {
    const UNSEEN: usize = usize::MAX;
    let n = edges.len();
    let (mut index, mut low, mut on_stack) = (vec![UNSEEN; n], vec![0; n], vec![false; n]);
    let (mut stack, mut order, mut cyclic) = (vec![], Vec::with_capacity(n), vec![false; n]);
    let mut next = 0;
    // The nodes being visited, each with the position of its next edge.
    let mut calls: Vec<(usize, usize)> = vec![];
    for root in 0..n {
        if index[root] != UNSEEN {
            continue;
        }
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        calls.push((root, 0));
        while let Some(top) = calls.last_mut() {
            let v = top.0;
            if let Some(&w) = edges[v].get(top.1) {
                top.1 += 1;
                if index[w] == UNSEEN {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    calls.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(u, _)) = calls.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let start = order.len();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    order.push(w);
                    if w == v {
                        break;
                    }
                }
                if order.len() - start > 1 || edges[v].contains(&v) {
                    for &w in &order[start..] {
                        cyclic[w] = true;
                    }
                }
            }
        }
    }
    (order, cyclic)
}

/// The symbols [`Document::inline_cycles`] last found, with the name of each symbol and a weak
/// pointer to its art. While a weak pointer to an art's allocation is held, nothing else can be at
/// that address, and the art can't be changed in place (`Arc::get_mut` refuses it and
/// `Arc::make_mut` moves it), so any edit inside a symbol's art, and any symbol added, removed,
/// renamed or replaced, gives symbols the cache does not match. Clones of a document share it.
#[derive(Default)]
pub(crate) struct InlineCycles(RwLock<Option<Arc<Found>>>);

/// The name and art of each symbol the cycles were found in, and the names of the symbols on one.
type Found = (Vec<(String, Weak<Node>)>, Arc<HashSet<String>>);

impl InlineCycles {
    /// The cached names, when they were found in `symbols`.
    fn get(&self, symbols: &[Symbol]) -> Option<Arc<HashSet<String>>> {
        let g = self.0.read().ok()?;
        let (from, found) = &**g.as_ref()?;
        let same = from.len() == symbols.len()
            && from.iter().zip(symbols).all(|((name, art), s)| *name == s.name && std::ptr::eq(art.as_ptr(), Arc::as_ptr(&s.art)));
        same.then(|| found.clone())
    }

    /// Keep `found`, found in `symbols`.
    fn set(&self, symbols: &[Symbol], found: &Arc<HashSet<String>>) {
        if let Ok(mut g) = self.0.write() {
            *g = Some(Arc::new((symbols.iter().map(|s| (s.name.clone(), Arc::downgrade(&s.art))).collect(), found.clone())));
        }
    }
}

impl Clone for InlineCycles {
    fn clone(&self) -> Self {
        InlineCycles(RwLock::new(self.0.read().ok().and_then(|g| g.clone())))
    }
}

/// A cache, not document content: two documents are equal whatever their caches hold.
impl PartialEq for InlineCycles {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for InlineCycles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InlineCycles")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The position of each node in `order`.
    fn positions(order: &[usize]) -> Vec<usize> {
        let mut at = vec![0; order.len()];
        for (p, &i) in order.iter().enumerate() {
            at[i] = p;
        }
        at
    }

    #[test]
    fn components_put_dependencies_first_and_find_every_cycle_member() {
        // 0 → 1 → 2 → 0 is a cycle, and 3, reached from 1, joins it through 2.
        // 4 depends on 0 and is on no cycle; 5 shows itself.
        let edges = vec![vec![1], vec![2, 3], vec![0], vec![2], vec![0], vec![5]];
        let (order, cyclic) = components(&edges);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3, 4, 5]);
        assert_eq!(cyclic, [true, true, true, true, false, true]);
        let at = positions(&order);
        assert!(at[4] > at[0], "{order:?}");
        // A chain: each node after the one it depends on.
        let (order, cyclic) = components(&[vec![], vec![0], vec![1]]);
        assert_eq!(order, [0, 1, 2]);
        assert!(cyclic.iter().all(|c| !c));
    }

    /// Type at the bottom of 2,000 nested groups is checked once, not once for each group above it,
    /// and only the groups on the way to it are copied.
    #[test]
    fn resolving_deeply_nested_type_visits_each_node_once() {
        use std::cell::Cell;
        use vectorcraft_geom::Point;

        let mut t = TextObject::point(Point::ZERO, "Type", crate::CharStyle::default());
        t.runs.push(crate::TextRun::new("", crate::CharStyle::default()));
        t.runs[1].inline = Some(crate::InlineArt { symbol: "S".into(), scale: 1.0, baseline_shift: 0.0, bounds: None });
        let depth = 2_000;
        let mut tree = Arc::new(Node::new(NodeId(0), NodeKind::Text(Box::new(t))));
        for i in 1..=depth {
            let side = Arc::new(Node::group(NodeId(depth + i), vec![]));
            tree = Arc::new(Node::group(NodeId(i), vec![side, tree]));
        }
        let before = tree.clone();
        let asked = Cell::new(0);
        let r = Rect::new(0.0, 0.0, 10.0, 20.0);
        let bounds = |_: &str| {
            asked.set(asked.get() + 1);
            Some(r)
        };
        let (mut laid_out, mut changed) = (0, vec![]);
        assert!(resolve_tree(&mut tree, &bounds, &mut |_| laid_out += 1, &mut changed));
        assert!(asked.get() <= 2, "bounds asked {} times for one inline graphic", asked.get());
        assert_eq!(laid_out, 1);
        assert_eq!(changed, [NodeId(0)]);
        // The bottom of the new tree holds the resolved type; the side groups are shared.
        let (mut new, mut old) = (&tree, &before);
        while let (Some(n), Some(o)) = (new.children(), old.children()) {
            assert!(Arc::ptr_eq(&n[0], &o[0]));
            (new, old) = (&n[1], &o[1]);
        }
        let NodeKind::Text(t) = &new.kind else { panic!("the type is at the bottom") };
        assert_eq!(t.runs[1].inline.as_ref().and_then(|a| a.bounds), Some(r));
        // Nothing left to change: the tree is kept and the type is checked once.
        asked.set(0);
        let kept = tree.clone();
        assert!(!resolve_tree(&mut tree, &bounds, &mut |_| laid_out += 1, &mut changed));
        assert!(Arc::ptr_eq(&tree, &kept));
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn a_symbol_is_measured_once_in_a_pass() {
        let measured = Measured::default();
        let count = std::cell::Cell::new(0);
        let measure = |b: Option<Rect>| {
            count.set(count.get() + 1);
            b
        };
        let r = Some(Rect::new(0.0, 0.0, 1.0, 1.0));
        for _ in 0..3 {
            assert_eq!(measured.get("A", || measure(r)), r);
            assert_eq!(measured.get("Missing", || measure(None)), None);
        }
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn components_handle_a_long_chain_without_recursion() {
        let n = 200_000;
        let edges: Vec<Vec<usize>> = (0..n).map(|i| if i + 1 < n { vec![i + 1] } else { vec![] }).collect();
        let (order, cyclic) = components(&edges);
        assert_eq!(order.first(), Some(&(n - 1)));
        assert_eq!(order.last(), Some(&0));
        assert!(cyclic.iter().all(|c| !c));
    }
}
