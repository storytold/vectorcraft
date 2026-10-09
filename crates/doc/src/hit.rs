//! Hit testing over the document tree.

use vectorcraft_geom::hit::{fill_contains, stroke_contains};
use vectorcraft_geom::{Point, Rect};

use crate::{Document, Node, NodeId, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Fill,
    Stroke,
    /// Hit a path outline in outline mode, or an unfilled path's edge.
    Outline,
    Bounds,
}

/// The result of a hit test: the leaf that was hit and its ancestry (layer first).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub leaf: NodeId,
    pub ancestry: Vec<NodeId>,
    pub kind: HitKind,
    /// The innermost envelope in the ancestry whose contents are being edited (Edit Contents):
    /// its content, not the envelope, is the object clicked.
    pub contents_of: Option<NodeId>,
    /// How many entries at the start of `ancestry` are layers (the layer and its sublayers).
    pub layers: usize,
}

impl Hit {
    /// The object the Selection tool selects: the child of the layer (the outermost group).
    /// Inside isolation mode (`scope`), the child of the isolated container instead.
    pub fn top_object(&self, scope: Option<NodeId>) -> NodeId {
        // The innermost of the isolated container and an envelope whose contents are edited.
        let at = |s: Option<NodeId>| s.and_then(|s| self.ancestry.iter().position(|x| *x == s));
        if let Some(i) = at(scope).max(at(self.contents_of)) {
            return self.ancestry.get(i + 1).copied().unwrap_or(self.leaf);
        }
        // Skip layers and sublayers: they are not objects.
        self.ancestry.get(self.layers.max(1)).copied().unwrap_or(self.leaf)
    }
}

/// Options for hit testing.
#[derive(Clone, Copy, Debug)]
pub struct HitOptions {
    /// The isolated construction container, whose retained Shaper sources are hittable.
    pub scope: Option<NodeId>,
    /// Tolerance in document units (screen tolerance / zoom).
    pub tol: f64,
    /// Outline mode: only outlines are hittable.
    pub outline: bool,
    /// "Object Selection by Path Only" preference.
    pub path_only: bool,
    /// "Type Object Selection by Path Only" preference: type is picked on its type path only
    /// (point type's baseline, area type's frame, type on a path's path), not anywhere in its
    /// bounds.
    pub type_path_only: bool,
}

impl Default for HitOptions {
    fn default() -> Self {
        Self { tol: 3.0, outline: false, path_only: false, type_path_only: false, scope: None }
    }
}

/// Topmost editable object under `p`.
pub fn hit_test(doc: &Document, p: Point, opt: HitOptions) -> Option<Hit> {
    hit_test_skipping(doc, p, opt, &|_| false)
}

/// A selected compound-shape member that is the topmost member of its compound shape with `p`
/// inside its own region or on its outline, even where it is a hole in the compound's outline (a
/// subtracted member): a selected member is pressed and dragged from anywhere in it, as a plain
/// object is. A member above it there takes the press instead (it isn't this one's to drag).
/// Hidden or locked members, or ones inside a hidden or locked container, don't count.
pub fn selected_member_at(doc: &Document, p: Point, opt: HitOptions, selected: &[NodeId]) -> Option<Hit> {
    doc.paint_order(selected.iter().copied()).into_iter().rev().find_map(|id| {
        let parent = doc.node(doc.parent_of(id)?)?;
        let NodeKind::CompoundShape { children } = &parent.kind else { return None };
        // The topmost visible member there must be this one.
        let top = children.iter().rev().find(|m| m.visible && member_at(m, p, opt.tol))?;
        if top.id != id {
            return None;
        }
        let m = &**top;
        let ancestry = doc.ancestry(id)?;
        if ancestry.iter().any(|a| doc.node(*a).is_none_or(|n| !n.visible || n.locked || n.is_template())) {
            return None;
        }
        let layers = ancestry.iter().take_while(|a| doc.node(**a).is_some_and(Node::is_layer)).count();
        let kind = if member_edge(m, p, opt.tol) { HitKind::Outline } else { HitKind::Fill };
        Some(Hit { leaf: id, ancestry, kind, contents_of: None, layers })
    })
}

/// The objects under `p` that [`Hit::top_object`] picks in `scope` (isolation mode), topmost
/// first: what clicks there select, one below the other (Cmd/Ctrl-click selects behind).
pub fn objects_at(doc: &Document, p: Point, opt: HitOptions, scope: Option<NodeId>) -> Vec<NodeId> {
    let opt = HitOptions { scope: scope.or(opt.scope), ..opt };
    let mut tops: Vec<NodeId> = vec![];
    // Each round leaves out the objects found so far (and everything inside them).
    while let Some(top) = hit_test_skipping(doc, p, opt, &|id| tops.contains(&id)).map(|h| h.top_object(scope)) {
        if tops.contains(&top) {
            break;
        }
        tops.push(top);
    }
    tops
}

/// [`hit_test`] passing over the nodes `skip` names (and their contents).
fn hit_test_skipping(doc: &Document, p: Point, opt: HitOptions, skip: &dyn Fn(NodeId) -> bool) -> Option<Hit> {
    let mut chain = Vec::new();
    for layer in doc.layers.iter().rev() {
        if !layer.visible || layer.locked {
            continue;
        }
        if layer.is_template() {
            continue;
        }
        // Pattern editing mode: only the tile art is editable.
        if doc.pattern_edit.as_ref().is_some_and(|e| e.layer != layer.id) {
            continue;
        }
        chain.push(layer.id);
        let opt = if matches!(layer.kind, NodeKind::Layer { preview: false, .. }) { HitOptions { outline: true, ..opt } } else { opt };
        if let Some(mut h) = hit_children(layer, p, opt, skip, &mut chain) {
            h.contents_of = h.ancestry.iter().rev().copied().find(|a| doc.node(*a).is_some_and(edits_contents));
            h.layers = h.ancestry.iter().take_while(|a| doc.node(**a).is_some_and(Node::is_layer)).count();
            return Some(h);
        }
        chain.pop();
    }
    None
}

/// Is `n` an envelope whose contents are being edited (they hit, not the envelope)?
fn edits_contents(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Envelope { editing: true, .. })
}

/// Is `p` inside the region `clip` clips to ([`Node::clip_shapes`])? Text counts by its frame:
/// glyph outlines need the font engine, above this crate.
fn clip_contains(clip: &Node, p: Point) -> bool {
    let frame = |n: &Node| {
        let b = n.geometric_bounds()?;
        Some(Node::path(n.id, vectorcraft_geom::shapes::rectangle(b), Default::default()))
    };
    clip.clip_shapes(Some(&frame)).iter().any(|(bp, rule)| fill_contains(bp, *rule, p))
}

fn hit_children(parent: &Node, p: Point, opt: HitOptions, skip: &dyn Fn(NodeId) -> bool, chain: &mut Vec<NodeId>) -> Option<Hit> {
    let children = parent.children()?;
    // A clip group (or a layer with a clipping mask) only hits inside its clipping path.
    if parent.clips()
        && let Some(clip) = children.first()
        && !clip_contains(clip, p)
    {
        return None;
    }
    for (i, c) in children.iter().enumerate().rev() {
        if parent.shaper.is_some() {
            let construction = children.first().is_some_and(|source| Some(source.id) == opt.scope);
            if (i == 0) != construction {
                continue;
            }
        }
        // Hidden, locked and template sublayers block clicks like top-level layers do.
        if !c.visible || c.locked || c.is_template() || skip(c.id) {
            continue;
        }
        // (An envelope's content sits where it was, not where the envelope draws it.)
        if !(edits_contents(c)
            || isolated_in(c, opt.scope)
            || c.shaper.is_some() && c.children().and_then(|children| children.first()).is_some_and(|source| Some(source.id) == opt.scope))
            && let Some(b) = c.reach_bounds()
            && !b.inflate(opt.tol, opt.tol).contains(p)
        {
            continue;
        }
        chain.push(c.id);
        let hit = match &c.kind {
            // A sublayer whose Preview is off is clicked in outline, like an outline view.
            NodeKind::Layer { preview: false, .. } => hit_children(c, p, HitOptions { outline: true, ..opt }, skip, chain),
            NodeKind::Layer { .. } | NodeKind::Group { .. } => hit_children(c, p, opt, skip, chain),
            // Edit Contents: the envelope's content hits, undistorted.
            NodeKind::Envelope { editing: true, .. } => hit_children(c, p, opt, skip, chain),
            // A blend's key objects first (Direct and Group Selection pick them); its steps hit
            // as the blend.
            NodeKind::Blend { .. } => hit_children(c, p, opt, skip, chain)
                .or_else(|| hit_leaf(c, p, opt).map(|kind| Hit { leaf: c.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 })),
            // A compound shape hits on its outline: then the member under the point, for Direct
            // and Group Selection (the Selection tool takes the compound shape). Isolated (or
            // with something inside it isolated), its members are objects of their own: each hits
            // anywhere in its own shape, whatever its mode (a subtracted one where it cuts away).
            NodeKind::CompoundShape { children } => {
                let isolated = isolated_in(c, opt.scope);
                let kind = if isolated {
                    children.iter().any(|m| m.visible && member_at(m, p, opt.tol)).then_some(HitKind::Fill)
                } else {
                    compound_shape_hit(c, p, opt)
                };
                kind.and_then(|kind| {
                    let member = children.iter().rev().find(|m| m.visible && !m.locked && !skip(m.id) && member_at(m, p, opt.tol));
                    let hit = match member {
                        Some(m) => {
                            chain.push(m.id);
                            let inner = if m.is_container() { hit_children(m, p, opt, skip, chain) } else { None };
                            let hit = inner.unwrap_or_else(|| Hit { leaf: m.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 });
                            chain.pop();
                            hit
                        }
                        // Isolated, only a member is something to pick.
                        None if isolated => return None,
                        None => Hit { leaf: c.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 },
                    };
                    Some(hit)
                })
            }
            _ => hit_leaf(c, p, opt).map(|kind| Hit { leaf: c.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 }),
        };
        if hit.is_some() {
            return hit;
        }
        chain.pop();
    }
    None
}

/// Is `scope` (the isolated container) `n` or inside it?
fn isolated_in(n: &Node, scope: Option<NodeId>) -> bool {
    let Some(s) = scope else { return false };
    let mut found = false;
    n.walk(&mut |c| found |= c.id == s);
    found
}

/// Is `p` inside member `m` of a compound shape (its region, whatever it paints)?
fn member_inside(m: &Node, p: Point) -> bool {
    if !m.visible {
        return false;
    }
    match &m.kind {
        NodeKind::Path { guide: true, .. } => false,
        NodeKind::Path { path, rule, .. } => fill_contains(&path.to_bezpath(), *rule, p),
        NodeKind::Compound { rule, .. } => m.stroke_path().is_some_and(|bp| fill_contains(&bp, *rule, p)),
        NodeKind::CompoundShape { children } => compound_inside(children, p),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => match m.clips() {
            true => children.first().is_some_and(|c| member_inside(c, p)),
            false => children.iter().any(|c| member_inside(c, p)),
        },
        _ => m.geometric_bounds().is_some_and(|b| b.contains(p)),
    }
}

/// Is `p` within `tol` of the outline of member `m`?
fn member_edge(m: &Node, p: Point, tol: f64) -> bool {
    if !m.visible {
        return false;
    }
    match &m.kind {
        NodeKind::Path { .. } | NodeKind::Compound { .. } => m.stroke_path().is_some_and(|bp| stroke_contains(&bp, 0.0, tol, p)),
        _ => m.children().is_some_and(|ch| ch.iter().any(|c| member_edge(c, p, tol))),
    }
}

/// Is `p` inside or on member `m`?
fn member_at(m: &Node, p: Point, tol: f64) -> bool {
    member_inside(m, p) || member_edge(m, p, tol)
}

/// Is `p` inside the outline of a compound shape with these members (their modes folded)?
fn compound_inside(children: &[std::sync::Arc<Node>], p: Point) -> bool {
    let (inside, modes): (Vec<bool>, Vec<crate::ShapeMode>) =
        children.iter().filter(|m| m.visible).map(|m| (member_inside(m, p), m.shape_mode)).unzip();
    crate::ShapeMode::fold(&inside, &modes)
}

/// A click on compound shape `n`: inside its outline (unless only outlines hit), or on one of its
/// members' outlines (where its own outline and stroke run, or the members show in outline mode).
fn compound_shape_hit(n: &Node, p: Point, opt: HitOptions) -> Option<HitKind> {
    let NodeKind::CompoundShape { children } = &n.kind else { return None };
    if children.iter().any(|m| member_edge(m, p, opt.tol + n.appearance.outset())) {
        return Some(HitKind::Outline);
    }
    (!opt.outline && !opt.path_only && compound_inside(children, p)).then_some(HitKind::Fill)
}

/// A click on the strokes of a path or compound path `n` along `bp`: what any visible stroke
/// paints there (arrowheads, alignment, width profile, projecting caps, miter spikes), else its
/// outline within the tolerance. Outline mode hits the outline only.
fn hit_stroke(n: &Node, bp: &vectorcraft_geom::BezPath, rule: vectorcraft_geom::FillRule, p: Point, opt: HitOptions) -> Option<HitKind> {
    if !opt.outline && n.appearance.stroke_hit(bp, rule, p, opt.tol) {
        return Some(HitKind::Stroke);
    }
    stroke_contains(bp, 0.0, opt.tol, p).then_some(HitKind::Outline)
}

fn hit_leaf(n: &Node, p: Point, opt: HitOptions) -> Option<HitKind> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => {
            let bp = path.to_bezpath();
            if let Some(k) = hit_stroke(n, &bp, *rule, p, opt) {
                return Some(k);
            }
            let filled = !n.appearance.fill_paint().is_none();
            if !opt.outline && !opt.path_only && filled && fill_contains(&bp, *rule, p) {
                return Some(HitKind::Fill);
            }
            None
        }
        NodeKind::Compound { rule, .. } => {
            let bp = n.stroke_path()?;
            if let Some(k) = hit_stroke(n, &bp, *rule, p, opt) {
                return Some(k);
            }
            (!opt.outline && !opt.path_only && fill_contains(&bp, *rule, p)).then_some(HitKind::Fill)
        }
        // The type path lies within the bounds: they rule out the rest before it is measured.
        NodeKind::Text(t) if opt.type_path_only => {
            n.geometric_bounds().filter(|b| b.inflate(opt.tol, opt.tol).contains(p))?;
            on_type_path(n, t, p, opt.tol).then_some(HitKind::Outline)
        }
        NodeKind::Text(_)
        | NodeKind::Image(_)
        | NodeKind::PlacedDocument(_)
        | NodeKind::SymbolInstance { .. }
        | NodeKind::Blend { .. }
        | NodeKind::Envelope { .. } => n.geometric_bounds().filter(|b| b.inflate(opt.tol, opt.tol).contains(p)).map(|_| HitKind::Bounds),
        NodeKind::Repeat(r) => r.expand().iter().find_map(|g| hit_any(g, p, opt)),
        NodeKind::CompoundShape { .. } => compound_shape_hit(n, p, opt),
        NodeKind::Mesh(m) => {
            let bp = m.outline().to_bezpath();
            if stroke_contains(&bp, 0.0, opt.tol, p) {
                return Some(HitKind::Outline);
            }
            (!opt.path_only && fill_contains(&bp, vectorcraft_geom::FillRule::NonZero, p)).then_some(HitKind::Fill)
        }
        _ => None,
    }
}

/// Is `p` within `tol` of the type path of `t` (in `n`), what "Type Object Selection by Path
/// Only" picks type by: its baselines ([`crate::TextObject::baselines`]), area type's frame and
/// type on a path's path, drawn through `n`'s perspective projection if it has one.
fn on_type_path(n: &Node, t: &crate::TextObject, p: Point, tol: f64) -> bool {
    use crate::TextKind;
    use vectorcraft_geom::kurbo::ParamCurveNearest;
    let h = n.projection();
    let project = |q: Point| h.as_ref().and_then(|h| h.apply(q)).unwrap_or(q);
    // A homography keeps straight lines straight: a baseline's ends are enough.
    let near = |(a, b): (Point, Point)| vectorcraft_geom::Line::new(project(t.xf * a), project(t.xf * b)).nearest(p, 1e-9).distance_sq <= tol * tol;
    if t.baselines().any(near) {
        return true;
    }
    let (TextKind::Area { frame: path } | TextKind::OnPath { path, .. }) = &t.kind else { return false };
    let mut path = path.transformed(t.xf);
    if h.is_some() {
        let piece = path.bounds().map_or(1.0, |b| (b.width().max(b.height()) / 64.0).max(0.25));
        path = crate::live::map_nonlinear(&path, piece, project);
    }
    stroke_contains(&path.to_bezpath(), 0.0, tol, p)
}

/// Hit anywhere in an evaluated subtree (groups recurse; leaves use [`hit_leaf`]).
fn hit_any(n: &Node, p: Point, opt: HitOptions) -> Option<HitKind> {
    match &n.kind {
        NodeKind::Group { children, .. } => children.iter().rev().find_map(|c| hit_any(c, p, opt)),
        _ => hit_leaf(n, p, opt),
    }
}

/// Objects (children of layers, or of `scope` in isolation mode) touched by a marquee rect.
pub fn marquee(doc: &Document, r: Rect, scope: Option<NodeId>, leaves: bool) -> Vec<NodeId> {
    let mut out = Vec::new();
    let tops: Vec<&std::sync::Arc<Node>> = match scope.and_then(|s| doc.node(s)) {
        Some(s) => s.children().map(|c| c.iter().collect()).unwrap_or_default(),
        // The objects of the layers, looking through sublayers (hidden, locked and template ones
        // are left out).
        None => doc
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked && !l.is_template() && doc.pattern_edit.as_ref().is_none_or(|e| e.layer == l.id))
            .flat_map(|l| layer_objects(l))
            .collect(),
    };
    fn layer_objects(l: &Node) -> Vec<&std::sync::Arc<Node>> {
        let mut out = vec![];
        for c in l.children().into_iter().flatten() {
            if c.is_layer() {
                if c.visible && !c.locked && !c.is_template() {
                    out.extend(layer_objects(c));
                }
            } else {
                out.push(c);
            }
        }
        out
    }
    fn touches(n: &Node, r: Rect) -> bool {
        match &n.kind {
            NodeKind::Path { path, .. } => vectorcraft_geom::hit::intersects_rect(path, r),
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::CompoundShape { children } => children.iter().any(|c| c.visible && touches(c, r)),
            _ => n.geometric_bounds().is_some_and(|b| b.intersect(r).area() > 0.0 || r.contains(b.origin())),
        }
    }
    fn collect_leaves(n: &Node, r: Rect, out: &mut Vec<NodeId>) {
        match &n.kind {
            // Direct Selection reaches a compound shape's members.
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::CompoundShape { children } => {
                for c in children {
                    if c.visible && !c.locked {
                        collect_leaves(c, r, out);
                    }
                }
            }
            _ => {
                if touches(n, r) {
                    out.push(n.id);
                }
            }
        }
    }
    for t in tops {
        if !t.visible || t.locked {
            continue;
        }
        if leaves {
            collect_leaves(t, r, &mut out);
        } else if touches(t, r) {
            out.push(t.id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Node};
    use std::sync::Arc;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_geom::shapes;

    #[test]
    fn hits_topmost() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(Some(l), 9, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
        let b = d.alloc_id();
        d.insert(Some(l), 9, Node::path(b, shapes::rectangle(Rect::new(25.0, 25.0, 75.0, 75.0)), Appearance::default_art())).unwrap();
        let h = hit_test(&d, Point::new(30.0, 30.0), HitOptions::default()).unwrap();
        assert_eq!(h.leaf, b);
        assert_eq!(h.kind, HitKind::Fill);
        let h = hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).unwrap();
        assert_eq!(h.leaf, a);
        assert!(hit_test(&d, Point::new(150.0, 150.0), HitOptions::default()).is_none());
        // Outline mode: interior misses.
        assert!(hit_test(&d, Point::new(10.0, 10.0), HitOptions { outline: true, ..Default::default() }).is_none());
        assert!(hit_test(&d, Point::new(0.5, 10.0), HitOptions { outline: true, ..Default::default() }).is_some());
    }

    #[test]
    fn unfilled_interior_misses() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(
            Some(l),
            9,
            Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0)),
        )
        .unwrap();
        assert!(hit_test(&d, Point::new(25.0, 25.0), HitOptions::default()).is_none());
        assert!(hit_test(&d, Point::new(50.5, 25.0), HitOptions::default()).is_some());
    }

    #[test]
    fn group_top_object() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        let g = d.alloc_id();
        let p = Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art());
        d.insert(Some(l), 0, Node::group(g, vec![Arc::new(p)])).unwrap();
        let h = hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).unwrap();
        assert_eq!(h.leaf, a);
        assert_eq!(h.top_object(None), g);
        assert_eq!(h.top_object(Some(g)), a);
        assert_eq!(marquee(&d, Rect::new(-5.0, -5.0, 5.0, 5.0), None, false), vec![g]);
        assert_eq!(marquee(&d, Rect::new(-5.0, -5.0, 5.0, 5.0), None, true), vec![a]);
        assert!(marquee(&d, Rect::new(100.0, 100.0, 105.0, 105.0), None, false).is_empty());
    }

    /// Art in a sublayer is an object of its own: a click or a marquee takes it, not the
    /// sublayer; hidden, locked and template sublayers block both.
    #[test]
    fn sublayers_are_not_objects() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let sub = d.alloc_id();
        d.insert(Some(l), 0, Node::layer(sub, "Sub", crate::LayerColor::Preset(2))).unwrap();
        let inner = d.alloc_id();
        d.insert(Some(sub), 0, Node::layer(inner, "Inner", crate::LayerColor::Preset(3))).unwrap();
        let a = d.alloc_id();
        d.insert(Some(inner), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
        let g = d.alloc_id();
        let b = d.alloc_id();
        let pb = Node::path(b, shapes::rectangle(Rect::new(100.0, 0.0, 150.0, 50.0)), Appearance::default_art());
        d.insert(Some(sub), 9, Node::group(g, vec![Arc::new(pb)])).unwrap();
        let h = hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).unwrap();
        assert_eq!((h.leaf, h.top_object(None), h.layers), (a, a, 3));
        let h = hit_test(&d, Point::new(110.0, 10.0), HitOptions::default()).unwrap();
        assert_eq!((h.top_object(None), h.top_object(Some(g))), (g, b));
        let all = Rect::new(-5.0, -5.0, 200.0, 200.0);
        assert_eq!(marquee(&d, all, None, false), vec![a, g]);
        assert_eq!(d.layer_containing(a), Some(inner));
        assert_eq!(d.layer_color(a), crate::LayerColor::Preset(3).rgb(), "the sublayer's own colour");
        assert_eq!(d.selectable_art(), vec![a, g]);
        for f in [
            |n: &mut Node| n.visible = false,
            |n: &mut Node| n.locked = true,
            |n: &mut Node| {
                if let NodeKind::Layer { template, .. } = &mut n.kind {
                    *template = true;
                }
            },
        ] {
            let mut d2 = d.clone();
            f(d2.node_mut(inner).unwrap());
            assert!(hit_test(&d2, Point::new(10.0, 10.0), HitOptions::default()).is_none());
            assert_eq!(marquee(&d2, all, None, false), vec![g]);
            assert_eq!(d2.selectable_art(), vec![g]);
        }
        // A sublayer whose Preview is off is clicked in outline: its fill no longer hits.
        if let NodeKind::Layer { preview, .. } = &mut d.node_mut(inner).unwrap().kind {
            *preview = false;
        }
        assert!(hit_test(&d, Point::new(25.0, 25.0), HitOptions::default()).is_none());
        assert!(hit_test(&d, Point::new(0.5, 25.0), HitOptions::default()).is_some());
    }

    /// Object Selection by Path Only: a filled path or compound path hits on its outline only.
    #[test]
    fn path_only_leaves_out_fills() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let (a, c) = (d.alloc_id(), d.alloc_id());
        d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
        let inner = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 0.0, 150.0, 50.0)), Appearance::default());
        let mut compound = Node::new(c, NodeKind::Compound { children: vec![Arc::new(inner)], rule: vectorcraft_geom::FillRule::NonZero });
        compound.appearance = Appearance::default_art();
        d.insert(Some(l), 9, compound).unwrap();
        let path_only = HitOptions { path_only: true, ..Default::default() };
        for (id, inside, edge) in [(a, Point::new(25.0, 25.0), Point::new(0.5, 25.0)), (c, Point::new(125.0, 25.0), Point::new(100.5, 25.0))] {
            assert_eq!(hit_test(&d, inside, HitOptions::default()).map(|h| h.leaf), Some(id), "the fill hits");
            assert!(hit_test(&d, inside, path_only).is_none(), "path only: the fill of {id:?} doesn't hit");
            assert_eq!(hit_test(&d, edge, path_only).map(|h| h.leaf), Some(id), "path only: the outline does");
        }
    }

    /// Type Object Selection by Path Only: point type is picked on its baseline, area type on its
    /// frame and type on a path on its path, no longer anywhere in its bounds.
    #[test]
    fn type_path_only_picks_type_on_its_path() {
        use crate::{CharStyle, TextKind, TextObject};
        let mut d = Document::new(400.0, 400.0);
        let l = d.layers[0].id;
        let mut add = |t: TextObject| {
            let id = d.alloc_id();
            d.insert(Some(l), 9, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
            id
        };
        // Point type at (10, 100), 20 pt: its glyphs rise above the baseline, its layout is 60 wide.
        let mut point = TextObject::point(Point::new(10.0, 100.0), "Hello", CharStyle { size: 20.0, ..CharStyle::default() });
        point.cached_bounds = Some(Rect::new(0.0, -16.0, 60.0, 4.0));
        let point = add(point);
        let mut area = TextObject::point(Point::new(200.0, 200.0), "Area", CharStyle::default());
        area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)) };
        let area = add(area);
        let mut on_path = TextObject::point(Point::new(10.0, 300.0), "Path", CharStyle::default());
        on_path.kind = TextKind::OnPath { path: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 40.0)), start: 0.0, end: None };
        let on_path = add(on_path);
        let path_only = HitOptions { type_path_only: true, ..Default::default() };
        let hit = |p: Point, opt: HitOptions| hit_test(&d, p, opt).map(|h| (h.leaf, h.kind));
        // Off: anywhere in the bounds.
        assert_eq!(hit(Point::new(40.0, 90.0), HitOptions::default()), Some((point, HitKind::Bounds)));
        assert_eq!(hit(Point::new(250.0, 225.0), HitOptions::default()), Some((area, HitKind::Bounds)));
        assert_eq!(hit(Point::new(60.0, 320.0), HitOptions::default()), Some((on_path, HitKind::Bounds)));
        // On: the glyphs and the inside of the frame or path no longer pick the type.
        assert_eq!(hit(Point::new(40.0, 90.0), path_only), None, "point type: among its glyphs");
        assert_eq!(hit(Point::new(250.0, 225.0), path_only), None, "area type: inside its frame");
        assert_eq!(hit(Point::new(60.0, 320.0), path_only), None, "type on a path: inside its path");
        // The baseline (within the tolerance, across the layout's width), frame and path do.
        assert_eq!(hit(Point::new(40.0, 101.5), path_only), Some((point, HitKind::Outline)), "point type: its baseline");
        assert_eq!(hit(Point::new(75.0, 100.0), path_only), None, "point type: past the end of its layout");
        assert_eq!(hit(Point::new(250.0, 200.5), path_only), Some((area, HitKind::Outline)), "area type: its frame");
        assert_eq!(hit(Point::new(60.0, 339.5), path_only), Some((on_path, HitKind::Outline)), "type on a path: its path");
    }

    /// Type Object Selection by Path Only with the layout's baselines: every line's for point type,
    /// area type's besides its frame, a vertical column's centre line, and type in perspective
    /// where it is drawn.
    #[test]
    fn type_path_only_picks_every_baseline() {
        use crate::{CharStyle, PerspectiveAttachment, TextKind, TextObject};
        use vectorcraft_geom::{Affine, Homography};
        let mut d = Document::new(400.0, 400.0);
        let l = d.layers[0].id;
        let mut add = |t: TextObject, perspective: Option<Affine>| {
            let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
            n.perspective = perspective.map(|a| {
                Box::new(PerspectiveAttachment { projection: Some(Homography::from_affine(a).to_array()), ..PerspectiveAttachment::new("left", 0.0) })
            });
            let id = n.id;
            d.insert(Some(l), 9, n).unwrap();
            id
        };
        let style = CharStyle { size: 20.0, ..CharStyle::default() };
        // Two lines of point type at (10, 100), 24 pt apart.
        let mut point = TextObject::point(
            Point::new(10.0, 100.0),
            "Hello
world",
            style.clone(),
        );
        point.cached_bounds = Some(Rect::new(0.0, -16.0, 60.0, 28.0));
        point.cached_baselines = vec![(Point::ZERO, Point::new(60.0, 0.0)), (Point::new(0.0, 24.0), Point::new(50.0, 24.0))];
        let point = add(point, None);
        // Area type: a line inside its frame.
        let mut area = TextObject::point(Point::new(200.0, 50.0), "Area", style.clone());
        area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)) };
        area.cached_baselines = vec![(Point::new(0.0, 16.0), Point::new(40.0, 16.0))];
        let area = add(area, None);
        // Vertical point type at (300, 200) with no baselines cached: down its first column.
        let mut vertical = TextObject::point(Point::new(300.0, 200.0), "Tate", style.clone());
        vertical.vertical = true;
        vertical.cached_bounds = Some(Rect::new(-10.0, 0.0, 10.0, 80.0));
        let vertical = add(vertical, None);
        // Point type drawn 100 to the right of where it is laid out.
        let mut moved = TextObject::point(Point::new(10.0, 350.0), "Moved", style);
        moved.cached_bounds = Some(Rect::new(0.0, -16.0, 60.0, 4.0));
        let moved = add(moved, Some(Affine::translate((100.0, 0.0))));
        let path_only = HitOptions { type_path_only: true, ..Default::default() };
        let hit = |p: Point| hit_test(&d, p, path_only).map(|h| h.leaf);
        assert_eq!(hit(Point::new(40.0, 100.0)), Some(point), "the first line's baseline");
        assert_eq!(hit(Point::new(40.0, 125.0)), Some(point), "the second line's");
        assert_eq!(hit(Point::new(40.0, 112.0)), None, "between the lines");
        assert_eq!(hit(Point::new(65.0, 124.0)), None, "past the end of the shorter second line");
        assert_eq!(hit(Point::new(220.0, 66.0)), Some(area), "area type: a line's baseline");
        assert_eq!(hit(Point::new(220.0, 80.0)), None, "area type: inside the frame, off the baselines");
        assert_eq!(hit(Point::new(299.0, 260.0)), Some(vertical), "vertical type: down the column");
        assert_eq!(hit(Point::new(306.0, 260.0)), None, "vertical type: beside it");
        assert_eq!(hit(Point::new(140.0, 351.0)), Some(moved), "in perspective: where it is drawn");
        assert_eq!(hit(Point::new(40.0, 351.0)), None, "in perspective: not where it is laid out");
    }

    /// `objects_at`: every object under the point, topmost first; a group counts once.
    #[test]
    fn objects_at_lists_the_stack() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let rect =
            |d: &mut Document, x: f64| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 50.0, 50.0)), Appearance::default_art());
        let a = rect(&mut d, 0.0);
        let (b, c) = (rect(&mut d, 10.0), rect(&mut d, 20.0));
        let (a_id, b_id, c_id, g) = (a.id, b.id, c.id, d.alloc_id());
        d.insert(Some(l), 0, a).unwrap();
        d.insert(Some(l), 9, Node::group(g, vec![Arc::new(b), Arc::new(c)])).unwrap();
        let far = rect(&mut d, 120.0);
        d.insert(Some(l), 9, far).unwrap();
        let at = |x: f64, scope| objects_at(&d, Point::new(x, 25.0), HitOptions::default(), scope);
        assert_eq!(at(30.0, None), vec![g, a_id]);
        assert_eq!(at(30.0, Some(g))[..2], [c_id, b_id], "isolated: the group's own objects");
        assert_eq!(at(5.0, None), vec![a_id]);
        assert!(at(90.0, None).is_empty());
    }

    #[test]
    fn locked_and_hidden_skip() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
        d.node_mut(a).unwrap().locked = true;
        assert!(hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).is_none());
        d.node_mut(a).unwrap().locked = false;
        d.node_mut(l).unwrap().visible = false;
        assert!(hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).is_none());
    }
}
