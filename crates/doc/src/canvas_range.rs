//! A read document brought within the canvas: what lies beyond [`MAX_COORD`] is removed (#3575).
//! Edits refuse to leave anything there (`doc_sane` in the engine checks the artboards, the guides,
//! the ruler origin and the objects an edit touches), so a file holding something beyond it could be
//! opened but not edited: one object, artboard or guide out there made every move fail.

use std::sync::Arc;

use vectorcraft_geom::{Point, Rect};

use crate::{Document, MAX_COORD, Node, NodeId, NodeKind};

/// What [`Document::drop_beyond_canvas`] removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BeyondCanvas {
    /// Objects removed whole (a layer or group keeps its children within the canvas).
    pub objects: usize,
    pub artboards: usize,
    /// Guides beyond the canvas, and the artboard guides of the artboards removed.
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

/// Does `n`, or any object in it, reach beyond the canvas? A container is looked into object by
/// object: its own bounds leave out a layer's hidden objects and, when it clips, all but its
/// clipping path.
fn reaches_beyond(n: &Node) -> bool {
    match clipping(n) {
        None => beyond(n),
        Some(_) => n.children().is_some_and(|c| c.iter().any(|c| reaches_beyond(c))),
    }
}

/// Does `n` clip by a path that reaches beyond the canvas? Then the path and the objects it clips
/// go together, so no other object becomes the clipping path.
fn clips_beyond(n: &Node) -> bool {
    clipping(n) == Some(true) && n.children().and_then(|c| c.first()).is_some_and(|c| reaches_beyond(c))
}

/// Add the ids of `n` and the layers in it, if it is a layer, to `layers`: their layer guides go
/// with them.
fn layers_in(n: &Node, layers: &mut Vec<NodeId>) {
    if matches!(n.kind, NodeKind::Layer { .. }) {
        layers.push(n.id);
    }
    if clipping(n).is_some() {
        for c in n.children().into_iter().flatten() {
            layers_in(c, layers);
        }
    }
}

/// Remove the objects in `node` that reach beyond the canvas: a layer or group keeps those of its
/// objects within it, unless its clipping path reaches beyond (see [`clips_beyond`]); anything
/// else goes whole. Only containers holding such an object are copied. Recursion follows the
/// nesting, which [`Document::limit_depth`] bounds. The layers removed are added to `layers`.
fn drop_children_beyond(node: &mut Arc<Node>, dropped: &mut usize, layers: &mut Vec<NodeId>) {
    if !node.children().is_some_and(|c| c.iter().any(|c| reaches_beyond(c))) {
        return;
    }
    let Some(children) = Arc::make_mut(node).children_mut() else { return };
    children.retain_mut(|c| {
        if !reaches_beyond(c) {
            return true;
        }
        if clipping(c).is_some() && !clips_beyond(c) {
            drop_children_beyond(c, dropped, layers);
            return true;
        }
        layers_in(c, layers);
        *dropped += 1;
        false
    });
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
        let mut layers = vec![];
        for layer in &mut self.layers {
            if clips_beyond(layer) {
                let layer = Arc::make_mut(layer);
                if let NodeKind::Layer { children, clip, .. } = &mut layer.kind {
                    for c in children.iter() {
                        layers_in(c, &mut layers);
                    }
                    gone.objects += children.len();
                    children.clear();
                    *clip = false;
                }
            } else {
                drop_children_beyond(layer, &mut gone.objects, &mut layers);
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
        let guides = self.guides.len();
        self.guides.retain(|g| in_range(g.pos) && g.artboard.is_none_or(|a| !removed.contains(&a)) && g.layer.is_none_or(|l| !layers.contains(&l)));
        gone.guides = guides - self.guides.len();
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
        d.guides = vec![guide(50.0, None), guide(1e17, None), guide(f64::INFINITY, None), guide(20.0, Some(9))];
        d.ruler_origin = Point::new(1e17, 0.0);
        let gone = d.drop_beyond_canvas();
        assert_eq!(gone, BeyondCanvas { objects: 0, artboards: 1, guides: 3, ruler_origin: true });
        assert_eq!(d.artboards.len(), 1);
        assert_eq!(d.guides, [guide(50.0, None)], "the far artboard's own guide goes with it");
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
        d.guides = vec![Guide { layer: Some(NodeId(40)), ..Guide::new(false, 5.0) }, Guide { layer: Some(d.layers[0].id), ..Guide::new(true, 5.0) }];
        d.saved_selections = vec![crate::records::SavedSelection { name: "s".into(), objects: vec![NodeId(10), NodeId(11)] }].into();
        d.assets = vec![
            crate::assets::ExportAsset { id: 1, name: "far".into(), nodes: vec![NodeId(11)] },
            crate::assets::ExportAsset { id: 2, name: "both".into(), nodes: vec![NodeId(10), NodeId(11)] },
        ];
        d.variables.bindings.insert(NodeId(11), ["v".to_string()].into());
        d.variables.bindings.insert(NodeId(10), ["w".to_string()].into());
        let gone = d.drop_beyond_canvas();
        assert_eq!((gone.objects, gone.guides), (2, 1), "the far rectangle, the clipping sublayer and its layer guide");
        assert_eq!(d.guides.len(), 1, "the guide of a layer that stays, stays");
        assert_eq!(d.saved_selections[0].objects, [NodeId(10)]);
        assert_eq!(d.assets.iter().map(|a| (a.id, a.nodes.clone())).collect::<Vec<_>>(), [(2, vec![NodeId(10)])]);
        assert_eq!(d.variables.bindings.keys().copied().collect::<Vec<_>>(), [NodeId(10)]);
    }
}
