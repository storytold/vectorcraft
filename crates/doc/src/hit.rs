//! Hit testing over the document tree.

use std::collections::HashSet;
use std::sync::OnceLock;

use vectorcraft_geom::hit::{fill_contains, stroke_contains};
use vectorcraft_geom::{BezPath, FillRule, Point, Rect};

use crate::{AppearanceItem, Document, Node, NodeId, NodeKind, SymbolBoxes};

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
    /// Guide objects (paths made into guides) are not hittable: View › Guides › Lock Guides is
    /// on or the guides are hidden.
    pub skip_guides: bool,
    /// The selection tools: a clip group's clipping path is hit on its outline, within the
    /// tolerance, before the art it clips, so that the mask can be reached where the art covers
    /// its edge. Off for the tools that act on what is drawn (sampling, type, gradients…);
    /// [`hit_test_visible`] never does it.
    pub clip_outlines: bool,
}

impl Default for HitOptions {
    fn default() -> Self {
        Self { tol: 3.0, outline: false, path_only: false, type_path_only: false, skip_guides: false, clip_outlines: false, scope: None }
    }
}

/// The shapes that an image's own fills and strokes paint, each with its fill rule and the fill or
/// stroke that paints it (none for fills and strokes that paint nothing). Effects are evaluated
/// above this crate, so the engine registers one with [`set_image_paint`].
pub type ImagePaint = fn(&Node) -> Vec<(BezPath, FillRule, AppearanceItem)>;

static IMAGE_PAINT: OnceLock<ImagePaint> = OnceLock::new();

/// Register how the shapes of an image's own fills and strokes are found (the first registration
/// stays).
pub fn set_image_paint(f: ImagePaint) {
    // Already set: the same engine, registered by another session.
    let _ = IMAGE_PAINT.set(f);
}

/// The shapes that the image `n`'s own fills and strokes paint ([`set_image_paint`]), none while
/// nothing is registered.
pub(crate) fn image_paint(n: &Node) -> Vec<(BezPath, FillRule, AppearanceItem)> {
    IMAGE_PAINT.get().map_or_else(Vec::new, |f| f(n))
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

/// The object of `selected` (topmost first) whose own art lies under `p` where a clipping mask
/// hides it: a clip group (or a layer with a clipping mask) holding it doesn't clip to `p`. A
/// click there hits nothing ([`hit_test`]), so the selection tools grab the selection by that
/// art instead of starting a marquee: an image in a mask, picked in the Layers panel, can be
/// dragged by its hidden part, whose frame shows while it is selected. An object that a click
/// can't hit anyway doesn't count: hidden, locked, a template, a guide while guides are locked
/// or hidden, in a Shaper group's part a click can't reach, outside the isolated container
/// (`opt.scope`) or on a locked artboard. On a layer whose Preview is off it counts by its
/// outline, as a click there hits it.
pub fn selected_hidden_at(doc: &Document, p: Point, opt: HitOptions, selected: &[NodeId]) -> Option<NodeId> {
    let symbols = doc.symbol_boxes();
    let locks = doc.locked_artboards();
    selected.iter().rev().copied().find(|&id| {
        let Some(chain) = doc.ancestry(id) else { return false };
        let nodes: Vec<&Node> = chain.iter().filter_map(|a| doc.node(*a)).collect();
        if nodes.len() != chain.len()
            || nodes.iter().any(|n| !n.visible || n.locked || n.is_template())
            || opt.scope.is_some_and(|s| !chain.contains(&s))
            || locks.holds_id(id)
        {
            return false;
        }
        // How a click reaches it, if one does.
        let Some(opt) = path_options(&nodes, &chain, opt) else { return false };
        // Hidden by a mask: one of the containers around it clips, and not to `p`.
        let masked = nodes
            .iter()
            .rev()
            .skip(1)
            .any(|n| n.clips() && n.children().and_then(|c| c.first()).is_some_and(|clip| clip.id != id && !clip_contains(clip, p)));
        // A click may reach it there ([`may_hit`]: not a guide while guides are locked or
        // hidden, nor a Shaper group's hidden part), and its own art is what a click would hit
        // in it: a group's hidden, locked or masked contents don't count.
        let [.., parent, n] = nodes.as_slice() else { return false };
        let reachable =
            parent.children().and_then(|c| c.iter().position(|c| c.id == id)).is_some_and(|i| may_hit(parent, i, n, p, opt, &symbols, true));
        masked && reachable && object_hit(n, p, opt, &symbols, true)
    })
}

/// The options a click goes down `nodes` (an object's ancestry, `chain`, the layer first) with, as
/// [`hit_test`] goes: from the isolated container (`opt.scope`), else from the layer, in outline
/// below a layer whose Preview is off. `None` when a Shaper group's part it can't reach lies on
/// the way ([`shaper_hides`]).
fn path_options(nodes: &[&Node], chain: &[NodeId], opt: HitOptions) -> Option<HitOptions> {
    let scope_at = opt.scope.and_then(|s| chain.iter().position(|a| *a == s));
    let child_index = |parent: &Node, child: &Node| parent.children().and_then(|c| c.iter().position(|c| c.id == child.id));
    let hidden = nodes.windows(2).skip(scope_at.unwrap_or(0)).any(|w| match w {
        [parent, child] => child_index(parent, child).is_some_and(|i| shaper_hides(parent, i, opt.scope)),
        _ => false,
    });
    if hidden {
        return None;
    }
    // The isolated container's own Preview doesn't count: a click starts inside it.
    let below = scope_at.map_or(0, |i| i + 1);
    let outline = nodes.iter().skip(below).any(|n| matches!(n.kind, NodeKind::Layer { preview: false, .. }));
    Some(if outline { HitOptions { outline: true, ..opt } } else { opt })
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
    if layer.clips()
        && let Some(clip) = children.first()
    {
        // As the hit test does: the clipping mask's outline first, then only inside the mask.
        if clip_outline_hit(layer, clip, p, opt, symbols, true).is_some() && seen.insert(clip.id) {
            tops.push(clip.id);
        }
        if !clip_contains(clip, p) {
            return true;
        }
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
    object_hit(n, p, opt, symbols, false)
}

/// Would a click at `p` hit the object `n`, wherever it is? A container is hit by its contents as
/// [`hit_test`] tries them (its own clipping mask, and with `editable_only` none of its hidden,
/// locked or template contents).
fn object_hit(n: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes, editable_only: bool) -> bool {
    let inside = || hit_children(n, p, opt, symbols, &|_| false, &mut Vec::new(), editable_only).is_some();
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
    if shaper_hides(parent, i, opt.scope) {
        return false;
    }
    // Read-only samples can read locked/template art; hidden art never participates.
    if !c.visible || editable_only && (c.locked || c.is_template()) {
        return false;
    }
    if opt.skip_guides && matches!(c.kind, NodeKind::Path { guide: true, .. }) {
        return false;
    }
    // (An envelope's content sits where it was, not where the envelope draws it.)
    edits_contents(c)
        || c.extras().shaper.is_some() && source_of(c, opt.scope)
        || c.reach_bounds_with(symbols).is_none_or(|b| b.inflate(opt.tol, opt.tol).contains(p))
}

/// Is the first child of `n` the isolated container `scope`?
fn source_of(n: &Node, scope: Option<NodeId>) -> bool {
    n.children().and_then(|children| children.first()).is_some_and(|source| Some(source.id) == scope)
}

/// Do clicks and marquees skip the child `i` of `parent` with `scope` isolated? For a Shaper
/// Group that is its original shapes (its first child), or, in Construction mode, when they are
/// isolated, the art the group shows.
fn shaper_hides(parent: &Node, i: usize, scope: Option<NodeId>) -> bool {
    parent.extras().shaper.is_some() && (i == 0) != source_of(parent, scope)
}

/// A press on the outline of clipping path `clip` (a path or a compound path; type clips by its
/// glyphs, which this crate cannot outline), within the tolerance.
fn clip_outline(clip: &Node, p: Point, opt: HitOptions) -> Option<HitKind> {
    let bp = match &clip.kind {
        NodeKind::Path { path, .. } => path.to_bezpath(),
        NodeKind::Compound { .. } => clip.stroke_path()?,
        _ => return None,
    };
    stroke_contains(&bp, 0.0, opt.tol, p).then_some(HitKind::Outline)
}

/// For the selection tools ([`HitOptions::clip_outlines`]), a press on the outline of `clip`, the
/// clipping path of `parent` (a clip group or a layer with a clipping mask), within the tolerance:
/// it is hit before the art it clips. There Direct Selection takes its segments, and the
/// Selection tool the clip group. The reference app reshapes a clipping path with Direct Selection
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-clipping-masks.html>);
/// without this, art drawn over the path's edge is hit first and its segments can't be dragged.
/// The clipping path must be one a click may hit ([`may_hit`]: visible, editable, within reach).
fn clip_outline_hit(parent: &Node, clip: &Node, p: Point, opt: HitOptions, symbols: &SymbolBoxes, editable_only: bool) -> Option<HitKind> {
    if !opt.clip_outlines || !editable_only || !may_hit(parent, 0, clip, p, opt, symbols, editable_only) {
        return None;
    }
    clip_outline(clip, p, opt)
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
    if parent.clips()
        && let Some(clip) = children.first()
    {
        if let Some(kind) = clip_outline_hit(parent, clip, p, opt, symbols, editable_only).filter(|_| !skip(clip.id)) {
            chain.push(clip.id);
            return Some(Hit { leaf: clip.id, ancestry: chain.clone(), kind, contents_of: None, layers: 1 });
        }
        // Elsewhere a clip group (or a layer with a clipping mask) only hits inside it.
        if !clip_contains(clip, p) {
            return None;
        }
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
        // An image is hit within its frame (on the frame's edge only with Selection by Path Only,
        // #3291), and where its own fills and strokes paint ([`image_paint`]) except in outline
        // mode.
        NodeKind::Image(im) => {
            if opt.path_only {
                use vectorcraft_geom::kurbo::Shape;
                let mut frame = Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)).to_path(0.1);
                frame.apply_affine(im.xf);
                if stroke_contains(&frame, 0.0, opt.tol, p) {
                    return Some(HitKind::Outline);
                }
            } else if n.geometric_bounds_with(symbols).is_some_and(|b| b.inflate(opt.tol, opt.tol).contains(p)) {
                return Some(HitKind::Bounds);
            }
            if opt.outline {
                return None;
            }
            image_paint(n).iter().rev().find_map(|(bp, rule, item)| match item {
                AppearanceItem::Stroke(s) => s.hits(bp, *rule, p, opt.tol).then_some(HitKind::Stroke),
                AppearanceItem::Fill(_) => (!opt.path_only && fill_contains(bp, *rule, p)).then_some(HitKind::Fill),
            })
        }
        NodeKind::Text(_) | NodeKind::PlacedDocument(_) | NodeKind::SymbolInstance { .. } | NodeKind::Blend { .. } | NodeKind::Envelope { .. } => {
            n.geometric_bounds_with(symbols).filter(|b| b.inflate(opt.tol, opt.tol).contains(p)).map(|_| HitKind::Bounds)
        }
        NodeKind::Repeat(r) => r.expand(Some(symbols.document())).iter().find_map(|g| hit_any(g, p, opt, symbols)),
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
            (Some(c), Some(r)) => c.iter().enumerate().filter(|(i, _)| !shaper_hides(s, *i, scope)).map(|(_, c)| (c, r)).collect(),
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
    /// Does the marquee `r` touch `n`? It skips the children of a Shaper Group that clicks skip
    /// ([`shaper_hides`]). An Intertwine group is touched by its members, and a repeat is touched
    /// by the copies it draws, as clicks hit them. Objects of every other kind except paths,
    /// compound paths, layers and groups are touched by their box.
    fn touches(n: &Node, s: &SymbolBoxes, r: Rect, scope: Option<NodeId>) -> bool {
        match &n.kind {
            NodeKind::Path { path, .. } => vectorcraft_geom::hit::intersects_rect(path, r),
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Intertwine { children } => marquee_in(n, r)
                .is_some_and(|r| children.iter().enumerate().any(|(i, c)| c.visible && !shaper_hides(n, i, scope) && touches(c, s, r, scope))),
            NodeKind::Compound { children, .. } => children.iter().any(|c| c.visible && touches(c, s, r, scope)),
            NodeKind::Repeat(spec) => spec.expand(Some(s.document())).iter().any(|copy| touches(copy, s, r, scope)),
            // A box with no width or no height (an instance of a symbol that draws a straight
            // line) is touched where the marquee crosses it.
            _ => n.geometric_bounds_with(s).is_some_and(|b| {
                let crosses = (b.width() == 0.0 || b.height() == 0.0) && b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1;
                b.intersect(r).area() > 0.0 || r.contains(b.origin()) || crosses
            }),
        }
    }
    fn collect_leaves(n: &Node, s: &SymbolBoxes, r: Rect, scope: Option<NodeId>, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Intertwine { children } => {
                let Some(r) = marquee_in(n, r) else { return };
                for (i, c) in children.iter().enumerate() {
                    if c.visible && !c.locked && !shaper_hides(n, i, scope) {
                        collect_leaves(c, s, r, scope, out);
                    }
                }
            }
            _ => {
                if touches(n, s, r, scope) {
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
            collect_leaves(t, &symbols, r, scope, &mut out);
        } else if touches(t, &symbols, r, scope) {
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
mod tests;
