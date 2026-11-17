//! Where the clipping path of a clip group or clipping layer sits among its members: first, under
//! the art, in files, and on top of the art in an open document, where Make Clipping Mask leaves
//! it ([`Document::put_clipping_paths`]). A clip finds its clipping path by its mark wherever it
//! is ([`Node::clipping_path_index`]); its place only matters to what reads files by place.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{Document, Node, NodeKind};

/// Where [`Document::put_clipping_paths`] puts each clipping path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClippingPathAt {
    /// First among the clip's members, under the art, as files keep it: versions that take a
    /// clip's first member as its clipping path read it, and a compound path or type, whose mark
    /// files don't keep, is found there again.
    First,
    /// On top of the art (under a layer's ruler guides), marked, as an open document keeps it.
    Top,
}

/// The clips of one tree whose clipping path moved: each by its place (the child indices from the
/// tree's root, as they are once the clips around it have moved theirs), with the index its
/// clipping path moved from and the one it moved to.
type Moves = HashMap<Vec<u32>, (usize, usize)>;

impl Document {
    /// Put the clipping path of every clip group and clipping layer in the stored art at `at`: in
    /// the layers with the opacity-mask art of their objects, and in the art of symbols, graph
    /// designs and pattern swatches (the original of a pattern being edited too) and
    /// [`Document::mounted_art`]. What shows doesn't change. With [`ClippingPathAt::Top`] each
    /// clipping path is also marked ([`Node::set_clipping_path`]). The overrides of dynamic symbol
    /// instances ([`crate::SymbolOverride::path`]) go on naming the objects they named in their
    /// symbol's art. Only the branches holding a clip to change are copied.
    pub fn put_clipping_paths(&mut self, at: ClippingPathAt) {
        let mut moved: HashMap<String, Moves> = HashMap::new();
        for s in &mut self.symbols {
            let mut moves = Moves::new();
            put_tree(&mut s.art, at, &mut moves);
            if !moves.is_empty() {
                moved.insert(s.name.clone(), moves);
            }
        }
        for root in self.other_roots_mut() {
            put_tree(root, at, &mut Moves::new());
        }
        if moved.is_empty() {
            return;
        }
        for s in &mut self.symbols {
            follow_tree(&mut s.art, &moved);
        }
        for root in self.other_roots_mut() {
            follow_tree(root, &moved);
        }
    }

    /// The roots of the stored art other than the symbols': the layers, the art of graph designs
    /// and pattern swatches (the original of a pattern being edited too) and the mounted art.
    fn other_roots_mut(&mut self) -> impl Iterator<Item = &mut Arc<Node>> {
        self.layers
            .iter_mut()
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()))
            .chain(self.mounted_art.iter_mut().map(|(n, _)| n))
    }
}

/// [`placed`] in place for the tree `root`.
fn put_tree(root: &mut Arc<Node>, at: ClippingPathAt, moves: &mut Moves) {
    if let Some(n) = placed(root, at, &mut vec![], moves) {
        *root = Arc::new(n);
    }
}

/// `n` with the clipping paths of the clips in it (itself, its members, and the opacity-mask art of
/// each) put at `at`, or None when none changes. Each clip whose clipping path moved is added to
/// `moves` with its place (`path` is `n`'s), except those in opacity-mask art, which no override
/// names.
fn placed(n: &Arc<Node>, at: ClippingPathAt, path: &mut Vec<u32>, moves: &mut Moves) -> Option<Node> {
    let mut out = own(n, at).map(|(m, from, to)| {
        if from != to {
            moves.insert(path.clone(), (from, to));
        }
        m
    });
    if let Some(art) = n.extras().mask.as_deref().and_then(|m| placed(&m.art, at, &mut vec![], &mut Moves::new())) {
        let m = out.get_or_insert_with(|| (**n).clone());
        if let Some(mask) = m.extras_mut().mask.as_mut() {
            mask.art = Arc::new(art);
        }
    }
    let mut changed = vec![];
    for (i, c) in out.as_ref().unwrap_or(n).children().into_iter().flatten().enumerate() {
        path.push(u32::try_from(i).unwrap_or(u32::MAX));
        if let Some(c) = placed(c, at, path, moves) {
            changed.push((i, c));
        }
        path.pop();
    }
    if !changed.is_empty() {
        let m = out.get_or_insert_with(|| (**n).clone());
        if let Some(children) = m.children_mut() {
            for (i, c) in changed {
                if let Some(slot) = children.get_mut(i) {
                    *slot = Arc::new(c);
                }
            }
        }
    }
    out
}

/// `n` with its own clipping path at `at` (and marked, for [`ClippingPathAt::Top`]), with the index
/// it moved from and to, when `n` is a clip whose clipping path is elsewhere or (for `Top`)
/// unmarked.
fn own(n: &Node, at: ClippingPathAt) -> Option<(Node, usize, usize)> {
    let from = n.clipping_path_index()?;
    let children = n.children()?;
    let path = children.get(from).filter(|c| !c.is_ruler_guide())?;
    let to = match at {
        ClippingPathAt::First => 0,
        ClippingPathAt::Top => Node::guide_run_start(children).checked_sub(1)?,
    };
    let mark = at == ClippingPathAt::Top && !path.is_clipping_path();
    if from == to && !mark {
        return None;
    }
    let mut m = n.clone();
    let children = m.children_mut()?;
    let mut path = children.remove(from);
    if mark {
        Arc::make_mut(&mut path).set_clipping_path(true);
    }
    children.insert(to.min(children.len()), path);
    Some((m, from, to))
}

/// [`followed`] in place for the tree `root`.
fn follow_tree(root: &mut Arc<Node>, moved: &HashMap<String, Moves>) {
    if let Some(n) = followed(root, moved) {
        *root = Arc::new(n);
    }
}

/// `n` with the overrides of the dynamic symbol instances in it (opacity-mask art included)
/// following the clipping paths `moved` moved in their symbols' art, or None when none changes.
fn followed(n: &Arc<Node>, moved: &HashMap<String, Moves>) -> Option<Node> {
    let mut out = None;
    if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
        && let Some(moves) = moved.get(symbol)
    {
        let overrides = &n.extras().symbol_overrides;
        let paths: Vec<Vec<u32>> = overrides.iter().map(|o| moved_path(&o.path, moves)).collect();
        if overrides.iter().zip(&paths).any(|(o, p)| o.path != *p) {
            let mut m = (**n).clone();
            for (o, p) in m.extras_mut().symbol_overrides.iter_mut().zip(paths) {
                o.path = p;
            }
            out = Some(m);
        }
    }
    if let Some(art) = n.extras().mask.as_deref().and_then(|m| followed(&m.art, moved)) {
        let m = out.get_or_insert_with(|| (**n).clone());
        if let Some(mask) = m.extras_mut().mask.as_mut() {
            mask.art = Arc::new(art);
        }
    }
    let changed: Vec<(usize, Node)> =
        n.children().into_iter().flatten().enumerate().filter_map(|(i, c)| followed(c, moved).map(|c| (i, c))).collect();
    if !changed.is_empty() {
        let m = out.get_or_insert_with(|| (**n).clone());
        if let Some(children) = m.children_mut() {
            for (i, c) in changed {
                if let Some(slot) = children.get_mut(i) {
                    *slot = Arc::new(c);
                }
            }
        }
    }
    out
}

/// `path` (child indices in a tree before `moves`) in the tree after them.
fn moved_path(path: &[u32], moves: &Moves) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(path.len());
    for &i in path {
        let i = match moves.get(&out) {
            Some(&(from, to)) => u32::try_from(moved_index(i as usize, from, to)).unwrap_or(i),
            None => i,
        };
        out.push(i);
    }
    out
}

/// Where the member at index `i` goes when the member at `from` moves to `to`.
fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        return to;
    }
    let j = if i > from { i - 1 } else { i };
    if j >= to { j + 1 } else { j }
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_geom::Rect;

    use super::*;
    use crate::{NodeId, Symbol, SymbolOverride};

    fn rect(id: u64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), crate::Appearance::default_art()))
    }

    fn ids(n: &Node) -> Vec<u64> {
        n.children().unwrap().iter().map(|c| c.id.0).collect()
    }

    #[test]
    fn moved_index_follows_one_member_moving() {
        // [a, b, c, d]: b to the top, then back first.
        assert_eq!((0..4).map(|i| moved_index(i, 1, 3)).collect::<Vec<_>>(), [0, 3, 1, 2]);
        assert_eq!((0..4).map(|i| moved_index(i, 3, 0)).collect::<Vec<_>>(), [1, 2, 3, 0]);
        assert_eq!((0..3).map(|i| moved_index(i, 1, 1)).collect::<Vec<_>>(), [0, 1, 2]);
    }

    /// A clip group's first member, unmarked, goes on top marked, and back first; a layer's
    /// clipping path goes under its ruler guides; an instance's override follows the object it
    /// names in its symbol's art.
    #[test]
    fn clipping_paths_go_on_top_and_back_first_and_overrides_follow() {
        let mut d = Document::new(100.0, 100.0);
        // A symbol whose art is a clip group [mask 2, a 3, b 4], and an instance overriding b.
        let art = Node::new(NodeId(1), NodeKind::Group { children: vec![rect(2), rect(3), rect(4)], clip: true });
        d.symbols.push(Symbol { name: "s".into(), art: Arc::new(art), dynamic: true });
        let mut instance = Node::new(NodeId(10), NodeKind::SymbolInstance { symbol: "s".into(), xf: Default::default() });
        let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
        instance.extras_mut().symbol_overrides = vec![SymbolOverride { path: vec![2], fill: Some(red), ..Default::default() }];
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, instance).unwrap();
        // A clip group in the layer, with a nested clip group as its art.
        let inner = Node::new(NodeId(21), NodeKind::Group { children: vec![rect(22), rect(23)], clip: true });
        let outer = Node::new(NodeId(20), NodeKind::Group { children: vec![rect(24), Arc::new(inner)], clip: true });
        d.insert(Some(layer), 1, outer).unwrap();

        d.put_clipping_paths(ClippingPathAt::Top);
        let art = d.symbols[0].art.clone();
        assert_eq!(ids(&art), [3, 4, 2]);
        assert!(art.children().unwrap()[2].is_clipping_path(), "marked");
        assert_eq!(d.node(NodeId(10)).unwrap().extras().symbol_overrides[0].path, [1], "b, now second");
        let outer = d.node(NodeId(20)).unwrap();
        assert_eq!(ids(outer), [21, 24]);
        assert_eq!(ids(d.node(NodeId(21)).unwrap()), [23, 22]);
        assert_eq!(outer.clipping_path().map(|c| c.id), Some(NodeId(24)));
        // Again: nothing moves.
        let again = d.clone();
        d.put_clipping_paths(ClippingPathAt::Top);
        assert!(Arc::ptr_eq(&again.layers[0], &d.layers[0]) && Arc::ptr_eq(&again.symbols[0].art, &d.symbols[0].art));

        d.put_clipping_paths(ClippingPathAt::First);
        assert_eq!(ids(&d.symbols[0].art), [2, 3, 4]);
        assert_eq!(d.node(NodeId(10)).unwrap().extras().symbol_overrides[0].path, [2]);
        assert_eq!(ids(d.node(NodeId(20)).unwrap()), [24, 21]);
        assert_eq!(ids(d.node(NodeId(21)).unwrap()), [22, 23]);

        // A clipping layer: its clipping path goes under its ruler guides.
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        for (i, id) in [31, 32].into_iter().enumerate() {
            d.insert(Some(layer), i, (*rect(id)).clone()).unwrap();
        }
        let guide = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::new(guide, NodeKind::RulerGuide { vertical: true, pos: 10.0, artboard: None })).unwrap();
        d.node_mut(layer).unwrap().set_clips(true);
        d.put_clipping_paths(ClippingPathAt::Top);
        let l = d.node(layer).unwrap();
        assert_eq!(ids(l), [32, 31, guide.0]);
        assert_eq!(l.clipping_path().map(|c| c.id), Some(NodeId(31)));
    }
}
