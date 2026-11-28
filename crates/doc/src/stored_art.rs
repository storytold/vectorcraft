//! Every object a document stores, wherever it is kept: in the layers, in opacity-mask art, and in
//! the art of symbols, graph designs and pattern swatches. [`Document::node`] finds objects in the
//! layer tree only; the lookups here find the others by their ids too, which
//! [`Document::fix_next_id`] keeps unique across all of them.

use std::collections::HashSet;
use std::sync::Arc;

use super::limits::MAX_NEXT_ID;
use super::{Document, Node, NodeId};

/// The art a stored object belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredRoot {
    /// Layer `index` of [`Document::layers`].
    Layer(usize),
    /// Symbol `index` of [`Document::symbols`].
    Symbol(usize),
    /// Graph design `index` of [`Document::graph_designs`].
    GraphDesign(usize),
    /// Object `art` of pattern swatch `pattern` of [`Document::patterns`].
    Pattern { pattern: usize, art: usize },
    /// Root `index` of [`Document::mounted_art`].
    Mounted(usize),
}

/// Where a stored object is: its root and, inside opacity-mask art, the object that mask belongs
/// to (the innermost one when masks hold masks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredPlace {
    pub root: StoredRoot,
    pub mask_of: Option<NodeId>,
}

impl StoredPlace {
    /// Is the object in the layer tree itself, where [`Document::node`] finds it and the canvas
    /// draws it as itself?
    pub fn in_layers(self) -> bool {
        matches!(self.root, StoredRoot::Layer(_)) && self.mask_of.is_none()
    }
}

/// One step down from a node: to its child at an index, or to its opacity-mask art.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Child(usize),
    Mask,
}

/// `n` and everything below it with its place, the opacity-mask art right after the object it
/// masks, then the children.
fn visit<'a>(n: &'a Arc<Node>, place: StoredPlace, f: &mut dyn FnMut(&'a Arc<Node>, StoredPlace)) {
    f(n, place);
    if let Some(m) = &n.extras().mask {
        visit(&m.art, StoredPlace { mask_of: Some(n.id), ..place }, f);
    }
    for c in n.children().into_iter().flatten() {
        visit(c, place, f);
    }
}

/// The steps from `n` to the first node in [`visit`] order with id `id`, and the object whose mask
/// holds it last on the way.
fn find(n: &Node, id: NodeId, steps: &mut Vec<Step>, mask_of: &mut Option<NodeId>) -> bool {
    if n.id == id {
        return true;
    }
    if let Some(m) = &n.extras().mask {
        steps.push(Step::Mask);
        let before = *mask_of;
        *mask_of = Some(n.id);
        if find(&m.art, id, steps, mask_of) {
            return true;
        }
        *mask_of = before;
        steps.pop();
    }
    for (i, c) in n.children().into_iter().flatten().enumerate() {
        steps.push(Step::Child(i));
        if find(c, id, steps, mask_of) {
            return true;
        }
        steps.pop();
    }
    false
}

/// The node `steps` below `n`.
fn at<'a>(mut n: &'a Arc<Node>, steps: &[Step]) -> Option<&'a Arc<Node>> {
    for s in steps {
        n = match s {
            Step::Mask => &n.extras().mask.as_ref()?.art,
            Step::Child(i) => n.children()?.get(*i)?,
        };
    }
    Some(n)
}

/// The node `steps` below `n` to change, the nodes on the way copied when shared.
fn at_mut<'a>(mut n: &'a mut Arc<Node>, steps: &[Step]) -> Option<&'a mut Node> {
    for s in steps {
        let node = Arc::make_mut(n);
        n = match s {
            Step::Mask => &mut node.extras_if_any()?.mask.as_mut()?.art,
            Step::Child(i) => node.children_mut()?.get_mut(*i)?,
        };
    }
    Some(Arc::make_mut(n))
}

/// Gives every node from `n` down that `fresh` names a new id from `next`, counting nodes in
/// [`visit`] order from `*count`; only the nodes on the way to one are copied. Each part below is
/// renumbered through a second handle, so a new pointer marks a change.
fn renumber(n: &mut Arc<Node>, fresh: &HashSet<usize>, count: &mut usize, next: &mut u64) {
    let own = fresh.contains(count);
    *count += 1;
    let mask = n.extras().mask.as_ref().and_then(|m| {
        let mut art = Arc::clone(&m.art);
        renumber(&mut art, fresh, count, next);
        (!Arc::ptr_eq(&art, &m.art)).then_some(art)
    });
    let mut slots = vec![];
    // Children's new ids by their old ones, for an Objects on Path group, which names its path and
    // members by id. An id another child keeps still names that child.
    let mut renamed = std::collections::HashMap::new();
    let mut kept = HashSet::new();
    for (i, c) in n.children().into_iter().flatten().enumerate() {
        let mut new = Arc::clone(c);
        renumber(&mut new, fresh, count, next);
        if new.id == c.id {
            kept.insert(c.id);
        } else {
            renamed.insert(c.id, new.id);
        }
        if !Arc::ptr_eq(&new, c) {
            slots.push((i, new));
        }
    }
    renamed.retain(|old, _| !kept.contains(old));
    if !own && mask.is_none() && slots.is_empty() {
        return;
    }
    let node = Arc::make_mut(n);
    if own {
        node.id = NodeId(*next);
        *next += 1;
    }
    if let (Some(art), Some(m)) = (mask, node.extras_if_any().and_then(|e| e.mask.as_mut())) {
        m.art = art;
    }
    if let Some(children) = node.children_mut() {
        for (i, c) in slots {
            if let Some(slot) = children.get_mut(i) {
                *slot = c;
            }
        }
    }
    if !renamed.is_empty()
        && let Some(on_path) = node.extras_if_any().and_then(|e| e.objects_on_path.as_mut())
    {
        on_path.rename(&renamed);
    }
}

impl Document {
    /// The roots of the art kept outside the layers, each with its [`StoredRoot`].
    fn definition_roots(&self) -> impl Iterator<Item = (&Arc<Node>, StoredRoot)> {
        let symbols = self.symbols.iter().enumerate().map(|(i, s)| (&s.art, StoredRoot::Symbol(i)));
        let designs = self.graph_designs.iter().enumerate().map(|(i, g)| (&g.art, StoredRoot::GraphDesign(i)));
        let patterns = self
            .patterns
            .iter()
            .enumerate()
            .flat_map(|(p, def)| def.art.iter().enumerate().map(move |(a, n)| (n, StoredRoot::Pattern { pattern: p, art: a })));
        let mounted = self.mounted_art.iter().enumerate().map(|(i, (n, _))| (n, StoredRoot::Mounted(i)));
        symbols.chain(designs).chain(patterns).chain(mounted)
    }

    /// Every root of stored art: the layers, then the art of symbols, graph designs and patterns,
    /// then [`Document::mounted_art`].
    fn stored_roots(&self) -> impl Iterator<Item = (&Arc<Node>, StoredRoot)> {
        self.layers.iter().enumerate().map(|(i, l)| (l, StoredRoot::Layer(i))).chain(self.definition_roots())
    }

    /// The root `root` names, to change.
    fn stored_root_mut(&mut self, root: StoredRoot) -> Option<&mut Arc<Node>> {
        match root {
            StoredRoot::Layer(i) => self.layers.get_mut(i),
            StoredRoot::Symbol(i) => self.symbols.get_mut(i).map(|s| &mut s.art),
            StoredRoot::GraphDesign(i) => self.graph_designs.get_mut(i).map(|g| &mut g.art),
            StoredRoot::Pattern { pattern, art } => self.patterns.get_mut(pattern)?.art.get_mut(art),
            StoredRoot::Mounted(i) => self.mounted_art.get_mut(i).map(|(n, _)| n),
        }
    }

    /// Visit every stored object with its place: the layer tree with the opacity-mask art of its
    /// objects, then the art of symbols, graph designs and pattern swatches (their definitions,
    /// once each, never the copies instances and tiles draw), then [`Document::mounted_art`].
    pub fn visit_stored<'a>(&'a self, mut f: impl FnMut(&'a Node, StoredPlace)) {
        self.visit_stored_arcs(|n, place| f(n, place));
    }

    /// [`Self::visit_stored`] with each object's shared handle, which keeps its identity from one
    /// look to the next while the object is unchanged (for caches such as thumbnails).
    pub fn visit_stored_arcs<'a>(&'a self, mut f: impl FnMut(&'a Arc<Node>, StoredPlace)) {
        for (root, r) in self.stored_roots() {
            visit(root, StoredPlace { root: r, mask_of: None }, &mut f);
        }
    }

    /// Where object `id` is stored: the root, and the steps from it.
    fn stored_path(&self, id: NodeId) -> Option<(StoredPlace, Vec<Step>)> {
        if let Some(path) = self.index_path(id) {
            let (first, rest) = path.split_first()?;
            return Some((StoredPlace { root: StoredRoot::Layer(*first), mask_of: None }, rest.iter().map(|i| Step::Child(*i)).collect()));
        }
        // Not in the layer tree: in mask art (in the layers too) or in a definition's art.
        for (root, r) in self.stored_roots() {
            let (mut steps, mut mask_of) = (vec![], None);
            if find(root, id, &mut steps, &mut mask_of) {
                return Some((StoredPlace { root: r, mask_of }, steps));
            }
        }
        None
    }

    /// Stored object `id` (see [`Self::visit_stored`]) with its place: one in the layer tree as
    /// [`Self::node`] finds it, else the first in visiting order.
    pub fn stored_node(&self, id: NodeId) -> Option<(&Arc<Node>, StoredPlace)> {
        let (place, steps) = self.stored_path(id)?;
        let root = self.stored_roots().find(|(_, r)| *r == place.root)?.0;
        Some((at(root, &steps)?, place))
    }

    /// [`Self::stored_node`] to change; the nodes on the way are copied when shared.
    pub fn stored_node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let (place, steps) = self.stored_path(id)?;
        at_mut(self.stored_root_mut(place.root)?, &steps)
    }

    /// Gives the objects outside the layer tree (in opacity-mask art and in the art of symbols,
    /// graph designs and patterns) whose id is 0, past the id range, or already taken by an object
    /// visited before them (the layer tree's own objects first) a new id, so every stored object
    /// can be named by its id. Returns how many changed; the id counter then stands above them.
    pub(crate) fn renumber_stored_duplicates(&mut self) -> usize {
        self.renumber_duplicates_in(|_| true)
    }

    /// Raises the id counter above the ids of [`Document::mounted_art`]'s objects, then gives those
    /// objects whose id is 0, past the id range, or already taken by an object visited before them
    /// (every other stored object first) a new id, as [`Self::renumber_stored_duplicates`] gives
    /// them. Returns how many changed.
    pub fn number_mounted_art(&mut self) -> usize {
        let mut max = 0;
        for (i, (root, _)) in self.mounted_art.iter().enumerate() {
            visit(root, StoredPlace { root: StoredRoot::Mounted(i), mask_of: None }, &mut |n, _| {
                if n.id.0 < MAX_NEXT_ID {
                    max = max.max(n.id.0);
                }
            });
        }
        self.raise_next_id(max.saturating_add(1));
        self.renumber_duplicates_in(|r| matches!(r, StoredRoot::Mounted(_)))
    }

    /// [`Self::renumber_stored_duplicates`] for the roots `pick` holds for only; the objects of the
    /// others still count as taken.
    fn renumber_duplicates_in(&mut self, pick: impl Fn(StoredRoot) -> bool) -> usize {
        let mut seen: HashSet<NodeId> = HashSet::new();
        for l in &self.layers {
            l.walk(&mut |n| {
                seen.insert(n.id);
            });
        }
        self.slices.iter().for_each(|s| {
            seen.insert(s.id);
        });
        // Per root, the visiting indices of the nodes that take a new id.
        let mut fresh: Vec<(StoredRoot, HashSet<usize>)> = vec![];
        for (root, r) in self.stored_roots() {
            let (mut i, mut here, take) = (0, HashSet::new(), pick(r));
            visit(root, StoredPlace { root: r, mask_of: None }, &mut |n, place| {
                if !place.in_layers() && (n.id.0 == 0 || n.id.0 >= MAX_NEXT_ID || !seen.insert(n.id)) && take {
                    here.insert(i);
                }
                i += 1;
            });
            if !here.is_empty() {
                fresh.push((r, here));
            }
        }
        let total = fresh.iter().map(|(_, f)| f.len()).sum();
        if total == 0 {
            return 0;
        }
        // No id left to give: the objects keep theirs rather than share one given out later.
        if self.peek_next_id().saturating_add(total as u64) > MAX_NEXT_ID {
            return 0;
        }
        let mut next = self.peek_next_id();
        for (r, f) in fresh {
            if let Some(root) = self.stored_root_mut(r) {
                renumber(root, &f, &mut 0, &mut next);
            }
        }
        self.raise_next_id(next);
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphDesign, OpacityMask, PatternDef, Symbol};
    use vectorcraft_geom::Rect;

    fn rect(id: u64) -> Node {
        Node::path(NodeId(id), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), crate::Appearance::default_art())
    }

    fn masked(id: u64, art: Node) -> Node {
        rect(id).with_mask(Some(Box::new(OpacityMask::new(art, true))))
    }

    /// A document with object 10 in its layer, mask art 11 on it, and one object in a symbol,
    /// a graph design and a pattern (ids `defs`).
    fn doc(defs: [u64; 3]) -> Document {
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, masked(10, rect(11))).unwrap();
        d.symbols.push(Symbol { name: "S".into(), art: Arc::new(rect(defs[0])), dynamic: false });
        d.graph_designs.push(GraphDesign { name: "G".into(), art: Arc::new(rect(defs[1])) });
        d.patterns.push(PatternDef::new("P", vec![Arc::new(rect(defs[2]))]));
        d.raise_next_id(20);
        d
    }

    #[test]
    fn stored_objects_are_found_in_every_art() {
        let d = doc([12, 13, 14]);
        let place = |id: u64| d.stored_node(NodeId(id)).map(|(n, p)| (n.id.0, p));
        assert!(place(10).unwrap().1.in_layers());
        assert_eq!(place(11).unwrap().1, StoredPlace { root: StoredRoot::Layer(0), mask_of: Some(NodeId(10)) });
        assert_eq!(place(12).unwrap().1.root, StoredRoot::Symbol(0));
        assert_eq!(place(13).unwrap().1.root, StoredRoot::GraphDesign(0));
        assert_eq!(place(14).unwrap().1.root, StoredRoot::Pattern { pattern: 0, art: 0 });
        assert!(d.node(NodeId(11)).is_none() && d.node(NodeId(12)).is_none(), "the layer lookup stays the layer tree's");
        assert!(d.stored_node(NodeId(99)).is_none());
        let mut seen = vec![];
        d.visit_stored(|n, _| seen.push(n.id.0));
        assert_eq!(seen, [d.layers[0].id.0, 10, 11, 12, 13, 14]);
    }

    #[test]
    fn stored_objects_change_by_id() {
        let mut d = doc([12, 13, 14]);
        let before = d.clone();
        d.stored_node_mut(NodeId(13)).unwrap().name = Some("changed".into());
        assert_eq!(d.graph_designs[0].art.name.as_deref(), Some("changed"));
        assert!(Arc::ptr_eq(&d.symbols[0].art, &before.symbols[0].art), "other art is not copied");
        assert_eq!(before.graph_designs[0].art.name, None, "the earlier document is unchanged");
        d.stored_node_mut(NodeId(11)).unwrap().name = Some("mask".into());
        assert_eq!(d.stored_node(NodeId(11)).unwrap().0.name.as_deref(), Some("mask"));
    }

    #[test]
    fn fix_next_id_gives_nested_duplicates_new_ids() {
        // The symbol's object repeats the layer's object, the design's has no id, and the pattern's
        // repeats the mask art's.
        let mut d = doc([10, 0, 11]);
        d.next_id = 1;
        d.fix_next_id();
        let mut ids = vec![];
        d.visit_stored(|n, _| ids.push(n.id.0));
        let unique: HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
        assert!(!ids.contains(&0));
        assert_eq!(&ids[..3], &[d.layers[0].id.0, 10, 11], "the layer tree and the first mask art keep theirs");
        assert!(ids.iter().all(|id| *id < d.peek_next_id()), "{ids:?} {}", d.peek_next_id());
        // Ids already unique are kept, and a second pass changes nothing.
        let again = d.clone();
        assert_eq!(d.renumber_stored_duplicates(), 0);
        assert_eq!(d, again);
    }

    /// Mounted art is found and changed by id after the other stored art, and numbering it gives
    /// its objects that repeat another stored object's id, or have none, new ids above the counter,
    /// leaving every other object's id and its own unique ones as they are.
    #[test]
    fn mounted_art_is_found_and_numbered_after_the_rest() {
        let mut d = doc([12, 13, 14]);
        let owner = serde_json::json!({ "kind": "brush" });
        d.mounted_art = vec![(Arc::new(masked(10, rect(0))), owner.clone()), (Arc::new(rect(15)), owner)];
        assert_eq!(d.stored_node(NodeId(15)).map(|(_, p)| p.root), Some(StoredRoot::Mounted(1)));
        let before = d.clone();
        assert_eq!(d.number_mounted_art(), 2);
        let mut ids = vec![];
        d.visit_stored(|n, _| ids.push(n.id.0));
        let unique: HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
        assert_eq!(&ids[..6], &[d.layers[0].id.0, 10, 11, 12, 13, 14], "the other art keeps its ids");
        assert_eq!(ids[8], 15, "a unique id is kept");
        assert!(ids.iter().all(|id| *id < d.peek_next_id()), "{ids:?} {}", d.peek_next_id());
        assert!(Arc::ptr_eq(&d.symbols[0].art, &before.symbols[0].art) && Arc::ptr_eq(&d.mounted_art[1].0, &before.mounted_art[1].0));
        d.stored_node_mut(NodeId(15)).unwrap().name = Some("changed".into());
        assert_eq!(d.mounted_art[1].0.name.as_deref(), Some("changed"));
        assert_eq!(d.number_mounted_art(), 0, "a second pass changes nothing");
    }

    #[test]
    fn renumbered_objects_on_path_keep_their_path_and_members() {
        // A symbol's Objects on Path group whose path repeats the layer's object 10 and whose
        // member has no id.
        let mut d = doc([12, 13, 14]);
        let (path, member) = (rect(10), rect(0));
        let rec = crate::objects_on_path::ObjectsOnPath::new(path.id, &[&member], crate::objects_on_path::CENTER, 0.0);
        let mut g = Node::new(NodeId(15), crate::NodeKind::Group { children: vec![Arc::new(path), Arc::new(member)], clip: false });
        g.extras_mut().objects_on_path = Some(Box::new(rec));
        d.symbols[0].art = Arc::new(g);
        d.fix_next_id();
        let art = &d.symbols[0].art;
        let kids: Vec<NodeId> = art.children().unwrap().iter().map(|c| c.id).collect();
        assert!(kids.iter().all(|id| id.0 != 10 && id.0 != 0), "{kids:?}");
        let on_path = art.extras().objects_on_path.as_ref().unwrap();
        assert_eq!(on_path.path, kids[0]);
        assert_eq!(on_path.members.iter().map(|m| m.id).collect::<Vec<_>>(), [kids[1]]);
    }
}
