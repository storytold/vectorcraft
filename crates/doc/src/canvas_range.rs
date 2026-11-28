//! A read document brought within the canvas: what lies beyond [`MAX_COORD`] is removed (#3575).
//! Edits refuse to leave anything there (`doc_sane` in the engine checks the artboards, the guides,
//! the ruler origin and the objects an edit touches), so a file holding something beyond it could be
//! opened but not edited: one object, artboard or guide out there made every move fail.

use std::collections::BTreeSet;
use std::sync::Arc;

use vectorcraft_geom::{Point, Rect};

use crate::{Document, MAX_COORD, Node, NodeId, NodeKind};

/// What [`Document::drop_beyond_canvas`] removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BeyondCanvas {
    /// Objects removed whole (a layer or group keeps its children within the canvas).
    pub objects: usize,
    pub artboards: usize,
    /// Guides beyond the canvas, the artboard guides of the artboards removed, and the layer
    /// guides of the layers removed.
    pub guides: usize,
    /// The ruler origin was beyond the canvas and is back at the origin.
    pub ruler_origin: bool,
}

impl BeyondCanvas {
    /// The warning for what was removed, or `None` when nothing was.
    pub fn note(&self) -> Option<String> {
        let count = |n: usize, one: &str, many: &str| match n {
            0 => None,
            1 => Some(format!("1 {one}")),
            n => Some(format!("{n} {many}")),
        };
        let parts: Vec<String> =
            [count(self.objects, "object", "objects"), count(self.artboards, "artboard", "artboards"), count(self.guides, "guide", "guides")]
                .into_iter()
                .flatten()
                .collect();
        let limit = format!("more than {MAX_COORD} pt from the origin");
        match (parts.is_empty(), self.ruler_origin) {
            (true, false) => None,
            (true, true) => Some(format!("the ruler origin lay beyond the canvas ({limit}) and was reset")),
            (false, origin) => Some(format!(
                "{} beyond the canvas ({limit}) {} removed{}",
                parts.join(", "),
                if self.objects + self.artboards + self.guides == 1 { "was" } else { "were" },
                if origin { "; the ruler origin, beyond it too, was reset" } else { "" }
            )),
        }
    }
}

/// Is every side of `r` finite and within [`MAX_COORD`] of the origin?
fn rect_in_range(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| in_range(*v))
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORD
}

/// Does `n` reach beyond the canvas? An object without geometry doesn't.
fn beyond(n: &Node) -> bool {
    n.geometric_bounds().is_some_and(|b| !rect_in_range(b))
}

/// Whether `n` is a layer or group, and if so whether it clips: the containers whose objects are
/// looked at one by one. Anything else is one object, measured whole.
fn clipping(n: &Node) -> Option<bool> {
    match n.kind {
        NodeKind::Layer { clip, .. } | NodeKind::Group { clip, .. } => Some(clip),
        _ => None,
    }
}

/// The objects of a layer or group that a walk picked, by index in order, each with what was
/// picked inside it (nothing for an object that is not a container).
#[derive(Debug, Default)]
struct Picked(Vec<(usize, Picked)>);

/// What in `n` reaches beyond the canvas, by `far`, or `None` when nothing does. A container is
/// looked into object by object: its own bounds leave out a layer's hidden objects and, when it
/// clips, all but its clipping path. Each object is measured once, and the objects a clipping
/// path beyond the canvas clips are not measured: they go with it (see [`clips_beyond`]).
fn reach(n: &Node, far: &impl Fn(&Node) -> bool) -> Option<Picked> {
    let Some(clip) = clipping(n) else { return far(n).then(Picked::default) };
    let mut inner = Vec::new();
    for (i, c) in n.children().into_iter().flatten().enumerate() {
        if let Some(p) = reach(c, far) {
            inner.push((i, p));
            if clip && i == 0 {
                break;
            }
        }
    }
    (!inner.is_empty()).then_some(Picked(inner))
}

/// Does `n`, whose objects beyond the canvas are `found`, clip by a path that reaches beyond it?
/// Then the path and the objects it clips go together, so no other object becomes the clipping
/// path.
fn clips_beyond(n: &Node, found: &Picked) -> bool {
    clipping(n) == Some(true) && found.0.first().is_some_and(|(i, _)| *i == 0)
}

/// Add the ids of `n` and the layers in it, if it is a layer, to `layers`: their layer guides go
/// with them.
fn layers_in(n: &Node, layers: &mut BTreeSet<NodeId>) {
    if matches!(n.kind, NodeKind::Layer { .. }) {
        layers.insert(n.id);
    }
    if clipping(n).is_some() {
        for c in n.children().into_iter().flatten() {
            layers_in(c, layers);
        }
    }
}

/// How many ruler guides the layers in `n` hold, if it is a layer or group.
fn guides_in(n: &Node) -> usize {
    match clipping(n) {
        None => 0,
        Some(_) => n.children().into_iter().flatten().map(|c| if c.is_ruler_guide() { 1 } else { guides_in(c) }).sum(),
    }
}

/// Remove the objects in `node` that reach beyond the canvas, which [`reach`] `found`: a layer or
/// group keeps those of its objects within it, unless its clipping path reaches beyond (see
/// [`clips_beyond`]); anything else goes whole. Only containers holding such an object are
/// copied, and only those are visited again. Recursion follows the nesting, which
/// [`Document::limit_depth`] bounds. The layers removed are added to `layers`, and the ruler
/// guides they held to `guides`.
fn drop_children_beyond(node: &mut Arc<Node>, found: Picked, dropped: &mut usize, layers: &mut BTreeSet<NodeId>, guides: &mut usize) {
    let Some(children) = Arc::make_mut(node).children_mut() else { return };
    let mut found = found.0.into_iter().peekable();
    let mut next = 0;
    children.retain_mut(|c| {
        let at = next;
        next += 1;
        let Some((_, inner)) = found.next_if(|(i, _)| *i == at) else { return true };
        if clipping(c).is_some() && !clips_beyond(c, &inner) {
            drop_children_beyond(c, inner, dropped, layers, guides);
            return true;
        }
        layers_in(c, layers);
        *guides += guides_in(c);
        *dropped += 1;
        false
    });
}

/// Is `id` unset, or not one of `ids`? The check takes time logarithmic in the number of ids.
fn not_in<T: Ord>(id: Option<T>, ids: &BTreeSet<T>) -> bool {
    id.is_none_or(|i| !ids.contains(&i))
}

/// Remove the ruler guides of layer `layer` and of its sublayers that `out` picks, adding how many
/// to `dropped`. `out` looks at each object in the layers once. Only layers holding one are
/// copied.
fn drop_guides(layer: &mut Arc<Node>, out: &impl Fn(&Node) -> bool, dropped: &mut usize) {
    /// The objects of `n`, if it is a layer, that `out` picks (with nothing inside), and its
    /// sublayers holding one (with what they hold), or `None` when it holds none.
    fn holds(n: &Node, out: &impl Fn(&Node) -> bool) -> Option<Picked> {
        if !n.is_layer() {
            return None;
        }
        let inner: Vec<(usize, Picked)> = n
            .children()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, c)| if out(c) { Some((i, Picked::default())) } else { holds(c, out).map(|p| (i, p)) })
            .collect();
        (!inner.is_empty()).then_some(Picked(inner))
    }
    fn drop(layer: &mut Arc<Node>, held: Picked, dropped: &mut usize) {
        let Some(children) = Arc::make_mut(layer).children_mut() else { return };
        let mut held = held.0.into_iter().peekable();
        let mut next = 0;
        children.retain_mut(|c| {
            let at = next;
            next += 1;
            let Some((_, inner)) = held.next_if(|(i, _)| *i == at) else { return true };
            // A sublayer holds something; an object picked itself has nothing inside.
            if inner.0.is_empty() {
                *dropped += 1;
                return false;
            }
            drop(c, inner, dropped);
            true
        });
    }
    if let Some(held) = holds(layer, out) {
        drop(layer, held, dropped);
    }
}

impl Document {
    /// Remove what a read file has beyond the canvas ([`MAX_COORD`], or not finite): objects
    /// (see [`BeyondCanvas::objects`]; a top-level layer clipping by a path beyond it is emptied
    /// and stops clipping), artboards with their ruler origins and artboard guides (their ids
    /// are retired, as deleting them does), guides with the layer guides of the layers removed,
    /// and put a ruler origin beyond it back at the origin. Saved selections, assets and variable
    /// bindings then forget the objects removed. Run after [`Document::limit_depth`]. A document
    /// whose artboards were all removed has none left.
    pub fn drop_beyond_canvas(&mut self) -> BeyondCanvas {
        let mut gone = BeyondCanvas::default();
        let mut layers = BTreeSet::new();
        for layer in &mut self.layers {
            let Some(found) = reach(layer, &beyond) else { continue };
            if clips_beyond(layer, &found) {
                let layer = Arc::make_mut(layer);
                if let NodeKind::Layer { children, clip, .. } = &mut layer.kind {
                    for c in children.iter() {
                        layers_in(c, &mut layers);
                    }
                    // The layer's own ruler guides stay; only its art goes, with its sublayers' guides.
                    gone.objects += children.iter().filter(|c| !c.is_ruler_guide()).count();
                    gone.guides += children.iter().filter(|c| !c.is_ruler_guide()).map(|c| guides_in(c)).sum::<usize>();
                    children.retain(|c| c.is_ruler_guide());
                    *clip = false;
                }
            } else {
                drop_children_beyond(layer, found, &mut gone.objects, &mut layers, &mut gone.guides);
            }
        }
        if gone.objects > 0 {
            self.tidy_saved_selections();
            self.prune_assets();
            self.prune_variable_bindings();
        }
        let removed: Vec<u32> = self.artboards.iter().filter(|a| !rect_in_range(a.rect)).map(|a| a.id).collect();
        self.artboards.retain(|a| rect_in_range(a.rect));
        for id in &removed {
            self.artboard_ruler_origins.remove(id);
            self.retire_artboard_id(*id);
        }
        gone.artboards = removed.len();
        let removed: BTreeSet<u32> = removed.into_iter().collect();
        // The saved list, when the guides are not nodes yet ([`Document::guides_into_nodes`]).
        let guides = self.saved_guides.len();
        self.saved_guides.retain(|g| in_range(g.pos) && not_in(g.artboard, &removed) && not_in(g.layer, &layers));
        gone.guides += guides - self.saved_guides.len();
        let out = |n: &Node| match n.kind {
            NodeKind::RulerGuide { pos, artboard, .. } => !in_range(pos) || !not_in(artboard, &removed),
            _ => false,
        };
        for layer in &mut self.layers {
            drop_guides(layer, &out, &mut gone.guides);
        }
        if !(in_range(self.ruler_origin.x) && in_range(self.ruler_origin.y)) {
            self.ruler_origin = Point::ZERO;
            gone.ruler_origin = true;
        }
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Artboard, Guide};
    use vectorcraft_geom::shapes;

    fn rect(id: u64, y: f64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, y, 10.0, y + 10.0)), Appearance::default_art()))
    }

    fn doc(children: Vec<Arc<Node>>) -> Document {
        let mut d = Document::new(100.0, 100.0);
        if let Some(ch) = d.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
            ch.extend(children);
        }
        d
    }

    fn ids(d: &Document) -> Vec<u64> {
        let mut out = vec![];
        fn walk(n: &Node, out: &mut Vec<u64>) {
            out.push(n.id.0);
            for c in n.children().into_iter().flatten() {
                walk(c, out);
            }
        }
        for l in &d.layers {
            walk(l, &mut out);
        }
        out
    }

    #[test]
    fn a_document_within_the_canvas_is_left_alone() {
        let mut d = doc(vec![rect(10, 0.0), rect(11, MAX_COORD - 10.0), rect(12, -MAX_COORD)]);
        let before = d.clone();
        assert_eq!(d.drop_beyond_canvas(), BeyondCanvas::default());
        assert_eq!(d, before);
        assert_eq!(BeyondCanvas::default().note(), None);
    }

    #[test]
    fn objects_beyond_the_canvas_are_removed_and_groups_keep_the_rest() {
        let group = Arc::new(Node::group(NodeId(20), vec![rect(21, 0.0), rect(22, 1e17)]));
        let clip = Arc::new(Node::new(NodeId(30), NodeKind::Group { children: vec![rect(31, f64::NAN), rect(32, 0.0)], clip: true }));
        let mut d = doc(vec![rect(10, 0.0), rect(11, 1e17), group, clip, rect(12, -MAX_COORD - 1.0)]);
        let layer = d.layers[0].id.0;
        let gone = d.drop_beyond_canvas();
        assert_eq!(gone, BeyondCanvas { objects: 4, ..Default::default() });
        assert_eq!(ids(&d), [layer, 10, 20, 21], "the group keeps its object within the canvas; the clip group goes with its clipping path");
        assert!(d.layers.iter().all(|l| !beyond(l)));
        assert_eq!(gone.note().unwrap(), format!("4 objects beyond the canvas (more than {MAX_COORD} pt from the origin) were removed"));
    }

    #[test]
    fn artboards_guides_and_the_ruler_origin_beyond_the_canvas_are_removed_or_reset() {
        let mut d = doc(vec![rect(10, 0.0)]);
        let far = Artboard { id: 9, rect: Rect::new(0.0, 1e17, 100.0, 1e17 + 100.0), ..d.artboards[0].clone() };
        d.artboards.push(far);
        let guide = |pos: f64, artboard| Guide { artboard, ..Guide::new(false, pos) };
        d.saved_guides = vec![guide(50.0, None), guide(1e17, None), guide(f64::INFINITY, None), guide(20.0, Some(9))];
        d.ruler_origin = Point::new(1e17, 0.0);
        // The same guides as nodes, as a document has them once read.
        let mut nodes = d.clone();
        assert_eq!(nodes.guides_into_nodes(), 4);
        let gone = d.drop_beyond_canvas();
        assert_eq!(gone, BeyondCanvas { objects: 0, artboards: 1, guides: 3, ruler_origin: true });
        assert_eq!(d.artboards.len(), 1);
        assert_eq!(d.saved_guides, [guide(50.0, None)], "the far artboard's own guide goes with it");
        assert_eq!(nodes.drop_beyond_canvas(), gone);
        let left: Vec<(bool, f64, Option<u32>)> = nodes.ruler_guides().iter().map(|g| (g.vertical, g.pos, g.artboard)).collect();
        assert_eq!(left, [(false, 50.0, None)]);
        assert_eq!(d.ruler_origin, Point::ZERO);
        assert_eq!(
            gone.note().unwrap(),
            format!(
                "1 artboard, 3 guides beyond the canvas (more than {MAX_COORD} pt from the origin) were removed; the ruler origin, beyond it too, was reset"
            )
        );
    }

    /// A sublayer like the document's first layer, holding `children`.
    fn sublayer(d: &Document, id: u64, children: Vec<Arc<Node>>, clip: bool) -> Arc<Node> {
        let mut l = (*d.layers[0]).clone();
        l.id = NodeId(id);
        if let NodeKind::Layer { children: c, clip: k, .. } = &mut l.kind {
            *c = children;
            *k = clip;
        }
        Arc::new(l)
    }

    #[test]
    fn objects_hidden_or_clipped_inside_a_container_are_looked_at_one_by_one() {
        // A layer's bounds leave out its hidden objects, and a clip group's are its clipping path's.
        let mut hidden = (*rect(41, 1e17)).clone();
        hidden.visible = false;
        let probe = Document::new(1.0, 1.0);
        let sub = sublayer(&probe, 40, vec![Arc::new(hidden), rect(42, 0.0)], false);
        let clip = Arc::new(Node::new(NodeId(30), NodeKind::Group { children: vec![rect(31, 0.0), rect(32, 1e17), rect(33, 5.0)], clip: true }));
        let mut d = doc(vec![sub, clip]);
        let layer = d.layers[0].id.0;
        assert_eq!(d.drop_beyond_canvas().objects, 2);
        assert_eq!(ids(&d), [layer, 40, 42, 30, 31, 33], "the far hidden object and the far clipped one go; the clipping path stays first");
    }

    #[test]
    fn a_top_level_layer_clipping_by_a_path_beyond_the_canvas_is_emptied() {
        let mut d = doc(vec![rect(10, 1e17), rect(11, 0.0), rect(12, 5.0)]);
        if let NodeKind::Layer { clip, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
            *clip = true;
        }
        assert_eq!(d.drop_beyond_canvas().objects, 3, "the clipping path and what it clipped go together");
        assert!(
            matches!(&d.layers[0].kind, NodeKind::Layer { children, clip: false, .. } if children.is_empty()),
            "no other object becomes the clipping path"
        );
        // A clipping sublayer goes whole, as a clip group does.
        let mut d = doc(vec![]);
        let sub = sublayer(&d, 40, vec![rect(41, 1e17), rect(42, 0.0)], true);
        let mut d2 = doc(vec![sub, rect(10, 0.0)]);
        let layer = d2.layers[0].id.0;
        assert_eq!(d2.drop_beyond_canvas().objects, 1);
        assert_eq!(ids(&d2), [layer, 10]);
        // The guides of a sublayer emptied out of a top-level layer go with it, counted.
        let probe = Document::new(1.0, 1.0);
        let sub = sublayer(&probe, 40, vec![rect(41, 0.0)], false);
        let mut d3 = doc(vec![rect(10, 1e17), sub]);
        if let NodeKind::Layer { clip, .. } = &mut Arc::make_mut(&mut d3.layers[0]).kind {
            *clip = true;
        }
        d3.saved_guides =
            vec![Guide { layer: Some(NodeId(40)), ..Guide::new(false, 5.0) }, Guide { layer: Some(d3.layers[0].id), ..Guide::new(true, 5.0) }];
        let mut nodes = d3.clone();
        assert_eq!(nodes.guides_into_nodes(), 2);
        let gone = d3.drop_beyond_canvas();
        assert_eq!((gone.objects, gone.guides), (2, 1), "the clipping path, the sublayer and its guide; the layer's own guide stays");
        assert_eq!(nodes.drop_beyond_canvas(), gone);
        assert_eq!(nodes.ruler_guides().len(), 1);
        assert_eq!(d.drop_beyond_canvas(), BeyondCanvas::default());
    }

    #[test]
    fn a_removed_artboards_ruler_origin_goes_and_its_id_is_not_given_again() {
        let mut d = doc(vec![]);
        let kept = d.artboards[0].id;
        d.artboards.push(Artboard { id: 9, rect: Rect::new(0.0, 1e17, 100.0, 1e17 + 100.0), ..d.artboards[0].clone() });
        d.artboard_ruler_origins.insert(kept, vectorcraft_geom::Vec2::new(1.0, 2.0));
        d.artboard_ruler_origins.insert(9, vectorcraft_geom::Vec2::new(3.0, 4.0));
        assert_eq!(d.drop_beyond_canvas().artboards, 1);
        assert_eq!(d.artboard_ruler_origins.keys().copied().collect::<Vec<_>>(), [kept]);
        assert!(d.next_artboard_id() > 9, "a new artboard doesn't take the removed one's id");
    }

    #[test]
    fn what_names_a_removed_object_forgets_it() {
        let probe = Document::new(1.0, 1.0);
        let sub = sublayer(&probe, 40, vec![rect(41, 1e17), rect(42, 0.0)], true);
        let mut d = doc(vec![rect(10, 0.0), rect(11, 1e17), sub]);
        d.saved_guides =
            vec![Guide { layer: Some(NodeId(40)), ..Guide::new(false, 5.0) }, Guide { layer: Some(d.layers[0].id), ..Guide::new(true, 5.0) }];
        d.saved_selections = vec![crate::records::SavedSelection { name: "s".into(), objects: vec![NodeId(10), NodeId(11)] }].into();
        d.assets = vec![
            crate::assets::ExportAsset { id: 1, name: "far".into(), nodes: vec![NodeId(11)], format_3d: None },
            crate::assets::ExportAsset { id: 2, name: "both".into(), nodes: vec![NodeId(10), NodeId(11)], format_3d: None },
        ];
        d.variables.bindings.insert(NodeId(11), ["v".to_string()].into());
        d.variables.bindings.insert(NodeId(10), ["w".to_string()].into());
        // The same guides as nodes, as a document has them once read.
        let mut nodes = d.clone();
        assert_eq!(nodes.guides_into_nodes(), 2);
        let gone = d.drop_beyond_canvas();
        assert_eq!((gone.objects, gone.guides), (2, 1), "the far rectangle, the clipping sublayer and its layer guide");
        assert_eq!(nodes.drop_beyond_canvas(), gone, "a guide node goes with its sublayer, counted as a saved one is");
        assert_eq!(nodes.ruler_guides().len(), 1);
        assert_eq!(d.saved_guides.len(), 1, "the guide of a layer that stays, stays");
        assert_eq!(d.saved_selections[0].objects, [NodeId(10)]);
        assert_eq!(d.assets.iter().map(|a| (a.id, a.nodes.clone())).collect::<Vec<_>>(), [(2, vec![NodeId(10)])]);
        assert_eq!(d.variables.bindings.keys().copied().collect::<Vec<_>>(), [NodeId(10)]);
    }

    #[test]
    fn guides_of_many_removed_artboards_and_layers_go_and_the_rest_keep_their_order() {
        let probe = Document::new(1.0, 1.0);
        // Sublayers 40 to 43 clip by a path beyond the canvas, so they go whole; 50 stays.
        let far: Vec<Arc<Node>> = (40..44).map(|id| sublayer(&probe, id, vec![rect(id * 10, 1e17), rect(id * 10 + 1, 0.0)], true)).collect();
        let near = sublayer(&probe, 50, vec![rect(500, 0.0)], false);
        let mut d = doc(far.into_iter().chain([near, rect(10, 0.0)]).collect());
        let kept = d.artboards[0].id;
        for id in 9..13 {
            d.artboards.push(Artboard { id, rect: Rect::new(0.0, 1e17, 100.0, 1e17 + 100.0), ..d.artboards[0].clone() });
        }
        let top = d.layers[0].id;
        let guide = |pos: f64, artboard: Option<u32>, layer: Option<NodeId>| Guide { artboard, layer, ..Guide::new(false, pos) };
        d.saved_guides = vec![
            guide(1.0, Some(9), None),
            guide(2.0, None, None),
            guide(3.0, Some(kept), Some(NodeId(41))),
            guide(4.0, Some(kept), None),
            guide(5.0, Some(12), None),
            guide(6.0, None, Some(NodeId(50))),
            guide(7.0, None, Some(NodeId(43))),
            guide(8.0, Some(10), Some(top)),
            guide(9.0, None, Some(top)),
        ];
        d.saved_selections = vec![crate::records::SavedSelection { name: "s".into(), objects: vec![NodeId(42), NodeId(10), NodeId(500)] }].into();
        let mut nodes = d.clone();
        assert_eq!(nodes.guides_into_nodes(), 9);
        let gone = d.drop_beyond_canvas();
        assert_eq!(gone, BeyondCanvas { objects: 4, artboards: 4, guides: 5, ruler_origin: false });
        assert_eq!(d.artboards.iter().map(|a| a.id).collect::<Vec<_>>(), [kept]);
        let left: Vec<f64> = d.saved_guides.iter().map(|g| g.pos).collect();
        assert_eq!(left, [2.0, 4.0, 6.0, 9.0], "the guides that stay keep their order");
        assert_eq!(d.saved_selections[0].objects, [NodeId(10), NodeId(500)]);
        assert_eq!(nodes.drop_beyond_canvas(), gone, "guide nodes go as saved guides do");
        let mut left: Vec<f64> = nodes.ruler_guides().iter().map(|g| g.pos).collect();
        left.sort_by(f64::total_cmp);
        assert_eq!(left, [2.0, 4.0, 6.0, 9.0]);
        assert_eq!(nodes.saved_selections, d.saved_selections);
    }

    thread_local! {
        static COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// An id that counts how often it is compared.
    #[derive(Debug)]
    struct Counted(u32);

    impl Ord for Counted {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            COMPARISONS.with(|c| c.set(c.get() + 1));
            self.0.cmp(&other.0)
        }
    }

    impl PartialOrd for Counted {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl PartialEq for Counted {
        fn eq(&self, other: &Self) -> bool {
            self.cmp(other) == std::cmp::Ordering::Equal
        }
    }

    impl Eq for Counted {}

    #[test]
    fn a_guide_is_checked_without_scanning_every_removed_id() {
        let per_guide = |removed: u32| {
            let ids: BTreeSet<Counted> = (0..removed).map(|i| Counted(i * 2)).collect();
            COMPARISONS.with(|c| c.set(0));
            // Guides on removed ids, on ids between them, on ids past them and on none.
            let guides: Vec<Option<Counted>> = (0..64).map(|i| (i % 4 != 3).then(|| Counted(i * 31 % (removed * 2 + 8)))).collect();
            let kept = guides.into_iter().filter(|g| not_in(g.as_ref().map(|c| Counted(c.0)), &ids)).count();
            assert!(kept > 0 && kept < 64);
            COMPARISONS.with(|c| c.get()) as f64 / 64.0
        };
        let small = per_guide(16);
        let large = per_guide(16_384);
        // A sorted lookup grows with the depth of the tree; a scan of 16,384 ids compares thousands per guide.
        assert!(large < small * 32.0 && large < 256.0, "{small} comparisons per guide among 16 ids, {large} among 16,384");
    }

    #[test]
    fn each_object_is_measured_once_however_deep_the_nesting() {
        // 32 groups, each inside the one before and holding an object within the canvas; the
        // innermost also holds one beyond it. Beside them, a group with nothing beyond and a clip
        // group whose clipping path reaches beyond.
        let mut chain = Arc::new(Node::group(NodeId(100), vec![rect(200, 0.0), rect(201, 1e17)]));
        for depth in 1..32 {
            chain = Arc::new(Node::group(NodeId(100 + depth), vec![rect(300 + depth, 0.0), chain]));
        }
        let still = Arc::new(Node::group(NodeId(400), vec![rect(401, 0.0), rect(402, 5.0)]));
        let clip = Arc::new(Node::new(NodeId(600), NodeKind::Group { children: vec![rect(601, 1e17), rect(602, 0.0), rect(603, 0.0)], clip: true }));
        let mut d = doc(vec![chain, Arc::clone(&still), clip]);
        let mut whole = d.clone();
        let measured = std::cell::Cell::new(0_usize);
        let far = |n: &Node| {
            measured.set(measured.get() + 1);
            beyond(n)
        };
        let layer = &mut d.layers[0];
        let found = reach(layer, &far).unwrap();
        assert_eq!(
            measured.get(),
            36,
            "the 33 objects in the groups, the 2 beside them and the clipping path, each measured once; not what the path clips"
        );
        assert!(!clips_beyond(layer, &found));
        let (mut dropped, mut layers, mut guides) = (0, BTreeSet::new(), 0);
        drop_children_beyond(layer, found, &mut dropped, &mut layers, &mut guides);
        assert_eq!(measured.get(), 36, "removing measures nothing again");
        assert_eq!((dropped, layers.len(), guides), (2, 0, 0));
        assert!(d.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[1], &still)), "a group with nothing beyond is not copied");
        assert_eq!(whole.drop_beyond_canvas(), BeyondCanvas { objects: 2, ..Default::default() });
        assert_eq!(ids(&whole), ids(&d));
        assert!(!ids(&d).contains(&201) && !ids(&d).contains(&600) && ids(&d).contains(&200) && ids(&d).contains(&331));
    }

    #[test]
    fn each_object_is_looked_at_once_when_guides_go_from_nested_layers() {
        let guide = |id: u64, pos: f64| Arc::new(Node::new(NodeId(id), NodeKind::RulerGuide { vertical: false, pos, artboard: None }));
        let probe = Document::new(1.0, 1.0);
        // 32 sublayers, each inside the one before, each holding a guide within the canvas and one
        // beyond it. Beside them, a sublayer with nothing to remove.
        let mut chain = sublayer(&probe, 100, vec![guide(200, 5.0), guide(201, 1e17)], false);
        for depth in 1..32 {
            chain = sublayer(&probe, 100 + depth, vec![guide(300 + depth * 2, 5.0), guide(301 + depth * 2, 1e17), chain], false);
        }
        let still = sublayer(&probe, 500, vec![guide(501, 5.0), rect(502, 0.0)], false);
        let mut d = doc(vec![chain, Arc::clone(&still)]);
        let looked = std::cell::Cell::new(0_usize);
        let out = |n: &Node| {
            looked.set(looked.get() + 1);
            matches!(n.kind, NodeKind::RulerGuide { pos, .. } if !in_range(pos))
        };
        let mut dropped = 0;
        drop_guides(&mut d.layers[0], &out, &mut dropped);
        assert_eq!(dropped, 32);
        assert_eq!(looked.get(), 2 + 2 + 31 * 3 + 2, "each object in the layers is looked at once");
        assert!(d.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[1], &still)), "a sublayer with nothing to remove is not copied");
        let left = d.ruler_guides();
        assert_eq!(left.len(), 33);
        assert!(left.iter().all(|g| g.pos == 5.0));
    }
}
