//! Hit testing over the document tree.

use std::collections::HashSet;

use vectorcraft_geom::hit::{fill_contains, stroke_contains};
use vectorcraft_geom::{Point, Rect};

use crate::{Document, Node, NodeId, NodeKind, SymbolBoxes};

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
        // A single object isolated: it is the only object.
        if scope == Some(self.leaf) {
            return self.leaf;
        }
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
    hit_test_skipping(doc, p, opt, &|_| false, true)
}

/// Topmost visible art for read-only sampling: locked objects and template layers participate,
/// including art outside the editing scope. This never changes their editability.
pub fn hit_test_visible(doc: &Document, p: Point, opt: HitOptions) -> Option<Hit> {
    hit_test_skipping(doc, p, opt, &|_| false, false)
}

/// The objects under `p` that [`Hit::top_object`] picks in `scope` (isolation mode), topmost
/// first: what clicks there select, one below the other (Cmd/Ctrl-click selects behind).
pub fn objects_at(doc: &Document, p: Point, opt: HitOptions, scope: Option<NodeId>) -> Vec<NodeId> {
    let opt = HitOptions { scope: scope.or(opt.scope), ..opt };
    // Normally one hit test per *object* suffices: layers and sublayers aren't selectable,
    // while a group (including a clipping group) is selected as one object. Repeatedly
    // searching the full tree for each hit made a stack of n overlapping paths O(n^2).
    // Isolation and Edit Contents change where selection groups begin; keep the original
    // traversal for those cases rather than change their Select Behind semantics.
    if opt.scope.is_some() {
        return objects_at_repeated(doc, p, opt, scope);
    }
    let mut tops = Vec::new();
    let mut seen = HashSet::new();
    let symbols = doc.symbol_boxes();
    for layer in doc.layers.iter().rev() {
        // Malformed files may contain non-layer roots; let the general traversal handle them.
        if !layer.is_layer() {
            return objects_at_repeated(doc, p, opt, scope);
        }
        if !layer.visible || layer.locked || layer.is_template() || doc.pattern_edit.as_ref().is_some_and(|e| e.layer != layer.id) {
            continue;
        }
        if !objects_at_layer(layer, p, opt, &symbols, &mut tops, &mut seen) {
            // An envelope in Edit Contents may expose objects *within* a top-level group.
            // The repeated traversal handles that mixed hierarchy precisely.
            return objects_at_repeated(doc, p, opt, scope);
        }
    }
    // The objects on a locked artboard can't be selected, as `hit_test_skipping` passes over them.
    let locks = doc.locked_artboards();
    if !locks.is_empty() {
        tops.retain(|id| !doc.node(*id).is_some_and(|n| locks.holds(n)));
    }
    tops
}

/// One traversal through a layer and its sublayers. Each selectable object's subtree is
/// inspected once (only until its first hit). `false` asks the caller to use the general
/// algorithm for an envelope whose contents are currently being edited. Symbol instances are
/// measured by their art ([`Node::geometric_bounds_with`]).
fn objects_at_layer(layer: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes, tops: &mut Vec<NodeId>, seen: &mut HashSet<NodeId>) -> bool {
    let Some(children) = layer.children() else { return true };
    if layer.clips() && children.first().is_some_and(|clip| !clip_contains(clip, p)) {
        return true;
    }
    let opt = if matches!(layer.kind, NodeKind::Layer { preview: false, .. }) { HitOptions { outline: true, ..opt } } else { opt };
    for (i, c) in children.iter().enumerate().rev() {
        if !may_hit(layer, i, c, p, opt, symbols, true) || seen.contains(&c.id) {
            continue;
        }
        if c.is_layer() {
            if !objects_at_layer(c, p, opt, symbols, tops, seen) {
                return false;
            }
            continue;
        }
        // Editing an envelope selects its contents rather than the containing object. Even
        // when nested in a group, the old traversal may select both a member and the group.
        if c.is_container() {
            let mut editing = false;
            c.walk(&mut |n| editing |= edits_contents(n));
            if editing {
                return false;
            }
        }
        let skip = |id| seen.contains(&id);
        let mut chain = Vec::new();
        let hit = match &c.kind {
            NodeKind::Group { .. } | NodeKind::Intertwine { .. } => hit_children(c, p, opt, symbols, &skip, &mut chain, true).is_some(),
            NodeKind::Blend { .. } => hit_children(c, p, opt, symbols, &skip, &mut chain, true).is_some() || hit_leaf(c, p, opt, symbols).is_some(),
            _ => hit_leaf(c, p, opt, symbols).is_some(),
        };
        if hit && seen.insert(c.id) {
            tops.push(c.id);
        }
    }
    true
}

/// The general selection order also handles isolation and nested Edit Contents.
fn objects_at_repeated(doc: &Document, p: Point, opt: HitOptions, scope: Option<NodeId>) -> Vec<NodeId> {
    let mut tops: Vec<NodeId> = vec![];
    // Each round leaves out the objects found so far (and everything inside them).
    while let Some(top) = hit_test_skipping(doc, p, opt, &|id| tops.contains(&id), true).map(|h| h.top_object(scope)) {
        if tops.contains(&top) {
            break;
        }
        tops.push(top);
    }
    tops
}

/// The most objects [`locked_objects_at`] lists.
pub const MAX_LOCKED_AT: usize = 20;

/// The locked objects under `p`, topmost first, at most [`MAX_LOCKED_AT`]: each object whose own
/// lock is on and that a click at `p` would hit if it were not locked. A locked group is listed,
/// not what it holds. Hidden objects and everything on hidden, locked and template layers are left
/// out. While a pattern is edited only its tile layer is searched, and in isolation mode
/// (`opt.scope`) only what is inside the isolated group or object.
pub fn locked_objects_at(doc: &Document, p: Point, opt: HitOptions) -> Vec<NodeId> {
    let symbols = doc.symbol_boxes();
    let mut out = Vec::new();
    if let Some(scope) = opt.scope
        && let Some(isolated) = doc.node(scope)
    {
        if isolated.is_container() {
            locked_children(isolated, p, opt, &symbols, &mut out);
        } else if isolated.visible && isolated.locked && locked_hit(isolated, p, opt, &symbols) {
            out.push(isolated.id);
        }
        return out;
    }
    for layer in doc.layers.iter().rev() {
        if !layer.visible || layer.locked || layer.is_template() || doc.pattern_edit.as_ref().is_some_and(|e| e.layer != layer.id) {
            continue;
        }
        let opt = if matches!(layer.kind, NodeKind::Layer { preview: false, .. }) { HitOptions { outline: true, ..opt } } else { opt };
        locked_children(layer, p, opt, &symbols, &mut out);
    }
    out
}

/// [`locked_objects_at`] through the children of `parent`, front first: a locked child is tested
/// whole, and an unlocked layer, group, blend or envelope in Edit Contents is searched inside.
fn locked_children(parent: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes, out: &mut Vec<NodeId>) {
    let Some(children) = parent.children() else { return };
    if parent.clips() && children.first().is_some_and(|clip| !clip_contains(clip, p)) {
        return;
    }
    for i in hit_order(parent, children, p) {
        if out.len() >= MAX_LOCKED_AT {
            return;
        }
        let Some(c) = children.get(i) else { continue };
        if !may_hit(parent, i, c, p, opt, symbols, false) || c.is_layer() && (c.locked || c.is_template()) {
            continue;
        }
        if c.locked {
            if locked_hit(c, p, opt, symbols) {
                out.push(c.id);
            }
            continue;
        }
        match &c.kind {
            NodeKind::Layer { preview: false, .. } => locked_children(c, p, HitOptions { outline: true, ..opt }, symbols, out),
            NodeKind::Layer { .. }
            | NodeKind::Group { .. }
            | NodeKind::Intertwine { .. }
            | NodeKind::Blend { .. }
            | NodeKind::Envelope { editing: true, .. } => locked_children(c, p, opt, symbols, out),
            _ => {}
        }
    }
}

/// Would a click at `p` hit the locked object `n` if it were not locked?
fn locked_hit(n: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes) -> bool {
    let inside = || hit_children(n, p, opt, symbols, &|_| false, &mut Vec::new(), false).is_some();
    match &n.kind {
        NodeKind::Group { .. } | NodeKind::Intertwine { .. } | NodeKind::Envelope { editing: true, .. } => inside(),
        NodeKind::Blend { .. } => inside() || hit_leaf(n, p, opt, symbols).is_some(),
        _ => hit_leaf(n, p, opt, symbols).is_some(),
    }
}

/// [`hit_test`] passing over the nodes `skip` names (and their contents), and with
/// `editable_only` over the objects of layers and sublayers on a locked artboard, and over all of
/// an isolated group on one ([`crate::LockedArtboards::holds`]). Symbol instances are measured by their art
/// ([`Node::geometric_bounds_with`]).
fn hit_test_skipping(doc: &Document, p: Point, opt: HitOptions, skip: &dyn Fn(NodeId) -> bool, editable_only: bool) -> Option<Hit> {
    let finish = |mut h: Hit| {
        h.contents_of = h.ancestry.iter().rev().copied().find(|a| doc.node(*a).is_some_and(edits_contents));
        h.layers = h.ancestry.iter().take_while(|a| doc.node(**a).is_some_and(Node::is_layer)).count();
        h
    };
    let locks = doc.locked_artboards();
    let on_locked = |id: NodeId| {
        editable_only
            && !locks.is_empty()
            && doc.parent_of(id).and_then(|l| doc.node(l)).is_some_and(Node::is_layer)
            && doc.node(id).is_some_and(|n| locks.holds(n))
    };
    let skip = &|id| skip(id) || on_locked(id);
    let symbols = doc.symbol_boxes();
    // Isolation mode: only what is inside the isolated container hits. The art around it is
    // locked, and clicks pass through it (#833).
    if editable_only
        && let Some(scope) = opt.scope
        && let Some(isolated) = doc.node(scope)
        && let Some(mut chain) = doc.ancestry(scope)
    {
        // Isolated before its artboard was locked: nothing in it hits.
        if locks.holds_id(scope) {
            return None;
        }
        if isolated.is_container() {
            return hit_children(isolated, p, opt, &symbols, skip, &mut chain, editable_only).map(finish);
        }
        // A single object isolated (a path, an image…): only it hits, among its siblings.
        chain.pop();
        let parent = chain.last().and_then(|id| doc.node(*id))?;
        return hit_children(parent, p, opt, &symbols, &|id| id != scope || skip(id), &mut chain, editable_only).map(finish);
    }
    let mut chain = Vec::new();
    for layer in doc.layers.iter().rev() {
        if !layer.visible || editable_only && (layer.locked || layer.is_template()) {
            continue;
        }
        // Pattern editing mode: only the tile art is editable.
        if editable_only && doc.pattern_edit.as_ref().is_some_and(|e| e.layer != layer.id) {
            continue;
        }
        chain.push(layer.id);
        let opt = if matches!(layer.kind, NodeKind::Layer { preview: false, .. }) { HitOptions { outline: true, ..opt } } else { opt };
        if let Some(h) = hit_children(layer, p, opt, &symbols, skip, &mut chain, editable_only) {
            return Some(finish(h));
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
    clip_regions(clip).iter().any(|(bp, rule)| fill_contains(bp, *rule, p))
}

/// The regions `clip` clips to ([`Node::clip_shapes`]), text counted by its frame as in
/// [`clip_contains`].
fn clip_regions(clip: &Node) -> Vec<(vectorcraft_geom::BezPath, vectorcraft_geom::FillRule)> {
    let frame = |n: &Node| {
        let b = n.geometric_bounds()?;
        Some(Node::path(n.id, vectorcraft_geom::shapes::rectangle(b), Default::default()))
    };
    clip.clip_shapes(Some(&frame))
}

/// Can a click at `p` hit `c`, the child `i` of `parent`, or something inside it? Not when it is
/// hidden, when it can't be edited (`editable_only`), when it is the part of a Shaper group that
/// isn't shown, or when `p` is outside its reach.
fn may_hit(parent: &Node, i: usize, c: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes, editable_only: bool) -> bool {
    let source_isolated = |n: &Node| n.children().and_then(|children| children.first()).is_some_and(|source| Some(source.id) == opt.scope);
    if parent.extras().shaper.is_some() && (i == 0) != source_isolated(parent) {
        return false;
    }
    // Read-only samples can read locked/template art; hidden art never participates.
    if !c.visible || editable_only && (c.locked || c.is_template()) {
        return false;
    }
    // (An envelope's content sits where it was, not where the envelope draws it.)
    edits_contents(c)
        || c.extras().shaper.is_some() && source_isolated(c)
        || c.reach_bounds_with(symbols).is_none_or(|b| b.inflate(opt.tol, opt.tol).contains(p))
}

fn hit_children(
    parent: &Node,
    p: Point,
    opt: HitOptions,
    symbols: &SymbolBoxes,
    skip: &dyn Fn(NodeId) -> bool,
    chain: &mut Vec<NodeId>,
    editable_only: bool,
) -> Option<Hit> {
    let children = parent.children()?;
    // A clip group (or a layer with a clipping mask) only hits inside its clipping path.
    if parent.clips()
        && let Some(clip) = children.first()
        && !clip_contains(clip, p)
    {
        return None;
    }
    for i in hit_order(parent, children, p) {
        let Some(c) = children.get(i) else { continue };
        if !may_hit(parent, i, c, p, opt, symbols, editable_only) || skip(c.id) {
            continue;
        }
        chain.push(c.id);
        let hit = match &c.kind {
            // A sublayer whose Preview is off is clicked in outline, like an outline view.
            NodeKind::Layer { preview: false, .. } => hit_children(c, p, HitOptions { outline: true, ..opt }, symbols, skip, chain, editable_only),
            NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Intertwine { .. } => {
                hit_children(c, p, opt, symbols, skip, chain, editable_only)
            }
            // Edit Contents: the envelope's content hits, undistorted.
            NodeKind::Envelope { editing: true, .. } => hit_children(c, p, opt, symbols, skip, chain, editable_only),
            // A blend's key objects first (Direct and Group Selection pick them); its steps hit
            // as the blend.
            NodeKind::Blend { .. } => hit_children(c, p, opt, symbols, skip, chain, editable_only)
                .or_else(|| hit_leaf(c, p, opt, symbols).map(|kind| Hit { leaf: c.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 })),
            _ => hit_leaf(c, p, opt, symbols).map(|kind| Hit { leaf: c.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 }),
        };
        if hit.is_some() {
            return hit;
        }
        chain.pop();
    }
    None
}

/// The order a click at `p` tries the children of `parent` in, front first: their stacking order
/// from the top, except that in an Intertwine group the members drawn over the others at `p`
/// come first (from the top among them too).
fn hit_order<'a>(parent: &Node, children: &'a [std::sync::Arc<Node>], p: Point) -> impl Iterator<Item = usize> + 'a {
    let over: Vec<usize> = if matches!(parent.kind, NodeKind::Intertwine { .. }) {
        let contains = |a: &vectorcraft_geom::PathData| fill_contains(&a.to_bezpath(), vectorcraft_geom::FillRule::NonZero, p);
        (0..children.len()).rev().filter(|i| children.get(*i).and_then(|c| crate::intertwine::over_area(c)).is_some_and(contains)).collect()
    } else {
        vec![]
    };
    let rest = (0..children.len()).rev().filter({
        let over = over.clone();
        move |i| !over.contains(i)
    });
    over.into_iter().chain(rest)
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

fn hit_leaf(n: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes) -> Option<HitKind> {
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
        | NodeKind::Envelope { .. } => n.geometric_bounds_with(symbols).filter(|b| b.inflate(opt.tol, opt.tol).contains(p)).map(|_| HitKind::Bounds),
        NodeKind::Repeat(r) => r.expand().iter().find_map(|g| hit_any(g, p, opt, symbols)),
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
fn hit_any(n: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes) -> Option<HitKind> {
    match &n.kind {
        NodeKind::Group { children, .. } => children.iter().rev().find_map(|c| hit_any(c, p, opt, symbols)),
        _ => hit_leaf(n, p, opt, symbols),
    }
}

/// The path of compound path `n` under `p`, for the Group Selection tool: the one whose outline is
/// within `tol`, else the innermost one whose area holds `p` (a hole inside an outer shape, an
/// island inside a hole). `None` when `n` isn't a compound path or no part is under `p`.
pub fn compound_part_at(n: &Node, p: Point, tol: f64) -> Option<NodeId> {
    let NodeKind::Compound { children, rule } = &n.kind else { return None };
    let parts: Vec<_> = children.iter().rev().filter_map(|c| c.path_data().map(|pd| (c.id, pd.to_bezpath(), pd.bounds()))).collect();
    if let Some((id, ..)) = parts.iter().find(|(_, bp, _)| stroke_contains(bp, 0.0, tol, p)) {
        return Some(*id);
    }
    // The innermost by the size of its box.
    let size = |b: &Option<Rect>| b.map_or(f64::INFINITY, |b| b.area());
    parts.iter().filter(|(_, bp, _)| fill_contains(bp, *rule, p)).min_by(|a, b| size(&a.2).total_cmp(&size(&b.2))).map(|(id, ..)| *id)
}

/// Objects (children of layers, or of `scope` in isolation mode) touched by a marquee rect, leaving
/// out those on a locked artboard ([`crate::LockedArtboards::holds_id`]). Symbol instances are measured
/// by their art ([`Node::geometric_bounds_with`]). The art of a clip group or of a layer with a
/// clipping mask counts only when the marquee meets the region its clipping path clips to, and then
/// only within that region's bounds.
pub fn marquee(doc: &Document, r: Rect, scope: Option<NodeId>, leaves: bool) -> Vec<NodeId> {
    let mut out = Vec::new();
    // `tops` pairs each object with the part of `r` in which the marquee can touch it.
    let tops: Vec<(&std::sync::Arc<Node>, Rect)> = match scope.and_then(|s| doc.node(s)) {
        Some(s) => match (s.children(), marquee_in(s, r)) {
            (Some(c), Some(r)) => c.iter().map(|c| (c, r)).collect(),
            (Some(_), None) => vec![],
            // A single object isolated: only it.
            (None, _) => doc
                .parent_of(s.id)
                .and_then(|p| doc.node(p))
                .and_then(|p| Some((p.children()?, marquee_in(p, r)?)))
                .into_iter()
                .flat_map(|(c, r)| c.iter().filter(|c| c.id == s.id).map(move |c| (c, r)))
                .collect(),
        },
        // The objects of the layers, looking through sublayers (hidden, locked and template ones
        // are left out).
        None => doc
            .layers
            .iter()
            .filter(|l| l.visible && !l.locked && !l.is_template() && doc.pattern_edit.as_ref().is_none_or(|e| e.layer == l.id))
            .flat_map(|l| layer_objects(l, r))
            .collect(),
    };
    /// The objects of layer `l`, each with the part of `r` that the clipping masks of `l` and of
    /// the sublayers it is in leave.
    fn layer_objects(l: &Node, r: Rect) -> Vec<(&std::sync::Arc<Node>, Rect)> {
        let mut out = vec![];
        let Some(r) = marquee_in(l, r) else { return out };
        for c in l.children().into_iter().flatten() {
            if c.is_layer() {
                if c.visible && !c.locked && !c.is_template() {
                    out.extend(layer_objects(c, r));
                }
            } else {
                out.push((c, r));
            }
        }
        out
    }
    fn touches(n: &Node, s: &SymbolBoxes, r: Rect) -> bool {
        match &n.kind {
            NodeKind::Path { path, .. } => vectorcraft_geom::hit::intersects_rect(path, r),
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
                marquee_in(n, r).is_some_and(|r| children.iter().any(|c| c.visible && touches(c, s, r)))
            }
            NodeKind::Compound { children, .. } => children.iter().any(|c| c.visible && touches(c, s, r)),
            // A box with no width or no height (an instance of a symbol that draws a straight
            // line) is touched where the marquee crosses it.
            _ => n.geometric_bounds_with(s).is_some_and(|b| {
                let crosses = (b.width() == 0.0 || b.height() == 0.0) && b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1;
                b.intersect(r).area() > 0.0 || r.contains(b.origin()) || crosses
            }),
        }
    }
    fn collect_leaves(n: &Node, s: &SymbolBoxes, r: Rect, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
                let Some(r) = marquee_in(n, r) else { return };
                for c in children {
                    if c.visible && !c.locked {
                        collect_leaves(c, s, r, out);
                    }
                }
            }
            _ => {
                if touches(n, s, r) {
                    out.push(n.id);
                }
            }
        }
    }
    let symbols = doc.symbol_boxes();
    let locks = doc.locked_artboards();
    for (t, r) in tops {
        if !t.visible || t.locked || locks.holds_id(t.id) {
            continue;
        }
        if leaves {
            collect_leaves(t, &symbols, r, &mut out);
        } else if touches(t, &symbols, r) {
            out.push(t.id);
        }
    }
    out
}

/// The part of a marquee `r` in which it can touch the children of `parent`. For a clip group or a
/// layer with a clipping mask, this is what [`marquee_in_clip`] returns for its clipping path (None
/// when the marquee misses it); otherwise it is all of `r`.
fn marquee_in(parent: &Node, r: Rect) -> Option<Rect> {
    match parent.children().and_then(|c| c.first()) {
        Some(clip) if parent.clips() => marquee_in_clip(clip, r),
        _ => Some(r),
    }
}

/// The part of a marquee `r` in which it can touch the art that `clip` clips (the clipping path of
/// a clip group or of a layer with a clipping mask): `r` cut to the bounds of the regions `clip`
/// clips to ([`clip_regions`]), or `None` when `r` meets none of those regions. Within those
/// bounds, art outside a region that is not a rectangle still counts.
fn marquee_in_clip(clip: &Node, r: Rect) -> Option<Rect> {
    use vectorcraft_geom::kurbo::Shape;
    let regions = clip_regions(clip);
    // The edge of a region is its outline with each open subpath closed, as `fill_contains` fills it.
    let meets = |(bp, rule): &(vectorcraft_geom::BezPath, vectorcraft_geom::FillRule)| {
        let mut edge = vectorcraft_geom::PathData::from_bezpath(bp);
        edge.subpaths.iter_mut().for_each(|sp| sp.closed = true);
        fill_contains(bp, *rule, r.center()) || vectorcraft_geom::hit::intersects_rect(&edge, r)
    };
    if !regions.iter().any(meets) {
        return None;
    }
    let bounds = regions.iter().map(|(bp, _)| bp.bounding_box()).reduce(|a, b| a.union(b))?;
    Some(r.intersect(bounds))
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

    /// #1103: a marquee drawn over a background picks what is on it, not the background, whose
    /// outline it doesn't touch.
    #[test]
    fn a_marquee_inside_a_background_leaves_the_background() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let (back, small) = (d.alloc_id(), d.alloc_id());
        d.insert(Some(l), 0, Node::path(back, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Appearance::default_art())).unwrap();
        d.insert(Some(l), 1, Node::path(small, shapes::rectangle(Rect::new(50.0, 50.0, 60.0, 60.0)), Appearance::default_art())).unwrap();
        assert_eq!(marquee(&d, Rect::new(40.0, 40.0, 70.0, 70.0), None, false), vec![small]);
        // Reaching the background's edge takes it too.
        assert_eq!(marquee(&d, Rect::new(-5.0, 40.0, 70.0, 70.0), None, false), vec![back, small]);
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

    /// The art of a 200 pt wide rectangle clipped to its left half (0..100 × 0..100): the mask and
    /// the art, in that order.
    fn half_clipped(d: &mut Document) -> (Node, Node) {
        let mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
        let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 100.0)), Appearance::default_art());
        (mask, art)
    }

    /// #2345: a marquee touches a clip group only where its art shows. Over the clipped-away
    /// half alone it selects nothing, and around a rectangle drawn beside the group's mask it
    /// takes that rectangle alone; reaching into the mask takes the group. The same holds for
    /// Group Selection's leaves and in isolation mode.
    #[test]
    fn a_marquee_leaves_a_clip_group_by_its_clipped_away_art() {
        let mut d = Document::new(300.0, 200.0);
        let l = d.layers[0].id;
        let (mask, art) = half_clipped(&mut d);
        let (g, art_id, beside) = (d.alloc_id(), art.id, d.alloc_id());
        let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
        clip.set_clips(true);
        d.insert(Some(l), 0, clip).unwrap();
        d.insert(Some(l), 1, Node::path(beside, shapes::rectangle(Rect::new(150.0, 80.0, 190.0, 130.0)), Appearance::default_art())).unwrap();
        // This marquee crosses only the hidden art's top and right edges.
        let hidden = Rect::new(150.0, -10.0, 210.0, 50.0);
        assert!(marquee(&d, hidden, None, false).is_empty());
        assert!(marquee(&d, hidden, None, true).is_empty());
        assert!(marquee(&d, hidden, Some(g), false).is_empty());
        assert!(marquee(&d, hidden, Some(art_id), false).is_empty());
        // This marquee surrounds the rectangle beside and crosses the hidden art's bottom edge.
        assert_eq!(marquee(&d, Rect::new(140.0, 70.0, 195.0, 140.0), None, false), vec![beside]);
        // This marquee crosses the shown half's top edge and goes on into the hidden half.
        let shown = Rect::new(50.0, -10.0, 160.0, 10.0);
        assert_eq!(marquee(&d, shown, None, false), vec![g]);
        assert!(marquee(&d, shown, None, true).contains(&art_id));
        assert_eq!(marquee(&d, shown, Some(g), false).len(), 2, "the mask and the art");
    }

    /// #2345: in the corner of a diamond mask's box, outside the diamond, a marquee over the
    /// clipped art's corner leaves the clip group. An open mask clips to its area closed by a
    /// straight line, and a marquee across that line takes the art it reaches inside.
    #[test]
    fn a_marquee_must_meet_the_clipping_path_itself() {
        // A clip group of a mask through `pts` (closed or open) over the art `art`.
        let clipped = |pts: &[(f64, f64)], closed: bool, art: Rect| {
            let mut d = Document::new(200.0, 200.0);
            let l = d.layers[0].id;
            let mut mask = vectorcraft_geom::BezPath::new();
            mask.move_to(pts[0]);
            for &p in &pts[1..] {
                mask.line_to(p);
            }
            if closed {
                mask.close_path();
            }
            let mask = Node::path(d.alloc_id(), vectorcraft_geom::PathData::from_bezpath(&mask), Appearance::default());
            let art = Node::path(d.alloc_id(), shapes::rectangle(art), Appearance::default_art());
            let g = d.alloc_id();
            let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
            clip.set_clips(true);
            d.insert(Some(l), 0, clip).unwrap();
            (d, g)
        };
        let (d, g) = clipped(&[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)], true, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(marquee(&d, Rect::new(-10.0, -10.0, 10.0, 10.0), None, false).is_empty());
        assert_eq!(marquee(&d, Rect::new(-10.0, 40.0, 10.0, 60.0), None, false), vec![g], "over the diamond's left point");
        // The mask is the triangle above the line from (0, 0) to (100, 100), and the art lies inside it.
        let (d, g) = clipped(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false, Rect::new(55.0, 40.0, 70.0, 50.0));
        assert_eq!(marquee(&d, Rect::new(40.0, 45.0, 60.0, 70.0), None, false), vec![g]);
    }

    /// #2345: the objects of a layer or sublayer with a clipping mask are touched only where the
    /// mask shows them, as in a clip group.
    #[test]
    fn a_marquee_leaves_art_a_layer_clips_away() {
        for sublayer in [false, true] {
            let mut d = Document::new(300.0, 200.0);
            let l = d.layers[0].id;
            let beside = d.alloc_id();
            d.insert(Some(l), 0, Node::path(beside, shapes::rectangle(Rect::new(150.0, 80.0, 190.0, 130.0)), Appearance::default_art())).unwrap();
            let masked = d.alloc_id();
            let mut layer = Node::layer(masked, "Masked", crate::LayerColor::Preset(2));
            layer.set_clips(true);
            d.insert(sublayer.then_some(l), 9, layer).unwrap();
            let (mask, art) = half_clipped(&mut d);
            let (mask_id, art_id) = (mask.id, art.id);
            d.insert(Some(masked), 0, mask).unwrap();
            d.insert(Some(masked), 1, art).unwrap();
            assert!(marquee(&d, Rect::new(150.0, -10.0, 210.0, 50.0), None, false).is_empty(), "sublayer: {sublayer}");
            assert_eq!(marquee(&d, Rect::new(140.0, 70.0, 195.0, 140.0), None, false), vec![beside], "sublayer: {sublayer}");
            assert_eq!(marquee(&d, Rect::new(50.0, -10.0, 160.0, 10.0), None, false), vec![mask_id, art_id], "sublayer: {sublayer}");
        }
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
        compound.appearance = Appearance::default_art().into();
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
            n.extras_mut().perspective = perspective.map(|a| {
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
        assert_eq!(at(30.0, Some(g)), [c_id, b_id], "isolated: the group's own objects");
        // Isolated, the art around the group is locked: clicks pass through it (#833).
        assert!(at(5.0, Some(g)).is_empty());
        assert!(at(130.0, Some(g)).is_empty());
        assert!(hit_test(&d, Point::new(5.0, 25.0), HitOptions { scope: Some(g), ..HitOptions::default() }).is_none());
        assert_eq!(at(5.0, None), vec![a_id]);
        assert!(at(90.0, None).is_empty());
    }

    /// Many overlapping paths: Select Behind must list each exactly once, front to back.
    /// This is quadratic with repeated full-document hit tests, but linear in tree size with
    /// the per-object walk.
    #[test]
    fn objects_at_large_overlapping_stack() {
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        let mut ids = Vec::new();
        for _ in 0..256 {
            let id = d.alloc_id();
            d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
            ids.push(id);
        }
        ids.reverse();
        assert_eq!(objects_at(&d, Point::new(25.0, 25.0), HitOptions::default(), None), ids);
    }

    /// Match the original skip-and-retest algorithm for normal groups, sublayers, clipping,
    /// invisibility, and path/outline-only settings.
    #[test]
    fn objects_at_one_pass_matches_repeated_hits() {
        let mut d = Document::new(200.0, 200.0);
        let layer = d.layers[0].id;
        let add = |d: &mut Document, parent, bounds| {
            let id = d.alloc_id();
            d.insert(Some(parent), usize::MAX, Node::path(id, shapes::rectangle(bounds), Appearance::default_art())).unwrap();
            id
        };
        add(&mut d, layer, Rect::new(0.0, 0.0, 60.0, 60.0));
        let sub = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sublayer", crate::LayerColor::Preset(2))).unwrap();
        add(&mut d, sub, Rect::new(0.0, 0.0, 60.0, 60.0));
        let group = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::group(group, Vec::new())).unwrap();
        add(&mut d, group, Rect::new(0.0, 0.0, 60.0, 60.0));
        add(&mut d, group, Rect::new(5.0, 5.0, 60.0, 60.0));
        let clip = d.alloc_id();
        let mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(5.0, 5.0, 45.0, 45.0)), Appearance::default_art());
        let content = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 80.0, 80.0)), Appearance::default_art());
        let mut clipping = Node::group(clip, vec![Arc::new(mask), Arc::new(content)]);
        clipping.set_clips(true);
        d.insert(Some(layer), usize::MAX, clipping).unwrap();
        for (point, opt) in [
            (Point::new(25.0, 25.0), HitOptions::default()),
            (Point::new(3.0, 25.0), HitOptions::default()),
            (Point::new(25.0, 25.0), HitOptions { outline: true, ..Default::default() }),
            (Point::new(6.0, 25.0), HitOptions { path_only: true, ..Default::default() }),
            (Point::new(100.0, 100.0), HitOptions::default()),
        ] {
            assert_eq!(objects_at(&d, point, opt, None), objects_at_repeated(&d, point, opt, None), "point={point:?}, opt={opt:?}");
        }
        if let NodeKind::Layer { preview, .. } = &mut d.node_mut(sub).unwrap().kind {
            *preview = false;
        }
        let p = Point::new(25.0, 25.0);
        let opt = HitOptions::default();
        assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None));
        d.node_mut(group).unwrap().locked = true;
        assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None));
    }

    /// Documents built at random (nested groups, sublayers, clipping groups, compound paths, blends
    /// of nothing, hidden and locked objects, outline layers): Select Behind lists the same objects
    /// in the same order as the hit test repeated over what is left.
    #[test]
    fn objects_at_matches_repeated_hits_on_random_documents() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = move |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n
        };
        fn fill(d: &mut Document, parent: NodeId, depth: u32, next: &mut impl FnMut(u32) -> u32) {
            for _ in 0..2 + next(4) {
                let id = d.alloc_id();
                let (x, y) = (f64::from(next(60)), f64::from(next(60)));
                let rect = Rect::new(x, y, x + 10.0 + f64::from(next(50)), y + 10.0 + f64::from(next(50)));
                let appearance = if next(4) == 0 { Appearance::default() } else { Appearance::default_art() };
                let mut n = match next(if depth < 3 { 8 } else { 4 }) {
                    4 | 5 => Node::group(id, Vec::new()),
                    6 => Node::layer(id, "Sublayer", crate::LayerColor::Preset(2)),
                    7 => {
                        let mut g = Node::group(id, Vec::new());
                        g.set_clips(true);
                        g
                    }
                    _ => Node::path(id, shapes::rectangle(rect), appearance),
                };
                n.visible = next(8) != 0;
                n.locked = next(8) == 0;
                if let NodeKind::Layer { preview, .. } = &mut n.kind {
                    *preview = next(3) != 0;
                }
                let container = n.is_container();
                // A sublayer belongs in a layer; in a group it is a group like another to the hit test.
                d.insert(Some(parent), usize::MAX, n).unwrap();
                if container {
                    fill(d, id, depth + 1, next);
                }
            }
        }
        for round in 0..200 {
            let mut d = Document::new(120.0, 120.0);
            let layer = d.layers[0].id;
            fill(&mut d, layer, 0, &mut next);
            for _ in 0..12 {
                let p = Point::new(f64::from(next(120)), f64::from(next(120)));
                let opt = HitOptions { outline: next(5) == 0, path_only: next(5) == 0, ..Default::default() };
                assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None), "round {round}, {p:?}, {opt:?}");
            }
        }
    }

    /// Select Behind finds a symbol instance by its symbol's art, as a click does: art from
    /// (100, 60) to (140, 80), outside the 20 × 20 square around the origin, drawn where it lies by
    /// one instance and 100 pt lower by an instance in a group.
    #[test]
    fn objects_at_finds_symbol_instances_by_their_art() {
        let mut d = Document::new(200.0, 200.0);
        let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 60.0, 140.0, 80.0)), Appearance::default_art());
        d.symbols.push(crate::Symbol { name: "Far".into(), art: Arc::new(art) });
        let (layer, top, inner, group) = (d.layers[0].id, d.alloc_id(), d.alloc_id(), d.alloc_id());
        let instance =
            |id, dy: f64| Node::new(id, NodeKind::SymbolInstance { symbol: "Far".into(), xf: vectorcraft_geom::Affine::translate((0.0, dy)) });
        d.insert(Some(layer), 0, instance(top, 0.0)).unwrap();
        d.insert(Some(layer), 1, Node::group(group, vec![Arc::new(instance(inner, 100.0))])).unwrap();
        let opt = HitOptions::default();
        for (x, y, want) in [(120.0, 70.0, vec![top]), (120.0, 170.0, vec![group]), (0.0, 0.0, vec![]), (0.0, 100.0, vec![])] {
            let p = Point::new(x, y);
            assert_eq!(objects_at(&d, p, opt, None), want, "{p:?}");
            assert_eq!(objects_at_repeated(&d, p, opt, None), want, "{p:?}");
        }
    }

    /// Edit Contents can expose descendants of a top-level group: preserve the original
    /// selection order by falling back to the general hit traversal.
    #[test]
    fn objects_at_edit_contents_uses_general_order() {
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        let inner = |id| Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art()));
        let (a, b) = (d.alloc_id(), d.alloc_id());
        let e = Node::new(
            d.alloc_id(),
            NodeKind::Envelope {
                content: vec![inner(a), inner(b)],
                kind: Box::new(crate::live::EnvelopeKind::Warp { style: "arc".to_string(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true }),
                fidelity: 100.0,
                editing: true,
                options: Default::default(),
                frame: Box::new(vectorcraft_geom::Affine::IDENTITY),
            },
        );
        d.insert(Some(layer), usize::MAX, e).unwrap();
        let p = Point::new(25.0, 25.0);
        let opt = HitOptions::default();
        assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None));
        assert_eq!(objects_at(&d, p, opt, None), vec![b, a]);
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

    /// Insert `n` locked at the top of `parent`.
    fn insert_locked(d: &mut Document, parent: NodeId, mut n: Node) -> NodeId {
        let id = n.id;
        n.locked = true;
        d.insert(Some(parent), usize::MAX, n).unwrap();
        id
    }

    /// `locked_objects_at`: the locked objects under the point, topmost first. A locked group is
    /// listed rather than its contents, and a locked object in an unlocked group is listed itself.
    /// Hidden objects, locked layers and the art outside an isolated group are left out.
    #[test]
    fn locked_objects_at_lists_the_locked_stack() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let rect =
            |d: &mut Document, x: f64| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 50.0, 50.0)), Appearance::default_art());
        let (a, b) = (rect(&mut d, 0.0), rect(&mut d, 10.0));
        let (a, b) = (insert_locked(&mut d, l, a), insert_locked(&mut d, l, b));
        let (mut inner, locked_group) = (rect(&mut d, 20.0), d.alloc_id());
        inner.locked = true;
        let inner_id = inner.id;
        let mut g = Node::group(locked_group, vec![Arc::new(inner)]);
        g.locked = true;
        d.insert(Some(l), usize::MAX, g).unwrap();
        let open_group = d.alloc_id();
        let mut child = rect(&mut d, 25.0);
        child.locked = true;
        let child_id = child.id;
        let free = rect(&mut d, 25.0);
        d.insert(Some(l), usize::MAX, Node::group(open_group, vec![Arc::new(child), Arc::new(free)])).unwrap();
        let at = |d: &Document, x: f64, scope| locked_objects_at(d, Point::new(x, 25.0), HitOptions { scope, ..HitOptions::default() });
        assert_eq!(at(&d, 30.0, None), [child_id, locked_group, b, a]);
        assert!(!at(&d, 30.0, None).contains(&inner_id), "the locked group stands for what it holds");
        assert_eq!(at(&d, 5.0, None), [a]);
        assert!(at(&d, 150.0, None).is_empty());
        // Isolated, only the group's own objects are looked at.
        assert_eq!(at(&d, 30.0, Some(open_group)), [child_id]);
        // A hidden object is left out, and nothing on a locked or hidden layer is listed.
        d.node_mut(b).unwrap().visible = false;
        assert_eq!(at(&d, 30.0, None), [child_id, locked_group, a]);
        d.node_mut(l).unwrap().locked = true;
        assert!(at(&d, 30.0, None).is_empty());
        d.node_mut(l).unwrap().locked = false;
        d.node_mut(l).unwrap().visible = false;
        assert!(at(&d, 30.0, None).is_empty());
    }

    /// `locked_objects_at` lists at most `MAX_LOCKED_AT` objects, the topmost ones.
    #[test]
    fn locked_objects_at_stops_at_its_limit() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.layers[0].id;
        let ids: Vec<NodeId> = (0..MAX_LOCKED_AT + 5)
            .map(|_| {
                let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art());
                insert_locked(&mut d, l, n)
            })
            .collect();
        let found = locked_objects_at(&d, Point::new(25.0, 25.0), HitOptions::default());
        assert_eq!(found, ids.iter().rev().take(MAX_LOCKED_AT).copied().collect::<Vec<_>>());
    }

    #[test]
    fn visible_sampling_reads_locked_art_and_layers_but_excludes_hidden_art() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
        let p = Point::new(10.0, 10.0);
        for lock in [a, l] {
            d.node_mut(lock).unwrap().locked = true;
            assert!(hit_test(&d, p, HitOptions::default()).is_none());
            assert_eq!(hit_test_visible(&d, p, HitOptions::default()).unwrap().leaf, a);
            assert!(d.node(lock).unwrap().locked);
            d.node_mut(lock).unwrap().locked = false;
        }
        d.node_mut(a).unwrap().visible = false;
        assert!(hit_test_visible(&d, p, HitOptions::default()).is_none());
        d.node_mut(a).unwrap().visible = true;
        d.node_mut(l).unwrap().visible = false;
        assert!(hit_test_visible(&d, p, HitOptions::default()).is_none());
    }
}
