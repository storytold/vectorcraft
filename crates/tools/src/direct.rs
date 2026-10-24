//! Direct Selection (A) and Group Selection tools.
//!
//! Direct Selection: click an anchor to select it (Shift toggles), click a segment to select the
//! segment independently of its endpoint anchors, drag to move selected anchors (the one pressed on snaps to
//! anchors, segments and Smart Guides, or with them off Snap to Point; Shift keeps the move at 45°
//! steps; Alt on an anchor or a segment picks the whole path and Alt-drag copies it), drag a
//! direction handle to
//! reshape (Shift keeps it at 45° steps round its anchor, Alt moves it alone; smart guides snap
//! it), marquee to select anchors (Shift-drag toggles them: the selected ones inside are
//! deselected, the others selected), drag a corner widget of any path (a star, a pen path) to round its corners
//! (the selected ones when anchors are selected; Alt-click cycles their kind, double-click opens
//! the Corners dialog).
//! Group Selection: click selects the leaf; each further click on it adds the next enclosing group;
//! its marquee works as Direct Selection's, and E pressed during the drag makes it select only the
//! objects entirely inside it (members of groups too), whole.
//! In isolation mode both pick only the isolated object and what is inside it: anchors, segments,
//! paths and groups outside it stay put.
//! Both move objects selected as a whole as the Selection tool does, snapping alike.
//! Both pick the key objects of a blend, and click or drag ruler guides ([`crate::rulerguide`]).
//! A Cmd/Ctrl+Shift double-click with either one releases the ruler guide or guide path under the
//! pointer ([`release_guide_at`]).
//! Direct Selection also edits a blend's spine: drag its points (a key object on a point moves
//! with it) and, once a point is clicked, its handles; and the points of selected gradient meshes
//! and mesh envelopes and their handles ([`MeshEdit`]; a solid fill colour recolours a gradient
//! mesh's point last clicked). Dragging a corner or an edge of area
//! type's frame reshapes the type area (`text.reshapeArea`): the text reflows at its size.
//! Dragging the brackets of selected type on a path moves or flips it ([`crate::pathtype`]).
//!
//! A press on the stroke of a path that isn't selected as a whole selects that segment's two anchors
//! (the fill, or Alt, selects the whole path); dragging the segment bends it if it's curved, else
//! moves its anchors.

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_doc::hit::{HitKind, compound_part_at, hit_test};
use vectorcraft_doc::{AnchorRef, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, Vec2};

use crate::bbox::move_delta;
use crate::corners::{self, CornerDrag, over_widget};
use crate::guides::{HandleSnap, Leave, PointSnap, Targets};
use crate::meshedit::MeshEdit;
use crate::pathtype::{self, BracketDrag, over_bracket};
use crate::rulerguide::GuideEdit;
use crate::select::{MoveSnap, hold_point, is_area_type, matrix_json, release_guide_at};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Clone, Debug)]
enum State {
    Idle,
    /// Dragging the selected anchors, which move alike: `grab` is where the anchor pressed on (or
    /// the point of a straight segment nearest the pointer pressed on it) was, the point that
    /// snaps.
    MoveAnchors {
        start: Point,
        grab: Point,
        began: bool,
    },
    /// Moving (Alt: copying) the selected objects; `grab`: the anchor an Alt-press picked the
    /// path by, which snaps as a dragged anchor does; else `hold`, where the press took hold of
    /// the object ([`hold_point`]), snaps as the Selection tool's does.
    MoveObject {
        start: Point,
        grab: Option<Point>,
        hold: Point,
        began: bool,
        /// Group Selection: Shift-click on a selected object toggles it off on release.
        deselect: Option<NodeId>,
    },
    /// Dragging a curved segment: it follows the pointer (`path.reshapeSegment`).
    Segment {
        id: NodeId,
        si: usize,
        seg: usize,
        t: f64,
        start: Point,
        began: bool,
    },
    Handle {
        id: NodeId,
        si: usize,
        ai: usize,
        out: bool,
    },
    Marquee {
        start: Point,
        cur: Point,
        /// Shift: the anchors inside leave the selection if selected, else join it.
        toggle: bool,
        /// Group Selection, E pressed an odd number of times during the drag: the marquee selects
        /// only the objects that lie entirely inside it, whole.
        enclosed: bool,
    },
    Corner(CornerDrag),
    /// Dragging a bracket of type on a path.
    Bracket(BracketDrag),
    /// Dragging a point of a blend's spine from `from`.
    SpinePoint {
        id: NodeId,
        anchor: usize,
        from: Point,
        start: Point,
        began: bool,
    },
    /// Dragging a curved segment of a selected blend's spine: it follows the pointer
    /// (`object.blend.spine.reshapeSegment`, #1111).
    SpineSegment {
        id: NodeId,
        seg: usize,
        t: f64,
        start: Point,
        began: bool,
    },
    /// Dragging a handle of the clicked spine point.
    SpineHandle {
        id: NodeId,
        anchor: usize,
        out: bool,
    },
    /// Dragging a mesh point or handle ([`MeshEdit`]).
    Mesh,
    /// Dragging anchors of area type's frame (a corner, or the two ends of an edge).
    TypeArea {
        id: NodeId,
        anchors: Vec<AnchorRef>,
        start: Point,
        began: bool,
    },
}

/// How far (pt) a snap must move a dragged point for the arrow to say it snapped.
const SNAPPED: f64 = 1e-9;

/// Whether a drag step snapped: the snapped move `d` left where the pointer alone put it
/// (`unsnapped`). With Shift the constraint (General › Constrain Angle) moves it too, so only a
/// snap's guide tells then.
fn snapped(d: Vec2, unsnapped: Vec2, shift: bool, guides: &[Overlay]) -> bool {
    if shift { !guides.is_empty() } else { (d - unsnapped).hypot() > SNAPPED }
}

pub struct DirectSelectionTool {
    group: bool,
    state: State,
    /// The spine point last clicked (blend, anchor): its handles show and can be dragged.
    spine: Option<(NodeId, usize)>,
    mesh: MeshEdit,
    /// Smart guides of the handle, anchors or objects being dragged.
    guides: Vec<Overlay>,
    /// What the dragged anchors snap to, gathered when the drag begins.
    anchor_snap: Option<PointSnap>,
    /// What the anchor holding paths moved whole snaps to while Alt makes a copy: the original left
    /// behind too. Gathered with [`Self::anchor_snap`], before the previews move the original.
    copy_snap: Option<PointSnap>,
    /// What the dragged handle snaps to.
    handle_snap: HandleSnap,
    /// What the objects being moved snap to, gathered when the move begins.
    move_snap: Option<MoveSnap>,
    /// The drag snapped to a point or a guide at its last step: the arrow turns hollow.
    snapped: bool,
    /// The anchor under the pointer (Highlight anchors on mouse over): its path, where it is.
    hover: Option<(NodeId, Point)>,
    guide: GuideEdit,
    /// The already selected anchor the press landed on: a click (no drag) selects it alone, as a
    /// drag moves every selected anchor with it.
    click: Option<(NodeId, usize, usize)>,
    /// A click on a whole path picks the segment on release; a drag still moves the whole path.
    click_segment: Option<(NodeId, usize, usize)>,
}

impl DirectSelectionTool {
    pub fn new(group: bool) -> Self {
        Self {
            group,
            state: State::Idle,
            spine: None,
            mesh: MeshEdit::default(),
            guides: vec![],
            anchor_snap: None,
            copy_snap: None,
            handle_snap: HandleSnap::default(),
            move_snap: None,
            snapped: false,
            hover: None,
            guide: GuideEdit::default(),
            click: None,
            click_segment: None,
        }
    }

    /// Is `key` the E that Group Selection takes while a marquee is being dragged? Direct
    /// Selection takes no letter.
    fn claims_letter(&self, key: ToolKey) -> bool {
        self.group && key == ToolKey::Letter('e') && matches!(self.state, State::Marquee { .. })
    }
}

/// Whether `id` may be picked while `scope` is isolated: it is that object, or it sits inside it.
/// With no isolation, everything may be picked.
fn in_scope(doc: &vectorcraft_doc::Document, scope: Option<NodeId>, id: NodeId) -> bool {
    scope.is_none_or(|scope| doc.ancestry(id).is_some_and(|anc| anc.contains(&scope)))
}

/// Visit the nodes Direct Selection may pick. In isolation mode that is the isolated object and
/// its contents; art around it is locked and the walk does not reach it. A Shaper Group's original
/// shapes, its first child, are left out unless they are the isolated object (Construction mode),
/// as clicks leave them out.
fn walk_pickable<'a>(doc: &'a vectorcraft_doc::Document, scope: Option<NodeId>, mut f: impl FnMut(&'a Node)) {
    fn walk<'a>(n: &'a Node, f: &mut impl FnMut(&'a Node)) {
        f(n);
        for c in n.children().into_iter().flatten().skip(usize::from(n.extras().shaper.is_some())) {
            walk(c, f);
        }
    }
    if let Some(id) = scope
        && let Some(n) = doc.node(id)
    {
        walk(n, &mut f);
    } else if scope.is_none() {
        for l in &doc.layers {
            walk(l, &mut f);
        }
    }
}

/// The spines of the editable blends, topmost first: (blend, spine).
pub(crate) fn spines(cx: &ToolContext) -> Vec<(NodeId, PathData)> {
    let mut v = vec![];
    walk_pickable(cx.doc, cx.isolation, |n| {
        if let NodeKind::Blend { children, spec } = &n.kind
            && let Some((path, _)) = vectorcraft_doc::live::blend_spine(children, spec)
        {
            v.push((n.id, path));
        }
    });
    v.reverse();
    let editable = cx.doc.editable();
    v.retain(|(id, _)| editable(*id));
    v
}

/// The spine point under `p`: (blend, anchor, where it is).
fn hit_spine_point(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, Point)> {
    spines(cx).into_iter().find_map(|(id, path)| {
        let (_, ai, a) = path.anchors().find(|(_, _, a)| a.p.distance(p) <= tol)?;
        Some((id, ai, a.p))
    })
}

/// The segment of a selected blend's spine passing within `tol` of `p`: (blend, segment, t, whether
/// it is straight).
pub(crate) fn hit_spine_segment(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, f64, bool)> {
    spines(cx).into_iter().filter(|(id, _)| cx.selection.contains(*id)).find_map(|(id, path)| {
        let (si, seg, t, _, dist) = path.nearest(p)?;
        (dist <= tol).then(|| (id, seg, t, path.subpaths.get(si).is_some_and(|sp| sp.segment_is_line(seg))))
    })
}

/// The spine point `sel` and whether `p` is on its outgoing handle's end (`Some(true)`) or its
/// incoming one's.
fn hit_spine_handle(cx: &ToolContext, sel: (NodeId, usize), p: Point, tol: f64) -> Option<bool> {
    let a = spine_anchor(cx, sel)?;
    if a.has_out() && a.h_out.distance(p) <= tol {
        return Some(true);
    }
    (a.has_in() && a.h_in.distance(p) <= tol).then_some(false)
}

/// Anchor `sel.1` of blend `sel.0`'s spine.
fn spine_anchor(cx: &ToolContext, (id, ai): (NodeId, usize)) -> Option<vectorcraft_geom::Anchor> {
    match &cx.doc.node(id)?.kind {
        NodeKind::Blend { children, spec } => vectorcraft_doc::live::blend_spine(children, spec)?.0.subpaths.first()?.anchors.get(ai).copied(),
        _ => None,
    }
}

/// The anchors Direct Selection edits on `n`, in document space: a path's, or area type's frame.
fn editable_path(n: &Node) -> Option<Cow<'_, PathData>> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(Cow::Borrowed(path)),
        NodeKind::Text(t) => t.area_frame().map(Cow::Owned),
        _ => None,
    }
}

/// The editable objects `f` accepts, topmost first.
fn anchor_owners(cx: &ToolContext, f: fn(&Node) -> bool) -> Vec<NodeId> {
    let mut v = vec![];
    walk_pickable(cx.doc, cx.isolation, |n| {
        if f(n) {
            v.push(n.id)
        }
    });
    v.reverse();
    let editable = cx.doc.editable();
    v.retain(|id| editable(*id));
    v
}

/// The anchor within `tol` of `p` nearest to it, of the topmost selected path or area type frame
/// that has one, else of the topmost visible one: (id, si, ai, where it is). Selected objects come
/// first so an object above can't take the press from the one being edited (#1508), as
/// [`hovered_anchor`] marks it. In normal viewmode, an opaque fill in front occludes unselected
/// objects behind it (#3405).
fn hit_anchor(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, Point)> {
    let mut owners = anchor_owners(cx, |n| matches!(n.kind, NodeKind::Path { .. }) || is_area_type(n));
    owners.sort_by_key(|id| !cx.selection.contains(*id));

    // 1. Check selected objects first. Selected objects are NEVER occluded (#1508).
    let mut unselected_idx = owners.len();
    for (i, &id) in owners.iter().enumerate() {
        if !cx.selection.contains(id) {
            unselected_idx = i;
            break;
        }
        if let Some(pd) = cx.doc.node(id).and_then(editable_path)
            && let Some((si, ai, a)) = nearest_anchor(&pd, p, tol)
        {
            return Some((id, si, ai, a));
        }
    }

    // 2. For unselected objects: in normal viewmode, an opaque fill in front occludes
    // unselected objects behind it (#3405).
    let unselected = &owners[unselected_idx..];
    if unselected.is_empty() {
        return None;
    }

    let filter: Option<Box<dyn Fn(NodeId) -> bool>> = if !cx.outline
        && let Some(h) = hit_test(cx.doc, p, cx.selection_hit_options())
        && matches!(h.kind, HitKind::Fill)
    {
        let leaf = h.leaf;
        let ancestry = h.ancestry.clone();
        let part = cx.doc.node(leaf).and_then(|n| compound_part_at(n, p, tol));
        Some(Box::new(move |id| id == leaf || ancestry.contains(&id) || part == Some(id)))
    } else {
        None
    };

    unselected.iter().copied().find_map(|id| {
        if let Some(f) = &filter
            && !f(id)
        {
            return None;
        }
        let pd = cx.doc.node(id).and_then(editable_path)?;
        nearest_anchor(&pd, p, tol).map(|(si, ai, a)| (id, si, ai, a))
    })
}

/// The anchor of `pd` within `tol` of `p` nearest to it: (subpath, anchor, where it is).
fn nearest_anchor(pd: &PathData, p: Point, tol: f64) -> Option<(usize, usize, Point)> {
    pd.anchors().map(|(si, ai, a)| (si, ai, a.p)).filter(|(.., a)| a.distance(p) <= tol).min_by(|x, y| x.2.distance(p).total_cmp(&y.2.distance(p)))
}

/// An edge of an area type frame under `p`, topmost first: (text, the edge's two anchors).
fn hit_frame_edge(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, Vec<AnchorRef>)> {
    anchor_owners(cx, is_area_type).into_iter().find_map(|id| {
        let frame = cx.doc.node(id).and_then(editable_path)?;
        let (si, seg, _, _, d) = frame.nearest(p)?;
        let n = frame.subpaths.get(si)?.anchors.len();
        (d <= tol && n > 0).then(|| (id, vec![(si, seg % n), (si, (seg + 1) % n)]))
    })
}

/// The anchors whose direction handles show, and drag, with the tools that edit anchors
/// ([`crate::catalog::edits_anchors`]): the direct-selected anchors of a path, every anchor of a
/// path selected as a whole. Selection & Anchor Display → Show handles when multiple anchors are
/// selected off, none while more than one anchor is selected.
fn handle_anchors(cx: &ToolContext) -> Vec<(NodeId, usize, usize, Anchor)> {
    let mut v = vec![];
    for &id in &cx.selection.objects {
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { continue };
        let shown = cx.selection.partial(id);
        v.extend(pd.anchors().filter(|(si, ai, _)| shown.is_none_or(|set| set.contains(&(*si, *ai)))).map(|(si, ai, a)| {
            let mut a = *a;
            if let Some(sp) = pd.subpaths.get(si) {
                let (inside, outside) = cx.selection.handle_sides(id, si, ai, sp);
                if !inside {
                    a.h_in = a.p;
                }
                if !outside {
                    a.h_out = a.p;
                }
            }
            (id, si, ai, a)
        }));
    }
    if !cx.handles_multiple && v.len() > 1 {
        v.clear();
    }
    v
}

/// The end of a shown direction handle ([`handle_anchors`]) nearest to `p` within `tol`, and
/// nearer to it than the handle's anchor (a short handle leaves its anchor to be picked): (path,
/// subpath, anchor, is_out).
pub(crate) fn hit_handle(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, bool)> {
    let mut best: Option<((NodeId, usize, usize, bool), f64)> = None;
    for (id, si, ai, a) in handle_anchors(cx) {
        for (out, has, h) in [(true, a.has_out(), a.h_out), (false, a.has_in(), a.h_in)] {
            let d = h.distance(p);
            if has && d <= tol && d < a.p.distance(p) && best.is_none_or(|b| d < b.1) {
                best = Some(((id, si, ai, out), d));
            }
        }
    }
    best.map(|b| b.0)
}

fn anchors_json(v: &[AnchorRef]) -> Value {
    Value::Array(v.iter().map(|(s, a)| json!([s, a])).collect())
}

/// A segment of a path under the pointer.
struct SegmentHit {
    si: usize,
    seg: usize,
    t: f64,
    curved: bool,
}

/// The segment of path `id` nearest to `p`, where the hit test found its stroke.
fn segment_at(cx: &ToolContext, id: NodeId, p: Point) -> Option<SegmentHit> {
    let pd = cx.doc.node(id)?.path_data()?;
    let (si, seg, t, ..) = pd.nearest(p)?;
    let sp = pd.subpaths.get(si)?;
    let n = sp.anchors.len();
    let (i0, i1) = (seg.checked_rem(n)?, (seg + 1).checked_rem(n)?);
    let (a, b) = (sp.anchors.get(i0)?, sp.anchors.get(i1)?);
    Some(SegmentHit { si, seg, t, curved: a.has_out() || b.has_in() })
}

impl Tool for DirectSelectionTool {
    fn id(&self) -> &'static str {
        if self.group { "groupSelection" } else { "directSelection" }
    }
    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle) || self.guide.busy()
    }
    fn options(&self) -> Value {
        self.mesh.options()
    }
    /// Escape during a drag (anchors, handles, segments, objects, type areas, blend spines, a
    /// marquee) cancels it: what it moved goes back and the tool waits for the next press (#1743).
    /// With Group Selection, E during a marquee drag makes the marquee select only the objects
    /// entirely inside it, whole, and E pressed again makes it select what it selects by default.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if self.claims_letter(key)
            && let State::Marquee { enclosed, .. } = &mut self.state
        {
            *enclosed = !*enclosed;
            return vec![];
        }
        if key == ToolKey::Escape && self.guide.busy() {
            return self.guide.cancel();
        }
        if key != ToolKey::Escape || matches!(self.state, State::Idle | State::Mesh | State::Corner(_) | State::Bracket(_)) {
            return vec![];
        }
        self.state = State::Idle;
        self.guides.clear();
        (self.anchor_snap, self.copy_snap, self.move_snap) = (None, None, None);
        (self.click, self.click_segment) = (None, None);
        vec![Action::Cancel]
    }
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.claims_letter(key)
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if let Some(out) = self.guide.pointer(cx, ev) {
            return out;
        }
        let p = ev.pos;
        // Anchors and handle ends are picked from a little further than segments and objects.
        let (tol, point) = (cx.pick_tol(), cx.point_tol());
        match (ev.kind, self.state.clone()) {
            (PointerKind::Down, _) if self.group => {
                if let Some(out) = self.guide.press(cx, ev) {
                    self.state = State::Idle;
                    return out;
                }
                let Some(mut h) = hit_test(cx.doc, p, cx.selection_hit_options()) else {
                    self.state = State::Marquee { start: p, cur: p, toggle: ev.mods.shift, enclosed: false };
                    return vec![];
                };
                // A compound path's paths are picked one at a time, below the compound path (#2441).
                if let Some(part) = cx.doc.node(h.leaf).and_then(|n| compound_part_at(n, p, tol)) {
                    h.ancestry.push(part);
                    h.leaf = part;
                }
                // Walk up from the leaf: select the first ancestor not yet selected (below the layer).
                // Stop at the isolated object: the next group up is outside isolation mode.
                let chain: Vec<NodeId> = h.ancestry.iter().skip(1).rev().copied().take_while(|id| in_scope(cx.doc, cx.isolation, *id)).collect();
                let target = if cx.selection.contains(h.leaf) {
                    chain.iter().find(|id| !cx.selection.contains(**id)).copied().unwrap_or(h.leaf)
                } else {
                    h.leaf
                };
                let mut deselect = None;
                let mut out = vec![];
                if ev.mods.shift {
                    // Shift on a selected member deselects what was hit (a compound path's part),
                    // not the next ancestor a plain click would add (#2441).
                    if cx.selection.contains(h.leaf) {
                        deselect = Some(h.leaf);
                    } else {
                        out.push(Action::Exec("select.toggle".into(), json!({"id": target.0})));
                    }
                } else if !cx.selection.contains(target) {
                    if cx.selection.contains(h.leaf) {
                        out.push(Action::Exec("select.add".into(), json!({"ids": [target.0]})));
                    } else {
                        out.push(Action::Exec("select.set".into(), json!({"ids": [target.0]})));
                    }
                }
                self.state = State::MoveObject { start: p, grab: None, hold: hold_point(cx, &h, p), began: false, deselect };
                out
            }
            (PointerKind::DoubleClick, _) if self.group => release_guide_at(cx, p, ev.mods).into_iter().collect(),
            (PointerKind::DoubleClick, _) if !self.group => {
                self.state = State::Idle;
                if let Some(a) = corners::double_click(cx, p, true) {
                    return vec![a];
                }
                if let Some(a) = release_guide_at(cx, p, ev.mods) {
                    return vec![a];
                }
                if crate::rulerguide::guide_at(cx, p).is_some() {
                    return vec![];
                }
                // A double-click edits what is drawn: not a clipping path's outline.
                if hit_test(cx.doc, p, cx.hit_options()).and_then(|h| cx.doc.node(h.leaf)).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))) {
                    return vec![Action::SwitchTool("type".into())];
                }
                vec![]
            }
            (PointerKind::Move, State::Idle) => {
                self.hover = if self.group || !cx.highlight_anchors { None } else { hovered_anchor(cx, p) };
                vec![]
            }
            (PointerKind::Down, _) => {
                self.click = None;
                self.click_segment = None;
                if let Some(c) = CornerDrag::hit(cx, ev, true) {
                    self.state = State::Corner(c);
                    return vec![];
                }
                if let Some(b) = BracketDrag::hit(cx, ev) {
                    self.state = State::Bracket(b);
                    return vec![];
                }
                if let Some(sel) = self.spine.filter(|(id, _)| cx.selection.contains(*id))
                    && let Some(out) = hit_spine_handle(cx, sel, p, point)
                {
                    self.state = State::SpineHandle { id: sel.0, anchor: sel.1, out };
                    return vec![Action::Begin("Reshape Spine".into())];
                }
                self.spine = None;
                self.snapped = false;
                if let Some(g) = self.mesh.hit(cx, &cx.selection.objects, p) {
                    self.state = State::Mesh;
                    return self.mesh.press(cx, g);
                }
                self.mesh.unfocus();
                if let Some((id, si, ai, out)) = hit_handle(cx, p, point) {
                    self.state = State::Handle { id, si, ai, out };
                    self.handle_snap = HandleSnap::default();
                    return vec![Action::Begin("Reshape".into())];
                }
                if let Some((id, anchor, from)) = hit_spine_point(cx, p, point) {
                    self.spine = Some((id, anchor));
                    self.state = State::SpinePoint { id, anchor, from, start: p, began: false };
                    return if cx.selection.contains(id) { vec![] } else { vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))] };
                }
                if let Some((id, si, ai, grab)) = hit_anchor(cx, p, point) {
                    if cx.doc.node(id).is_some_and(is_area_type) {
                        return self.press_type_area(cx, id, vec![(si, ai)], p, ev.mods.shift);
                    }
                    // Alt picks the whole path (and Alt-drag copies it), as on a segment.
                    if ev.mods.alt {
                        self.state = State::MoveObject { start: p, grab: Some(grab), hold: grab, began: false, deselect: None };
                        let whole = cx.selection.contains(id) && cx.selection.partial(id).is_none();
                        return if whole { vec![] } else { vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))] };
                    }
                    // A path with all its anchors selected is stored whole (no partial entry).
                    let already = cx.selection.contains(id)
                        && !cx.selection.segments.contains_key(&id)
                        && cx.selection.partial(id).is_none_or(|s| s.contains(&(si, ai)));
                    // Paths all selected whole move as objects, held by this anchor, so Alt can turn
                    // the copy on and off during the drag as with the Selection tool.
                    self.state = if already && cx.selection.anchors.is_empty() {
                        State::MoveObject { start: p, grab: Some(grab), hold: grab, began: false, deselect: None }
                    } else {
                        State::MoveAnchors { start: p, grab, began: false }
                    };
                    if ev.mods.shift {
                        return vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "toggle"}))];
                    }
                    // Pressed among other selected anchors: a click selects this one alone (the
                    // drag, which moves them all, keeps them).
                    self.click = already.then_some((id, si, ai));
                    if !already {
                        return vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "set"}))];
                    }
                    return vec![];
                }
                if let Some((id, anchors)) = hit_frame_edge(cx, p, tol) {
                    return self.press_type_area(cx, id, anchors, p, ev.mods.shift);
                }
                // A curved segment of a selected blend's spine reshapes, as a path's does.
                if let Some((id, seg, t, false)) = hit_spine_segment(cx, p, tol) {
                    self.state = State::SpineSegment { id, seg, t, start: p, began: false };
                    return vec![];
                }
                if let Some(out) = self.guide.press(cx, ev) {
                    self.state = State::Idle;
                    return out;
                }
                if let Some(mut h) = hit_test(cx.doc, p, cx.selection_hit_options()) {
                    if let Some(part) = cx.doc.node(h.leaf).and_then(|n| compound_part_at(n, p, tol)) {
                        h.ancestry.push(part);
                        h.leaf = part;
                    }
                    // A segment pick keeps its endpoints available for movement without selecting
                    // the anchors themselves. Dragging reshapes a curve, else moves it. Alt picks
                    // the whole path (and Alt-drag copies it), as with Group Selection.
                    let whole = cx.selection.contains(h.leaf) && cx.selection.partial(h.leaf).is_none();
                    if !ev.mods.alt
                        && matches!(h.kind, HitKind::Stroke | HitKind::Outline)
                        && let Some(s) = segment_at(cx, h.leaf, p)
                    {
                        if whole && !ev.mods.shift {
                            self.click_segment = Some((h.leaf, s.si, s.seg));
                        } else {
                            let selected = cx.selection.segments.get(&h.leaf).is_some_and(|sel| sel.contains(&(s.si, s.seg)));
                            self.state = if s.curved && !ev.mods.shift {
                                State::Segment { id: h.leaf, si: s.si, seg: s.seg, t: s.t, start: p, began: false }
                            } else {
                                // Held by the segment itself, so the segment is what lands on a
                                // ruler guide or lines up, not the pressed spot beside it (#3701).
                                State::MoveAnchors { start: p, grab: hold_point(cx, &h, p), began: false }
                            };
                            if ev.mods.shift {
                                return vec![Action::Exec(
                                    "select.segments".into(),
                                    json!({"id": h.leaf.0, "segments": [[s.si, s.seg]], "mode": "toggle"}),
                                )];
                            }
                            if selected {
                                return vec![];
                            }
                            return vec![Action::Exec("select.segments".into(), json!({"id": h.leaf.0, "segments": [[s.si, s.seg]]}))];
                        }
                    }
                    // Clicking the fill selects the whole leaf path (all anchors).
                    self.state = State::MoveObject { start: p, grab: None, hold: hold_point(cx, &h, p), began: false, deselect: None };
                    if ev.mods.shift {
                        return vec![Action::Exec("select.toggle".into(), json!({"id": h.leaf.0}))];
                    }
                    if !cx.selection.contains(h.leaf) || cx.selection.partial(h.leaf).is_some() {
                        return vec![Action::Exec("select.set".into(), json!({"ids": [h.leaf.0]}))];
                    }
                    return vec![];
                }
                self.state = State::Marquee { start: p, cur: p, toggle: ev.mods.shift, enclosed: false };
                vec![]
            }
            (PointerKind::Drag, State::MoveAnchors { start, grab, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Move".into()));
                    self.state = State::MoveAnchors { start, grab, began: true };
                    self.anchor_snap = Some(PointSnap::new(cx, || Targets::for_anchor_drag(cx.doc, cx.selection)));
                }
                let mut d = move_delta(start, p, ev.mods.shift, cx.constrain_angle);
                let unsnapped = d;
                // The grabbed anchor snaps and the others follow it. Shift keeps the move at its
                // angle, and the anchor slides along it onto what Smart Guides find there (#886).
                self.guides.clear();
                if let Some(snap) = &self.anchor_snap {
                    let shift = ev.mods.shift.then(|| Leave::segment(cx, grab, true));
                    let (q, guides) = snap.snap_from(cx, grab + (p - start), shift.as_ref());
                    (d, self.guides) = (q - grab, guides);
                }
                self.snapped = snapped(d, unsnapped, ev.mods.shift, &self.guides);
                out.push(Action::Preview("path.moveAnchors".into(), json!({"dx": d.x, "dy": d.y})));
                out
            }
            (PointerKind::Drag, State::Segment { id, si, seg, t, start, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Reshape".into()));
                    self.state = State::Segment { id, si, seg, t, start, began: true };
                }
                let d = p - start;
                out.push(Action::Preview(
                    "path.reshapeSegment".into(),
                    json!({"id": id.0, "subpath": si, "segment": seg, "t": t, "dx": d.x, "dy": d.y, "perpendicular": ev.mods.shift}),
                ));
                out
            }
            (PointerKind::Drag, State::MoveObject { start, grab, hold, began, .. }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin(if ev.mods.alt { "Copy".into() } else { "Move".into() }));
                    self.state = State::MoveObject { start, grab, hold, began: true, deselect: None };
                    if grab.is_some() {
                        // Alt can go down or up at any time: gather both while nothing has moved.
                        self.anchor_snap = Some(PointSnap::new(cx, || Targets::for_anchor_drag(cx.doc, cx.selection)));
                        self.copy_snap = Some(PointSnap::new(cx, || Targets::for_anchor_drag(cx.doc, &Selection::default())));
                    } else {
                        self.move_snap = Some(MoveSnap::new(cx));
                    }
                }
                let mut d = move_delta(start, p, ev.mods.shift, cx.constrain_angle);
                let unsnapped = d;
                self.guides.clear();
                let snap = if ev.mods.alt { &self.copy_snap } else { &self.anchor_snap };
                if let (Some(grab), Some(snap)) = (grab, snap) {
                    let shift = ev.mods.shift.then(|| Leave::segment(cx, grab, true));
                    let (q, guides) = snap.snap_from(cx, grab + (p - start), shift.as_ref());
                    (d, self.guides) = (q - grab, guides);
                } else if let Some(snap) = &self.move_snap {
                    (d, self.guides) = if ev.mods.shift { snap.snap_along(cx, hold, d, ev.mods.alt) } else { snap.snap(cx, hold, d, ev.mods.alt) };
                }
                self.snapped = snapped(d, unsnapped, ev.mods.shift, &self.guides);
                out.push(Action::Preview(
                    "object.transform".into(),
                    json!({"matrix": matrix_json(vectorcraft_geom::Affine::translate(d)), "copy": ev.mods.alt}),
                ));
                out
            }
            (PointerKind::Drag, State::TypeArea { id, anchors, start, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Reshape Type Area".into()));
                }
                let d = move_delta(start, p, ev.mods.shift, cx.constrain_angle);
                out.push(Action::Preview("text.reshapeArea".into(), json!({"id": id.0, "anchors": anchors_json(&anchors), "dx": d.x, "dy": d.y})));
                self.state = State::TypeArea { id, anchors, start, began: true };
                out
            }
            (PointerKind::Drag, State::Handle { id, si, ai, out }) => {
                let (q, guides) = self.handle_snap.snap(cx, (id, si, ai), p, ev.mods.shift);
                self.snapped = snapped(q.to_vec2(), p.to_vec2(), ev.mods.shift, &guides);
                self.guides = guides;
                vec![Action::Preview(
                    "path.setHandle".into(),
                    json!({"id": id.0, "subpath": si, "anchor": ai, "which": if out {"out"} else {"in"}, "x": q.x, "y": q.y, "independent": ev.mods.alt}),
                )]
            }
            (PointerKind::Drag, State::SpinePoint { id, anchor, from, start, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Reshape Spine".into()));
                    self.state = State::SpinePoint { id, anchor, from, start, began: true };
                }
                let q = from + move_delta(start, p, ev.mods.shift, cx.constrain_angle);
                out.push(Action::Preview("object.blend.spine.moveAnchor".into(), json!({"id": id.0, "anchor": anchor, "x": q.x, "y": q.y})));
                out
            }
            (PointerKind::Drag, State::SpineSegment { id, seg, t, start, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Reshape Spine".into()));
                    self.state = State::SpineSegment { id, seg, t, start, began: true };
                }
                let d = p - start;
                out.push(Action::Preview(
                    "object.blend.spine.reshapeSegment".into(),
                    json!({"id": id.0, "segment": seg, "t": t, "dx": d.x, "dy": d.y, "perpendicular": ev.mods.shift}),
                ));
                out
            }
            (PointerKind::Drag, State::SpineHandle { id, anchor, out }) => vec![Action::Preview(
                "object.blend.spine.moveAnchor".into(),
                json!({"id": id.0, "anchor": anchor, "handle": if out {"out"} else {"in"}, "x": p.x, "y": p.y, "independent": ev.mods.alt}),
            )],
            (PointerKind::Drag, State::Mesh) => self.mesh.drag_to(p).unwrap_or_default(),
            (PointerKind::Up, State::Mesh) => {
                self.state = State::Idle;
                self.mesh.release().unwrap_or_default()
            }
            (PointerKind::Drag, State::Marquee { start, toggle, enclosed, .. }) => {
                self.state = State::Marquee { start, cur: p, toggle, enclosed };
                vec![]
            }
            (PointerKind::Drag, State::Corner(mut c)) => {
                let out = c.drag(cx, p);
                self.state = State::Corner(c);
                out
            }
            (PointerKind::Up, State::Corner(c)) => {
                self.state = State::Idle;
                c.finish()
            }
            (PointerKind::Drag, State::Bracket(mut b)) => {
                let out = b.drag(cx, p, ev.mods.cmd);
                self.state = State::Bracket(b);
                out
            }
            (PointerKind::Up, State::Bracket(b)) => {
                self.state = State::Idle;
                b.finish()
            }
            (
                PointerKind::Up,
                State::MoveAnchors { began, .. }
                | State::MoveObject { began, .. }
                | State::Segment { began, .. }
                | State::SpinePoint { began, .. }
                | State::SpineSegment { began, .. }
                | State::TypeArea { began, .. },
            ) => {
                let deselect = match self.state {
                    State::MoveObject { deselect, .. } => deselect,
                    _ => None,
                };
                self.state = State::Idle;
                self.guides.clear();
                (self.anchor_snap, self.copy_snap, self.move_snap) = (None, None, None);
                match (began, deselect, self.click.take(), self.click_segment.take()) {
                    (true, ..) => vec![Action::Commit],
                    (false, Some(id), _, _) => vec![Action::Exec("select.toggle".into(), json!({"id": id.0}))],
                    (false, None, Some((id, si, ai)), _) => {
                        vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "set"}))]
                    }
                    (false, None, None, Some((id, si, seg))) => {
                        vec![Action::Exec("select.segments".into(), json!({"id": id.0, "segments": [[si, seg]]}))]
                    }
                    (false, None, None, None) => vec![],
                }
            }
            (PointerKind::Up, State::Handle { .. } | State::SpineHandle { .. }) => {
                self.state = State::Idle;
                self.guides.clear();
                vec![Action::Commit]
            }
            (PointerKind::Up, State::Marquee { start, toggle, enclosed, .. }) => {
                self.state = State::Idle;
                let r = Rect::from_points(start, p);
                if r.width() < cx.tol(3.0) && r.height() < cx.tol(3.0) {
                    return if toggle { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
                }
                // With Group Selection's enclosed marquee, the tool selects the objects inside it
                // (members of groups too), each whole, as a click selects one.
                if enclosed {
                    let mut touched = vectorcraft_doc::hit::marquee(cx.doc, r, cx.isolation, true);
                    let editable = cx.doc.editable();
                    touched.retain(|id| editable(*id));
                    let ids: Vec<Value> = crate::select::inside(cx, r, touched).into_iter().map(|id| json!(id.0)).collect();
                    let art = Action::Exec(if toggle { "select.toggle" } else { "select.set" }.into(), json!({ "ids": ids }));
                    return std::iter::once(art).chain(crate::rulerguide::marquee_select(cx, r, true)).collect();
                }
                // Anchors inside the rect, on editable paths inside isolation mode.
                let mut sel: Vec<(NodeId, Vec<AnchorRef>)> = vec![];
                walk_pickable(cx.doc, cx.isolation, |n| {
                    if let NodeKind::Path { path, .. } = &n.kind {
                        let v: Vec<AnchorRef> = path.anchors().filter(|(_, _, a)| r.contains(a.p)).map(|(s, i, _)| (s, i)).collect();
                        if !v.is_empty() {
                            sel.push((n.id, v));
                        }
                    }
                });
                // Objects without anchors to pick (type, images, symbols…) are selected whole when
                // the marquee touches them, as with the Selection tool (#927).
                for id in vectorcraft_doc::hit::marquee(cx.doc, r, cx.isolation, true) {
                    let whole = cx.doc.node(id).is_some_and(|n| !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }));
                    if whole && !sel.iter().any(|(s, _)| *s == id) {
                        sel.push((id, vec![]));
                    }
                }
                let editable = cx.doc.editable();
                sel.retain(|(id, _)| editable(*id));
                let items: Vec<Value> = sel.iter().map(|(id, v)| json!({"id": id.0, "anchors": anchors_json(v)})).collect();
                let art = Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": if toggle { "toggle" } else { "set" }}));
                std::iter::once(art).chain(crate::rulerguide::marquee_select(cx, r, false)).collect()
            }
            _ => vec![],
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        match &self.state {
            State::Marquee { start, cur, .. } => vec![Overlay::Marquee(Rect::from_points(*start, *cur))],
            State::Corner(c) => c.overlays(cx),
            _ => {
                let mut out = self.spine_overlays(cx);
                out.extend(self.mesh.overlays(cx));
                if !self.group {
                    out.extend(frame_overlays(cx));
                    out.extend(pathtype::overlays(cx));
                }
                if matches!(self.state, State::Handle { .. } | State::MoveAnchors { .. } | State::MoveObject { .. }) {
                    out.extend(self.guides.iter().cloned());
                }
                if let Some((id, p)) = self.hover.filter(|_| matches!(self.state, State::Idle)) {
                    out.push(Overlay::Anchor { p, color: cx.doc.layer_color(id), filled: false, size: 8.0 });
                }
                out.extend(self.guide.overlays(cx));
                out
            }
        }
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if !self.group && (matches!(self.state, State::Corner(_)) || over_widget(cx, p, true)) {
            return Cursor::CornerRadius;
        }
        if !self.group && (matches!(self.state, State::Bracket(_)) || over_bracket(cx, p)) {
            return Cursor::PathBracket;
        }
        let rest = if self.group { Cursor::GroupSelection } else { Cursor::ArrowHollow };
        if !self.group {
            match self.state {
                // Alt-dragging leaves a copy behind.
                State::MoveObject { began: true, .. } if m.alt => return Cursor::SelectionCopy,
                // Bending a segment keeps the cursor that offered it.
                State::Segment { began: true, .. } | State::SpineSegment { began: true, .. } => return Cursor::ReshapeSegment,
                // A drag shows the filled arrow, hollow while it snaps to a point or a guide.
                State::Handle { .. }
                | State::SpineHandle { .. }
                | State::SpinePoint { began: true, .. }
                | State::TypeArea { began: true, .. }
                | State::MoveAnchors { began: true, .. }
                | State::MoveObject { began: true, .. } => {
                    return if self.snapped { Cursor::ArrowHollow } else { Cursor::Arrow };
                }
                // Alt makes it the Group Selection tool for the next press, unless the press takes
                // a handle, a mesh or spine point, a type frame's edge or a ruler guide.
                State::Idle if m.alt => {
                    let other = self.takes_point(cx, p)
                        || hovered_anchor(cx, p).is_none() && (hit_frame_edge(cx, p, cx.pick_tol()).is_some() || self.guide.cursor(cx, p).is_some());
                    if !other {
                        return Cursor::GroupSelection;
                    }
                }
                State::Idle => {
                    if let Some(c) = self.part_under(cx, p, m) {
                        return c;
                    }
                }
                _ => {}
            }
        }
        // A press on the highlighted anchor picks it, not the guide.
        self.guide.cursor(cx, p).filter(|_| self.hover.is_none() || self.guide.busy()).unwrap_or(rest)
    }
}

impl DirectSelectionTool {
    /// A press at `p` takes a mesh point or handle, a direction handle's end, the clicked spine
    /// point's handle or a spine point, before any anchor or segment.
    fn takes_point(&self, cx: &ToolContext, p: Point) -> bool {
        let point = cx.point_tol();
        self.mesh.hit(cx, &cx.selection.objects, p).is_some()
            || hit_handle(cx, p, point).is_some()
            || self.spine.is_some_and(|sel| hit_spine_handle(cx, sel, p, point).is_some())
            || hit_spine_point(cx, p, point).is_some()
    }

    /// The cursor for the path part a press at `p` takes, in the press's order: an anchor (its
    /// square filled when its path is selected), then a curved segment a drag reshapes (a path's
    /// not selected whole, or a selected blend's spine's). Handles, mesh and spine points, frame
    /// edges, ruler guides, straight segments and fills keep the plain cursor.
    fn part_under(&self, cx: &ToolContext, p: Point, m: Mods) -> Option<Cursor> {
        let tol = cx.pick_tol();
        if self.takes_point(cx, p) {
            return None;
        }
        // A selected path's anchors or the anchors of the path under the pointer, as Highlight
        // anchors on mouse over finds them: walking every path on each hover is too slow.
        if let Some((id, _)) = hovered_anchor(cx, p) {
            return Some(if cx.selection.contains(id) { Cursor::DirectAnchorSelected } else { Cursor::DirectAnchor });
        }
        if hit_frame_edge(cx, p, tol).is_some() {
            return None;
        }
        if matches!(hit_spine_segment(cx, p, tol), Some((.., false))) {
            return Some(Cursor::ReshapeSegment);
        }
        if m.shift || self.guide.cursor(cx, p).is_some() {
            return None;
        }
        let h = hit_test(cx.doc, p, cx.selection_hit_options())?;
        let whole = cx.selection.contains(h.leaf) && cx.selection.partial(h.leaf).is_none();
        let curved = matches!(h.kind, HitKind::Stroke | HitKind::Outline) && segment_at(cx, h.leaf, p).is_some_and(|s| s.curved);
        (!whole && curved).then_some(Cursor::ReshapeSegment)
    }
}

/// The anchor within a press's reach of `p` ([`ToolContext::point_tol`]), of a selected path or the
/// path under `p`: what Highlight anchors on mouse over marks (a press there picks it).
fn hovered_anchor(cx: &ToolContext, p: Point) -> Option<(NodeId, Point)> {
    let tol = cx.point_tol();
    let under =
        hit_test(cx.doc, p, cx.selection_hit_options()).map(|h| cx.doc.node(h.leaf).and_then(|n| compound_part_at(n, p, tol)).unwrap_or(h.leaf));
    cx.selection.objects.iter().copied().filter(|id| in_scope(cx.doc, cx.isolation, *id)).chain(under).find_map(|id| {
        let path = cx.doc.node(id).and_then(editable_path)?;
        nearest_anchor(&path, p, tol).map(|(.., a)| (id, a))
    })
}

/// The frame anchors of the selected area type, which Direct Selection drags.
fn frame_overlays(cx: &ToolContext) -> Vec<Overlay> {
    let mut out = vec![];
    for id in &cx.selection.objects {
        let Some(frame) = cx.doc.node(*id).filter(|n| is_area_type(n)).and_then(editable_path) else { continue };
        let color = cx.doc.layer_color(*id);
        out.extend(frame.anchors().map(|(_, _, a)| Overlay::Anchor { p: a.p, color, filled: false, size: 5.0 }));
    }
    out
}

impl DirectSelectionTool {
    /// Press on area type's frame anchors (a corner, or an edge's two ends): select the text and
    /// get ready to drag them.
    fn press_type_area(&mut self, cx: &ToolContext, id: NodeId, anchors: Vec<AnchorRef>, p: Point, shift: bool) -> Vec<Action> {
        self.state = State::TypeArea { id, anchors, start: p, began: false };
        if cx.selection.contains(id) {
            vec![]
        } else if shift {
            vec![Action::Exec("select.add".into(), json!({"ids": [id.0]}))]
        } else {
            vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]
        }
    }

    /// The clicked spine point, filled, with its handles.
    fn spine_overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(sel) = self.spine.filter(|(id, _)| cx.selection.contains(*id)) else { return vec![] };
        let Some(a) = spine_anchor(cx, sel) else { return vec![] };
        let color = cx.doc.layer_color(sel.0);
        let mut out = vec![];
        for h in [a.h_in, a.h_out].into_iter().filter(|h| h.distance(a.p) > 1e-6) {
            out.push(Overlay::Line { a: a.p, b: h, color, dashed: false });
            out.push(Overlay::Handle { p: h, color });
        }
        out.push(Overlay::Anchor { p: a.p, color, filled: true, size: 5.0 });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    /// #414: a ruler guide is picked over the art, but an anchor on it is picked first.
    #[test]
    fn direct_and_group_selection_pick_ruler_guides() {
        let (mut d, id) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(true, 100.0));
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let down = |t: &mut DirectSelectionTool, y: f64| t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, y));
        let pick = vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))];
        for group in [false, true] {
            let mut t = DirectSelectionTool::new(group);
            assert_eq!(down(&mut t, 150.0), pick, "on the rect's edge (group: {group})");
            assert!(t.busy());
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, 100.0, 150.0));
            assert_eq!(t.cursor(&c, Point::new(101.0, 300.0), Mods::default()), Cursor::ResizeH);
        }
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(down(&mut t, 100.0), vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))]);
    }

    /// A Cmd/Ctrl+Shift double-click with Direct Selection or Group Selection releases the ruler
    /// guide or the guide path under the pointer; a plain double-click on a ruler guide, or one while
    /// guides are hidden or locked, releases nothing.
    #[test]
    fn a_cmd_shift_double_click_releases_the_guide_under_the_pointer() {
        let (mut d, id) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
        let Some(NodeKind::Path { guide, .. }) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("the rectangle is a path") };
        *guide = true;
        let (s, p) = (Selection::default(), paint());
        let both = Mods { cmd: true, shift: true, ..Mods::default() };
        let index = vec![Action::Exec("view.guides.release".into(), json!({"index": 0}))];
        let ids = vec![Action::Exec("view.guides.release".into(), json!({"ids": [id.0]}))];
        let locked = ToolContext { guides: false, ..cx(&d, &s, &p) };
        for group in [false, true] {
            let dbl = |c: &ToolContext, x: f64, y: f64, m: Mods| {
                DirectSelectionTool::new(group).pointer(c, &PointerEvent::new(PointerKind::DoubleClick, x, y).with_mods(m))
            };
            let c = cx(&d, &s, &p);
            assert_eq!(dbl(&c, 350.0, 301.0, both), index, "on the ruler guide (group: {group})");
            assert_eq!(dbl(&c, 200.0, 150.0, both), ids, "on the guide path (group: {group})");
            assert!(dbl(&c, 350.0, 301.0, Mods::default()).is_empty(), "a plain double-click (group: {group})");
            assert!(dbl(&c, 350.0, 301.0, Mods { cmd: true, ..Mods::default() }).is_empty(), "Cmd/Ctrl alone (group: {group})");
            assert!(dbl(&locked, 350.0, 301.0, both).is_empty(), "guides locked (group: {group})");
            assert!(dbl(&locked, 200.0, 150.0, both).is_empty(), "guides locked (group: {group})");
        }
    }

    /// An anchor is picked from 2 px past its drawn square even where its segment is nearer, and
    /// further with a larger anchor Size or Tolerance (#593); away from it the segment is.
    #[test]
    fn anchors_are_picked_before_the_segments_through_them() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let corner = vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))];
        let press = |cx: &ToolContext, x: f64, y: f64| DirectSelectionTool::new(false).pointer(cx, &PointerEvent::new(PointerKind::Down, x, y));
        let c = cx(&d, &s, &p);
        // 4.3 px from the corner, 0.5 px from the top edge.
        assert_eq!(press(&c, 104.2, 100.5), corner);
        assert_ne!(press(&c, 110.0, 100.5), corner, "the segment, away from the anchor");
        assert_ne!(press(&c, 105.0, 100.5), corner, "past the default reach (4.5 px)");
        let large = ToolContext { anchor_size: 7, ..cx(&d, &s, &p) };
        assert_eq!(press(&large, 106.0, 100.5), corner, "Size 7 draws them 9 px wide");
        let loose = ToolContext { selection_tolerance: 8.0, ..cx(&d, &s, &p) };
        assert_eq!(press(&loose, 107.5, 100.5), corner);
        // On screen: at 200% the reach is half as far in the document.
        let zoomed = ToolContext { zoom: 2.0, ..cx(&d, &s, &p) };
        assert_eq!(press(&zoomed, 102.1, 100.2), corner);
        assert_ne!(press(&zoomed, 102.6, 100.2), corner);
    }

    /// A press on a clip group's clipping path, where the art it clips lies over its edge, takes
    /// the clipping path's segment, so the mask can be reshaped on the canvas; inside the mask the
    /// art is taken, as before.
    #[test]
    fn a_press_on_a_clipping_path_takes_its_segment_over_the_art() {
        let (d, _, clip, art) = doc_with_masked_square();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let press = |x: f64, y: f64| DirectSelectionTool::new(false).pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
        let names = |actions: &[Action], id: NodeId| {
            actions.iter().any(|a| matches!(a, Action::Exec(_, v) if v["id"] == json!(id.0) || v["ids"] == json!([id.0])))
        };
        // The middle of the right edge, on it and just inside it.
        for x in [500.0, 498.5] {
            let a = press(x, 450.0);
            assert!(names(&a, clip), "at {x}: {a:?}");
        }
        let a = press(450.0, 450.0);
        assert!(names(&a, art) && !names(&a, clip), "{a:?}");
    }

    /// A blend of two 20 pt squares centred on (110, 310) and (210, 310), selected.
    fn blend_doc() -> (vectorcraft_doc::Document, NodeId, NodeId) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let key = |id: NodeId, x: f64| {
            let r = vectorcraft_geom::shapes::rectangle(Rect::new(x, 300.0, x + 20.0, 320.0));
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, r, vectorcraft_doc::Appearance::default_art()))
        };
        let (g, k1, k2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let b = vectorcraft_doc::Node::new(g, NodeKind::Blend { children: vec![key(k1, 100.0), key(k2, 200.0)], spec: Default::default() });
        d.insert(Some(l), 1, b).unwrap();
        (d, g, k1)
    }

    /// Highlight anchors on mouse over (#394): the anchor under the pointer, within the selection
    /// tolerance, shows enlarged; none with the preference off or away from anchors.
    #[test]
    fn the_anchor_under_the_pointer_is_highlighted() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let big = |t: &DirectSelectionTool, cx: &ToolContext| {
            t.overlays(cx).into_iter().find_map(|o| match o {
                Overlay::Anchor { p, size, .. } if size > 5.0 => Some(p),
                _ => None,
            })
        };
        let mut t = DirectSelectionTool::new(false);
        let on = cx(&d, &s, &p);
        t.pointer(&on, &PointerEvent::new(PointerKind::Move, 102.0, 101.0));
        assert_eq!(big(&t, &on), Some(Point::new(100.0, 100.0)), "the corner of {id:?}");
        t.pointer(&on, &PointerEvent::new(PointerKind::Move, 150.0, 100.0));
        assert_eq!(big(&t, &on), None, "mid-segment: no anchor");
        let off = ToolContext { highlight_anchors: false, ..cx(&d, &s, &p) };
        t.pointer(&off, &PointerEvent::new(PointerKind::Move, 102.0, 101.0));
        assert_eq!(big(&t, &off), None, "off");
    }

    #[test]
    fn dragging_a_spine_point_moves_it() {
        let (d, g, _) = blend_doc();
        let mut s = Selection::default();
        s.set([g]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 211.0, 310.0)), vec![]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 221.0, 350.0));
        assert_eq!(a[0], Action::Begin("Reshape Spine".into()));
        assert_eq!(a[1], Action::Preview("object.blend.spine.moveAnchor".into(), json!({"id": g.0, "anchor": 1, "x": 220.0, "y": 350.0})));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 221.0, 350.0)), vec![Action::Commit]);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Anchor { filled: true, .. })), "the clicked point shows");
    }

    /// #1111: the Anchor Point tool bends a selected blend's straight spine; Direct Selection
    /// reshapes a curved spine segment and leaves a straight one alone, as it does a path's.
    #[test]
    fn spine_segments_bend_with_the_anchor_point_tool_and_direct_selection() {
        let (mut d, g, _) = blend_doc();
        let mut s = Selection::default();
        s.set([g]);
        let p = paint();
        let drag = |t: &mut dyn crate::Tool, cx: &ToolContext| {
            t.pointer(cx, &PointerEvent::new(PointerKind::Down, 160.0, 311.0));
            let a = t.pointer(cx, &PointerEvent::new(PointerKind::Drag, 160.0, 351.0));
            t.pointer(cx, &PointerEvent::new(PointerKind::Up, 160.0, 351.0));
            a
        };
        let straight = cx(&d, &s, &p);
        let a = drag(crate::create("anchorPoint").as_mut(), &straight);
        assert!(
            matches!(a.last(), Some(Action::Preview(c, v)) if c == "object.blend.spine.reshapeSegment" && v["id"] == g.0 && v["segment"] == 0 && v["dy"] == 40.0),
            "{a:?}"
        );
        let a = drag(&mut DirectSelectionTool::new(false), &straight);
        assert!(!a.iter().any(|x| matches!(x, Action::Preview(c, _) if c == "object.blend.spine.reshapeSegment")), "straight: {a:?}");
        // The spine bent upward: Direct Selection takes it from where it passes.
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(210.0, 310.0)], false);
        sp.bend_segment(0, 0.5, vectorcraft_geom::Vec2::new(0.0, 20.0));
        let Some(NodeKind::Blend { spec, .. }) = d.node_mut(g).map(|n| &mut n.kind) else { panic!("blend") };
        (spec.spine, spec.key_anchors) = (Some(PathData::single(sp)), vec![0, 1]);
        let mut t = DirectSelectionTool::new(false);
        let curved = cx(&d, &s, &p);
        t.pointer(&curved, &PointerEvent::new(PointerKind::Down, 160.0, 330.0));
        let a = t.pointer(&curved, &PointerEvent::new(PointerKind::Drag, 160.0, 345.0));
        assert_eq!(a[0], Action::Begin("Reshape Spine".into()));
        assert!(
            matches!(&a[1], Action::Preview(c, v) if c == "object.blend.spine.reshapeSegment" && v["segment"] == 0 && v["dx"] == 0.0 && v["dy"] == 15.0),
            "{a:?}"
        );
        assert_eq!(t.pointer(&curved, &PointerEvent::new(PointerKind::Up, 160.0, 345.0)), vec![Action::Commit]);
    }

    /// Shift held while a curved segment is dragged sends `perpendicular`. The test drags a path's
    /// segment with Direct Selection, and a blend spine's segment with Direct Selection and with the
    /// Anchor Point tool.
    #[test]
    fn shift_reshapes_a_segment_perpendicular() {
        let shift = Mods { shift: true, ..Mods::default() };
        let p = paint();
        let drag = |t: &mut dyn crate::Tool, cx: &ToolContext, from: (f64, f64), mods: Mods| {
            t.pointer(cx, &PointerEvent::new(PointerKind::Down, from.0, from.1));
            let a = t.pointer(cx, &PointerEvent::new(PointerKind::Drag, from.0 + 10.0, from.1 - 20.0).with_mods(mods));
            t.pointer(cx, &PointerEvent::new(PointerKind::Up, from.0 + 10.0, from.1 - 20.0).with_mods(mods));
            a
        };
        let sent = |a: &[Action], cmd: &str| match a.last() {
            Some(Action::Preview(c, v)) if c == cmd => v["perpendicular"].as_bool(),
            _ => None,
        };
        let (d, _) = arch_doc();
        let s = Selection::default();
        let arch = cx(&d, &s, &p);
        let (mut b, g, _) = blend_doc();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(210.0, 310.0)], false);
        sp.bend_segment(0, 0.5, vectorcraft_geom::Vec2::new(0.0, 20.0));
        let Some(NodeKind::Blend { spec, .. }) = b.node_mut(g).map(|n| &mut n.kind) else { panic!("blend") };
        (spec.spine, spec.key_anchors) = (Some(PathData::single(sp)), vec![0, 1]);
        let mut bs = Selection::default();
        bs.set([g]);
        let spine = cx(&b, &bs, &p);
        for (mods, perpendicular) in [(Mods::default(), false), (shift, true)] {
            let a = drag(&mut DirectSelectionTool::new(false), &arch, (150.0, 262.5), mods);
            assert_eq!(sent(&a, "path.reshapeSegment"), Some(perpendicular), "{a:?}");
            let a = drag(&mut DirectSelectionTool::new(false), &spine, (160.0, 330.0), mods);
            assert_eq!(sent(&a, "object.blend.spine.reshapeSegment"), Some(perpendicular), "{a:?}");
            let a = drag(crate::create("anchorPoint").as_mut(), &spine, (160.0, 330.0), mods);
            assert_eq!(sent(&a, "object.blend.spine.reshapeSegment"), Some(perpendicular), "{a:?}");
        }
    }

    #[test]
    fn direct_and_group_selection_pick_blend_keys() {
        let (d, g, k1) = blend_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let a = DirectSelectionTool::new(false).pointer(&cx, &PointerEvent::new(PointerKind::Down, 104.0, 304.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [k1.0]}))]);
        let a = DirectSelectionTool::new(true).pointer(&cx, &PointerEvent::new(PointerKind::Down, 104.0, 304.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [k1.0]}))]);
        // A click on a step (between the keys) still selects the blend.
        let a = DirectSelectionTool::new(true).pointer(&cx, &PointerEvent::new(PointerKind::Down, 160.0, 304.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [g.0]}))]);
    }

    #[test]
    fn dragging_a_frame_corner_or_edge_reshapes_area_type() {
        let (d, text) = doc_with_area_type();
        let p = paint();
        let s = Selection::default();
        let cx1 = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        // The bottom-right corner of the 120 × 40 frame at (300, 300) selects the text...
        assert_eq!(
            t.pointer(&cx1, &PointerEvent::new(PointerKind::Down, 421.0, 339.0)),
            vec![Action::Exec("select.set".into(), json!({"ids": [text.0]}))]
        );
        // ...and drags that corner.
        let a = t.pointer(&cx1, &PointerEvent::new(PointerKind::Drag, 461.0, 399.0));
        assert_eq!(a[0], Action::Begin("Reshape Type Area".into()));
        assert_eq!(a[1], Action::Preview("text.reshapeArea".into(), json!({"id": text.0, "anchors": [[0, 2]], "dx": 40.0, "dy": 60.0})));
        assert_eq!(t.pointer(&cx1, &PointerEvent::new(PointerKind::Up, 461.0, 399.0)), vec![Action::Commit]);
        // The top edge drags both its ends; a click alone changes nothing.
        let mut s = Selection::default();
        s.add(text);
        let cx2 = cx(&d, &s, &p);
        assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 350.0, 301.0)).is_empty());
        let a = t.pointer(&cx2, &PointerEvent::new(PointerKind::Drag, 350.0, 281.0));
        assert_eq!(a[1], Action::Preview("text.reshapeArea".into(), json!({"id": text.0, "anchors": [[0, 0], [0, 1]], "dx": 0.0, "dy": -20.0})));
        t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 350.0, 281.0));
        assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 300.0, 340.0)).is_empty());
        assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 300.0, 340.0)).is_empty());
        // The selected frame's corners show.
        assert_eq!(t.overlays(&cx2).iter().filter(|o| matches!(o, Overlay::Anchor { filled: false, .. })).count(), 4);
    }

    #[test]
    fn click_anchor_selects_it() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))]);
    }

    /// Pressing an anchor of a path whose anchors are all selected (stored as the whole path)
    /// keeps them all selected, so the drag moves them all; Shift still deselects that anchor.
    #[test]
    fn pressing_an_anchor_of_a_fully_selected_path_keeps_the_selection() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0));
        assert!(matches!(a.as_slice(), [Action::Begin(_), Action::Preview(c, _)] if c == "object.transform"), "{a:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 100.0));
        let shift = Mods { shift: true, ..Default::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "toggle"}))]);
    }

    /// A click (no drag) on an anchor that is selected with others, whether the whole path is or
    /// some of its anchors, selects that anchor alone, as in Illustrator: a press keeps the
    /// selection so a drag moves it all, the release without a drag narrows it. Shift still
    /// toggles, and a click on a path's only selected anchor changes nothing visible.
    #[test]
    fn a_click_on_a_selected_anchor_selects_it_alone() {
        let (d, id) = doc_with_rect();
        let p = paint();
        let alone = vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))];
        let mut whole = Selection::default();
        whole.set([id]);
        for s in [whole, anchors_of(id, &[(0, 0), (0, 1)])] {
            let cx = cx(&d, &s, &p);
            let mut t = DirectSelectionTool::new(false);
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)).is_empty(), "{s:?}");
            assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)), alone, "{s:?}");
            // Dragged instead: moved, selection kept.
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)).is_empty());
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0));
            assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 100.0)), vec![Action::Commit], "{s:?}");
        }
        // An anchor not yet selected is selected on the press, and the release adds nothing.
        let none = Selection::default();
        let cx = cx(&d, &none, &p);
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)), alone);
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)).is_empty());
    }

    /// Alt-pressing an anchor picks the whole path, as Alt on a segment does, and the drag copies
    /// it as the Selection tool's Alt-drag does.
    #[test]
    fn alt_dragging_an_anchor_copies_the_whole_path() {
        let (d, id) = doc_with_rect();
        let alt = Mods { alt: true, ..Default::default() };
        let mut whole = Selection::default();
        whole.set([id]);
        let p = paint();
        for (s, pick) in [(Selection::default(), true), (anchors_of(id, &[(0, 0)]), true), (whole, false)] {
            let cx = cx(&d, &s, &p);
            let mut t = DirectSelectionTool::new(false);
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(alt));
            let set = vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))];
            assert_eq!(a, if pick { set } else { vec![] }, "{s:?}");
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0).with_mods(alt));
            assert!(matches!(a.as_slice(), [Action::Begin(_), Action::Preview(c, v)] if c == "object.transform" && v["copy"] == true), "{a:?}");
        }
    }

    #[test]
    fn marquee_selects_anchors() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 150.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "set"}))]);
    }

    #[test]
    fn marquee_selects_ruler_guides() {
        let (mut d, id) = doc_with_rect();
        d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 150.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        assert_eq!(
            a,
            vec![
                Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "set"})),
                Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true})),
            ]
        );
    }

    /// #2441: Group Selection picks one path of a compound path (the outer shape or a hole);
    /// a second click on it adds the compound path, like a group around a member.
    #[test]
    fn group_selection_picks_a_path_of_a_compound_path() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (outer, hole, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let rect = |id, a: f64, b: f64| {
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()))
        };
        let mut c = vectorcraft_doc::Node::new(
            compound,
            vectorcraft_doc::NodeKind::Compound {
                children: vec![rect(outer, 0.0, 200.0), rect(hole, 50.0, 150.0)],
                rule: vectorcraft_geom::FillRule::EvenOdd,
            },
        );
        c.appearance = vectorcraft_doc::Appearance::default_art().into();
        d.insert(Some(l), 0, c).unwrap();
        let p = paint();
        let none = Selection::default();
        let click = |s: &Selection, x, y| DirectSelectionTool::new(true).pointer(&cx(&d, s, &p), &PointerEvent::new(PointerKind::Down, x, y));
        let set = |id: NodeId| vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))];
        assert_eq!(click(&none, 25.0, 100.0), set(outer), "the fill between the outline and the hole");
        assert_eq!(click(&none, 50.0, 100.0), set(hole), "the hole's outline");
        assert!(click(&none, 100.0, 100.0).is_empty(), "inside the hole nothing is hit");
        let mut s = Selection::default();
        s.set([outer]);
        assert_eq!(click(&s, 25.0, 100.0), vec![Action::Exec("select.add".into(), json!({"ids": [compound.0]}))]);
    }

    /// Group Selection Shift-click on a selected compound-path member deselects it on release.
    #[test]
    fn group_selection_shift_click_deselects_a_compound_path_member() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (outer, hole, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let rect = |id, a: f64, b: f64| {
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()))
        };
        let mut c = vectorcraft_doc::Node::new(
            compound,
            vectorcraft_doc::NodeKind::Compound {
                children: vec![rect(outer, 0.0, 200.0), rect(hole, 50.0, 150.0)],
                rule: vectorcraft_geom::FillRule::EvenOdd,
            },
        );
        c.appearance = vectorcraft_doc::Appearance::default_art().into();
        d.insert(Some(l), 0, c).unwrap();
        let p = paint();
        let mut s = Selection::default();
        s.set([outer]);
        let shift = Mods { shift: true, ..Mods::default() };
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(true);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 25.0, 100.0).with_mods(shift));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 25.0, 100.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("select.toggle".into(), json!({"id": outer.0}))]);
    }

    /// A path inside an inner group, a sibling path outside it, and the groups' ids.
    /// The inside path is the rectangle at (100, 100); the outside one at (300, 300).
    fn nested_group_doc() -> (vectorcraft_doc::Document, NodeId, NodeId, NodeId, NodeId) {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (inside, outside, inner, outer) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
        let path = |id, x| {
            std::sync::Arc::new(vectorcraft_doc::Node::path(
                id,
                vectorcraft_geom::shapes::rectangle(Rect::new(x, x, x + 100.0, x + 100.0)),
                vectorcraft_doc::Appearance::default_art(),
            ))
        };
        let inner_g = vectorcraft_doc::Node::group(inner, vec![path(inside, 100.0)]);
        let outer_g = vectorcraft_doc::Node::group(outer, vec![std::sync::Arc::new(inner_g), path(outside, 300.0)]);
        d.insert(Some(l), 0, outer_g).unwrap();
        (d, inside, outside, inner, outer)
    }

    /// Isolation mode: a marquee and an anchor click take the path inside the isolated group and
    /// leave the path beside it. Group Selection stops at the isolated group instead of adding
    /// the group around it.
    #[test]
    fn isolation_mode_leaves_art_outside_the_isolated_group() {
        let (d, inside, outside, inner, outer) = nested_group_doc();
        let p = paint();
        let none = Selection::default();
        let isolated = ToolContext { isolation: Some(inner), ..cx(&d, &none, &p) };
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&isolated, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
        let a = t.pointer(&isolated, &PointerEvent::new(PointerKind::Up, 450.0, 450.0));
        let items = a.first().and_then(|act| match act {
            Action::Exec(c, v) if c == "select.anchorsMany" => v.get("items").cloned(),
            _ => None,
        });
        let items = items.unwrap();
        assert!(items.as_array().is_some_and(|a| a.iter().all(|item| item["id"] == inside.0)), "marquee: {a:?}");
        assert!(items.as_array().is_some_and(|a| !a.is_empty()), "the path inside is still selected: {a:?}");

        let a = DirectSelectionTool::new(false).pointer(&isolated, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        assert!(a.is_empty(), "a click on {outside:?}, outside isolation, starts a marquee: {a:?}");
        let a = DirectSelectionTool::new(false).pointer(&isolated, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": inside.0, "anchors": [[0, 0]], "mode": "set"}))]);

        let mut s = Selection::default();
        s.set([inside, inner]);
        let climbed = ToolContext { isolation: Some(inner), ..cx(&d, &s, &p) };
        let a = DirectSelectionTool::new(true).pointer(&climbed, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert!(a.is_empty(), "does not add the group outside isolation ({outer:?}): {a:?}");
    }

    /// #927: a marquee across type selects the type object whole, beside the anchors it holds.
    #[test]
    fn marquee_selects_type_it_touches() {
        // The rectangle at (100, 100)–(200, 200) and area type at (300, 300).
        let (d, text) = doc_with_area_type();
        let rect = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        // From empty canvas across the type to inside the rectangle (its bottom-right corner).
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 450.0, 450.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 150.0));
        let items = json!([{"id": rect.0, "anchors": [[0, 2]]}, {"id": text.0, "anchors": []}]);
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": "set"}))]);
        // Away from the type, only anchors.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": rect.0, "anchors": [[0, 1]]}], "mode": "set"}))]);
    }

    /// A marquee across a member of an Intertwine group selects the member's anchors, as in a
    /// plain group, and not the Intertwine group whole.
    #[test]
    fn marquee_selects_the_anchors_of_intertwine_members() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (a, b, g) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let path = |id, x| {
            std::sync::Arc::new(vectorcraft_doc::Node::path(
                id,
                vectorcraft_geom::shapes::rectangle(Rect::new(x, 100.0, x + 100.0, 200.0)),
                vectorcraft_doc::Appearance::default_art(),
            ))
        };
        let intertwine = vectorcraft_doc::Node::new(g, vectorcraft_doc::NodeKind::Intertwine { children: vec![path(a, 100.0), path(b, 300.0)] });
        d.insert(Some(l), 0, intertwine).unwrap();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        for group in [false, true] {
            let mut t = DirectSelectionTool::new(group);
            // Around the first member's top-right corner.
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
            let actions = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
            let items = json!([{"id": a.0, "anchors": [[0, 1]]}]);
            assert_eq!(actions, vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": "set"}))], "group: {group}");
        }
    }

    /// Shift-drag a marquee: the anchors inside toggle (#483), with Group Selection too.
    #[test]
    fn shift_marquee_toggles_anchors() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        for group in [false, true] {
            let mut t = DirectSelectionTool::new(group);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0).with_mods(shift));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 150.0).with_mods(shift));
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0).with_mods(shift));
            assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "toggle"}))]);
        }
    }

    /// Group Selection: E pressed during a marquee drag makes it select the objects entirely inside
    /// it whole, a group's member without its group. Direct Selection does not take E.
    #[test]
    fn e_during_a_group_selection_marquee_selects_the_objects_inside() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let rect = |id, r| Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art());
        let (g, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        d.insert(Some(l), 1, Node::group(g, vec![])).unwrap();
        d.insert(Some(g), 0, rect(a, Rect::new(100.0, 220.0, 120.0, 240.0))).unwrap();
        d.insert(Some(g), 1, rect(b, Rect::new(100.0, 300.0, 120.0, 320.0))).unwrap();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let e = ToolKey::Letter('e');
        let drag = |group: bool, mods: Mods| {
            let mut t = DirectSelectionTool::new(group);
            assert!(!t.claims_key(&cx, e), "no marquee yet");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0).with_mods(mods));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 250.0).with_mods(mods));
            assert_eq!(t.claims_key(&cx, e), group);
            t.key(&cx, e, Mods::default());
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 250.0).with_mods(mods))
        };
        assert_eq!(drag(true, Mods::default()), vec![Action::Exec("select.set".into(), json!({"ids": [id.0, a.0]}))]);
        let shift = Mods { shift: true, ..Default::default() };
        assert_eq!(drag(true, shift), vec![Action::Exec("select.toggle".into(), json!({"ids": [id.0, a.0]}))]);
        // Direct Selection's marquee still selects the anchors inside it.
        let anchors = |id: NodeId, list: Value| json!({"id": id.0, "anchors": list});
        let all = json!([[0, 0], [0, 1], [0, 2], [0, 3]]);
        assert_eq!(
            drag(false, Mods::default()),
            vec![Action::Exec("select.anchorsMany".into(), json!({"items": [anchors(id, all.clone()), anchors(a, all)], "mode": "set"}))]
        );
    }

    /// An open path from (100, 300) to (200, 300) arching up through (150, 262.5).
    fn arch_doc() -> (vectorcraft_doc::Document, NodeId) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
        sp.anchors[0].h_out = Point::new(100.0, 250.0);
        sp.anchors[1].h_in = Point::new(200.0, 250.0);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, PathData::single(sp), vectorcraft_doc::Appearance::default_art())).unwrap();
        (d, id)
    }

    /// #1743: Escape mid-drag cancels it; the release after it does nothing more.
    #[test]
    fn escape_cancels_a_drag() {
        let (d, _) = arch_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 240.0));
        assert!(t.busy());
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy());
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 220.0)), vec![]);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 220.0)), vec![]);
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![], "nothing to cancel");
    }

    #[test]
    fn dragging_a_curved_segment_reshapes_it_and_a_straight_one_moves_its_anchors() {
        let (d, arch) = arch_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        // The arch: its segment gets selected, and the drag bends it.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
        assert_eq!(a, vec![Action::Exec("select.segments".into(), json!({"id": arch.0, "segments": [[0, 0]]}))]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 242.5));
        assert_eq!(a[0], Action::Begin("Reshape".into()));
        let Action::Preview(cmd, v) = &a[1] else { panic!("{a:?}") };
        assert_eq!(
            (cmd.as_str(), v["id"].clone(), v["segment"].clone(), v["dx"].clone(), v["dy"].clone()),
            ("path.reshapeSegment", json!(arch.0), json!(0), json!(0.0), json!(-20.0))
        );
        assert!((v["t"].as_f64().unwrap() - 0.5).abs() < 0.01, "{v}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 242.5)), vec![Action::Commit]);
        // A straight edge of the rectangle: its two anchors get selected and move.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0));
        assert!(matches!(&a[..], [Action::Exec(c, v)] if c == "select.segments" && v["segments"] == json!([[0, 0]])), "{a:?}");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 81.0));
        assert_eq!(a[1], Action::Preview("path.moveAnchors".into(), json!({"dx": 0.0, "dy": -20.0})));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 81.0));
        // Alt on the stroke picks the whole path, and Alt-drag copies it.
        let alt = Mods { alt: true, ..Default::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0).with_mods(alt));
        assert!(matches!(&a[..], [Action::Exec(c, _)] if c == "select.set"), "{a:?}");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 81.0).with_mods(alt));
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, v)] if c == "object.transform" && v["copy"] == true), "{a:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 81.0).with_mods(alt));
        // The fill still selects the whole path, and a path selected as a whole still moves whole.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert!(matches!(&a[..], [Action::Exec(c, _)] if c == "select.set"), "{a:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 150.0));
        let rect = d.layers[0].children().unwrap()[0].id;
        let mut whole = Selection::default();
        whole.set([rect]);
        let cx = crate::testutil::cx(&d, &whole, &p);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0)), vec![]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 81.0));
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "object.transform"), "{a:?}");
    }

    /// #494: the handles shown are the handles dragged. A path selected as a whole (the one the
    /// Pen just drew) shows all its handles, and they drag; the direct-selected anchors' handles
    /// only otherwise; none with Show handles when multiple anchors are selected off and several
    /// anchors selected. A handle shorter than the tolerance leaves its anchor to be picked, and
    /// the tolerance is the preference's, in screen pixels at any zoom.
    #[test]
    fn the_handles_shown_are_the_handles_dragged() {
        let (d, arch) = arch_doc();
        let p = paint();
        // Does a press at (x, y) take a handle?
        let handle = |c: &ToolContext, x: f64, y: f64| {
            DirectSelectionTool::new(false).pointer(c, &PointerEvent::new(PointerKind::Down, x, y)) == vec![Action::Begin("Reshape".into())]
        };
        let mut whole = Selection::default();
        whole.set([arch]);
        let c = cx(&d, &whole, &p);
        assert!(handle(&c, 101.0, 251.0), "the first anchor's handle, the path selected whole");
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 101.0, 251.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 120.0, 240.0));
        assert!(matches!(&a[..], [Action::Preview(c, v)] if c == "path.setHandle" && v["anchor"] == 0 && v["which"] == "out"), "{a:?}");
        // Only the second anchor direct-selected: the first one's handle is hidden.
        let second = anchors_of(arch, &[(0, 1)]);
        assert!(!handle(&cx(&d, &second, &p), 101.0, 251.0));
        assert!(handle(&cx(&d, &second, &p), 201.0, 251.0));
        // Show handles when multiple anchors are selected off: the whole path's are hidden.
        assert!(!handle(&ToolContext { handles_multiple: false, ..cx(&d, &whole, &p) }, 101.0, 251.0));
        assert!(handle(&ToolContext { handles_multiple: false, ..cx(&d, &second, &p) }, 201.0, 251.0));
        // The tolerance is the preference's, in screen pixels: 3 px is 0.75 pt at 400%.
        let zoomed = ToolContext { zoom: 4.0, ..cx(&d, &whole, &p) };
        assert!(!handle(&zoomed, 101.0, 251.0));
        assert!(handle(&zoomed, 100.5, 250.5));
        assert!(handle(&ToolContext { selection_tolerance: 8.0, ..cx(&d, &whole, &p) }, 106.0, 254.0));
    }

    /// #494: a handle 2 pt long, inside the tolerance round its anchor, doesn't hide the anchor.
    #[test]
    fn a_short_handle_leaves_its_anchor_to_be_picked() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
        sp.anchors[0].h_out = Point::new(102.0, 300.0);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, PathData::single(sp), vectorcraft_doc::Appearance::default_art())).unwrap();
        let s = anchors_of(id, &[(0, 0)]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 300.0)), vec![], "the anchor, already selected");
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 90.0, 290.0));
        assert_eq!(a[0], Action::Begin("Move".into()));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 90.0, 290.0));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 102.5, 300.0)), vec![Action::Begin("Reshape".into())], "the handle");
    }

    /// Direct Selection's cursors from Illustrator's manual: the white arrow at rest; over an anchor
    /// a square, hollow on an unselected path and filled on a selected one; the Reshape Segment
    /// cursor over a curved segment a drag reshapes, not over a straight one (which moves) or a
    /// path selected whole (which moves whole); Group Selection's plus with Alt and as its tool.
    #[test]
    fn hovering_shows_the_anchor_square_reshape_and_group_cursors() {
        let (d, arch) = arch_doc();
        let p = paint();
        let m = Mods::default();
        let pt = Point::new;
        let none = Selection::default();
        let c = cx(&d, &none, &p);
        let t = DirectSelectionTool::new(false);
        assert_eq!(t.cursor(&c, pt(20.0, 20.0), m), Cursor::ArrowHollow);
        assert_eq!(t.cursor(&c, pt(100.0, 300.0), m), Cursor::DirectAnchor);
        assert_eq!(t.cursor(&c, pt(150.0, 262.5), m), Cursor::ReshapeSegment);
        assert_eq!(t.cursor(&c, pt(150.0, 100.0), m), Cursor::ArrowHollow, "a straight segment moves");
        assert_eq!(t.cursor(&c, pt(150.0, 150.0), m), Cursor::ArrowHollow, "the fill");
        let alt = Mods { alt: true, ..m };
        assert_eq!(t.cursor(&c, pt(100.0, 300.0), alt), Cursor::GroupSelection);
        assert_eq!(DirectSelectionTool::new(true).cursor(&c, pt(100.0, 300.0), m), Cursor::GroupSelection);
        let mut whole = Selection::default();
        whole.set([arch]);
        let c = cx(&d, &whole, &p);
        assert_eq!(t.cursor(&c, pt(200.0, 300.0), m), Cursor::DirectAnchorSelected);
        assert_eq!(t.cursor(&c, pt(150.0, 262.5), m), Cursor::ArrowHollow, "a path selected whole moves whole");
        // One anchor selected: its handle end shows and keeps the plain arrow, Alt or not (a press
        // takes the handle).
        let s = anchors_of(arch, &[(0, 0)]);
        let c = cx(&d, &s, &p);
        assert_eq!(t.cursor(&c, pt(100.0, 250.0), m), Cursor::ArrowHollow, "a handle end");
        assert_eq!(t.cursor(&c, pt(100.0, 250.0), alt), Cursor::ArrowHollow, "a handle end with Alt");
        assert_eq!(t.cursor(&c, pt(200.0, 300.0), m), Cursor::DirectAnchorSelected);
        // Bending the curve keeps the Reshape Segment cursor.
        let mut t = DirectSelectionTool::new(false);
        let c = cx(&d, &none, &p);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 230.0));
        assert_eq!(t.cursor(&c, pt(150.0, 230.0), m), Cursor::ReshapeSegment);
    }

    /// A drag shows the filled arrow, hollow while it snaps to a point; an Alt-drag copy shows the
    /// double arrowhead.
    #[test]
    fn drags_show_the_filled_arrow_hollow_on_a_snap_and_alt_the_copy_cursor() {
        let (d, _, b) = two_squares();
        let p = paint();
        let none = Mods::default();
        let at = |kind, x, y, m: Mods| PointerEvent::new(kind, x, y).with_mods(m);
        let one = anchors_of(b, &[(0, 0)]);
        let c = cx(&d, &one, &p);
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&c, &at(PointerKind::Down, 300.0, 300.0, none));
        t.pointer(&c, &at(PointerKind::Drag, 250.0, 260.0, none));
        assert_eq!(t.cursor(&c, Point::new(250.0, 260.0), none), Cursor::Arrow);
        // Onto A's bottom-right corner.
        t.pointer(&c, &at(PointerKind::Drag, 203.0, 199.0, none));
        assert_eq!(t.cursor(&c, Point::new(203.0, 199.0), none), Cursor::ArrowHollow);
        t.pointer(&c, &at(PointerKind::Up, 203.0, 199.0, none));
        // Shift with a Constrain Angle of 10°: the constraint alone isn't a snap.
        let shift = Mods { shift: true, ..none };
        let tilted = ToolContext { constrain_angle: 10.0, ..cx(&d, &one, &p) };
        t.pointer(&tilted, &at(PointerKind::Down, 300.0, 300.0, shift));
        t.pointer(&tilted, &at(PointerKind::Drag, 380.0, 262.0, shift));
        assert_eq!(t.cursor(&tilted, Point::new(380.0, 262.0), shift), Cursor::Arrow);
        t.pointer(&tilted, &at(PointerKind::Up, 380.0, 262.0, shift));
        let mut whole = Selection::default();
        whole.set([b]);
        let c = cx(&d, &whole, &p);
        let alt = Mods { alt: true, ..none };
        t.pointer(&c, &at(PointerKind::Down, 300.0, 300.0, alt));
        t.pointer(&c, &at(PointerKind::Drag, 250.0, 260.0, alt));
        assert_eq!(t.cursor(&c, Point::new(250.0, 260.0), alt), Cursor::SelectionCopy);
    }

    /// [`doc_with_rect`] (A, 100..200) and a second square B, 300..400: (A, B).
    fn two_squares() -> (vectorcraft_doc::Document, NodeId, NodeId) {
        let (mut d, a) = doc_with_rect();
        let l = d.layers[0].id;
        let b = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        (d, a, b)
    }

    /// An anchor of a selected path is picked before the one of an unselected path on a layer
    /// above that covers it, as Highlight anchors on mouse over already marks it (#1508); with
    /// nothing selected, the topmost path's anchor is picked.
    #[test]
    fn a_selected_path_s_anchor_is_picked_before_one_above_it() {
        let (mut d, a) = doc_with_rect();
        let top = d.add_layer(None);
        let b = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(101.0, 100.0, 201.0, 200.0));
        d.insert(Some(top), 0, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        let p = paint();
        let pick = |id: NodeId| vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))];
        let s = anchors_of(a, &[(0, 2)]);
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(t.pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 101.0, 100.0)), pick(a));
        let mut whole = Selection::default();
        whole.set([a]);
        let mut t = DirectSelectionTool::new(false);
        let cx2 = cx(&d, &whole, &p);
        assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 101.0, 100.0)).is_empty());
        assert_eq!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 101.0, 100.0)), pick(a));
        let none = Selection::default();
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(t.pointer(&cx(&d, &none, &p), &PointerEvent::new(PointerKind::Down, 101.0, 100.0)), pick(b));
    }

    /// The anchors `anchors` of path `id` direct-selected.
    fn anchors_of(id: NodeId, anchors: &[AnchorRef]) -> Selection {
        let mut s = Selection::default();
        s.set([id]);
        s.anchors.insert(id, anchors.iter().copied().collect());
        s
    }

    /// Press at `from`, drag to `to`: the move the drag previews, and the smart guide labels shown.
    fn drag(t: &mut DirectSelectionTool, cx: &ToolContext, from: (f64, f64), to: (f64, f64), mods: Mods) -> (Value, Vec<String>) {
        t.pointer(cx, &PointerEvent::new(PointerKind::Down, from.0, from.1).with_mods(mods));
        let a = t.pointer(cx, &PointerEvent::new(PointerKind::Drag, to.0, to.1).with_mods(mods));
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        let labels = t
            .overlays(cx)
            .into_iter()
            .filter_map(|o| match o {
                Overlay::Label { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(t.pointer(cx, &PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(mods)), vec![Action::Commit]);
        assert!(t.overlays(cx).iter().all(|o| !matches!(o, Overlay::Label { .. })), "the guides go with the drag");
        (v.clone(), labels)
    }

    /// Alt-dragging a path by an anchor copies it, and that anchor snaps as a dragged anchor does:
    /// onto another path's anchor, or onto an anchor of the original the copy leaves behind.
    #[test]
    fn alt_dragged_copies_snap_by_the_anchor_pressed() {
        let (d, _, b) = two_squares();
        let p = paint();
        let alt = Mods { alt: true, ..Default::default() };
        let mut whole = Selection::default();
        whole.set([b]);
        let c = cx(&d, &whole, &p);
        let mut t = DirectSelectionTool::new(false);
        let copy =
            |dx: f64, dy: f64| json!({"matrix": matrix_json(vectorcraft_geom::Affine::translate(vectorcraft_geom::Vec2::new(dx, dy))), "copy": true});
        // B's top-left corner, pressed 1 pt off, onto A's bottom-right one.
        let (v, labels) = drag(&mut t, &c, (301.0, 301.0), (203.0, 199.0), alt);
        assert_eq!((v, labels), (copy(-100.0, -100.0), vec!["anchor".to_string()]));
        // B's top-left corner onto the original B's top-right one: the copy sits beside it.
        let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (402.0, 301.0), alt);
        assert_eq!((v, labels), (copy(100.0, 0.0), vec!["anchor".to_string()]));
        // The history says what the drag did.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0).with_mods(alt));
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 250.0).with_mods(alt));
        assert_eq!(out.first(), Some(&Action::Begin("Copy".into())));
    }

    /// Dragging a fully selected path by an anchor moves it whole, so Alt pressed or released
    /// during the drag turns the copy on and off, as with the Selection tool; the anchor pressed
    /// snaps all along, onto the original only while it is left behind.
    #[test]
    fn alt_during_a_drag_by_an_anchor_toggles_the_copy() {
        let (d, _, b) = two_squares();
        let p = paint();
        let (none, alt) = (Mods::default(), Mods { alt: true, ..Default::default() });
        let mut whole = Selection::default();
        whole.set([b]);
        let c = cx(&d, &whole, &p);
        let mut t = DirectSelectionTool::new(false);
        let preview = |dx: f64, dy: f64, copy: bool| {
            Action::Preview(
                "object.transform".into(),
                json!({"matrix": matrix_json(vectorcraft_geom::Affine::translate(vectorcraft_geom::Vec2::new(dx, dy))), "copy": copy}),
            )
        };
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0)).is_empty());
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 250.0)),
            vec![Action::Begin("Move".into()), preview(-50.0, -50.0, false)]
        );
        // The tool sees the document as previewed: B moved along, not where the copy leaves it.
        let (mut moved, _, b2) = two_squares();
        let l = moved.layers[0].id;
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(250.0, 250.0, 350.0, 350.0));
        moved.remove(b2).unwrap();
        moved.insert(Some(l), 1, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        let c = cx(&moved, &whole, &p);
        // Alt down mid-drag: a copy, whose pressed corner can land on the original's top-right one.
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 402.0, 301.0).with_mods(alt)), vec![preview(100.0, 0.0, true)]);
        // Alt up: a move again, which leaves nothing behind to snap to.
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 402.0, 301.0).with_mods(none)), vec![preview(102.0, 1.0, false)]);
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 402.0, 301.0)), vec![Action::Commit]);
        // With only some anchors selected, the drag still moves just those.
        let one = anchors_of(b, &[(0, 0)]);
        let c = cx(&d, &one, &p);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 260.0).with_mods(alt));
        assert!(matches!(a.last(), Some(Action::Preview(c, _)) if c == "path.moveAnchors"), "{a:?}");
    }

    /// A dragged anchor lands on another path's anchor or segment, lines up with the other anchors
    /// of its own path, and with several selected the one pressed on snaps, the others following.
    #[test]
    fn dragged_anchors_snap_to_points_segments_and_smart_guides() {
        let (d, _, b) = two_squares();
        let p = paint();
        let none = Mods::default();
        let one = anchors_of(b, &[(0, 0)]);
        let c = cx(&d, &one, &p);
        let mut t = DirectSelectionTool::new(false);
        // B's top-left corner, pressed 1 pt off, onto A's bottom-right one.
        let (v, labels) = drag(&mut t, &c, (301.0, 301.0), (203.0, 199.0), none);
        assert_eq!((v, labels), (json!({"dx": -100.0, "dy": -100.0}), vec!["anchor".to_string()]));
        // Onto A's right side.
        let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (203.0, 150.0), none);
        assert_eq!((v, labels), (json!({"dx": -100.0, "dy": -150.0}), vec!["path".to_string()]));
        // In line with B's own bottom-left corner, which stays put.
        let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (303.0, 330.0), none);
        assert_eq!((v, labels), (json!({"dx": 0.0, "dy": 30.0}), vec!["align".to_string()]));
        // Shift keeps the move at 45° steps and still snaps along that line (#886): on the
        // diagonal from B's corner, onto A's bottom-right corner…
        let shift = Mods { shift: true, ..none };
        let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (203.0, 199.0), shift);
        let near = |v: &serde_json::Value, (x, y): (f64, f64)| (v["dx"].as_f64().unwrap() - x).hypot(v["dy"].as_f64().unwrap() - y) < 1e-9;
        assert!(near(&v, (-100.0, -100.0)) && labels == ["anchor"], "{v} {labels:?}");
        // …and with nothing in reach, at the pointer's distance along its angle.
        let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (380.0, 302.0), shift);
        let s = move_delta(Point::new(300.0, 300.0), Point::new(380.0, 302.0), true, 0.0);
        assert!(near(&v, (s.x, s.y)) && labels.is_empty(), "{v} {labels:?}");
        // Two anchors: the one pressed on (B's top-right) lands on A's corner.
        let two = anchors_of(b, &[(0, 0), (0, 1)]);
        let c = cx(&d, &two, &p);
        let (v, _) = drag(&mut t, &c, (400.0, 300.0), (202.0, 198.0), none);
        assert_eq!(v, json!({"dx": -200.0, "dy": -100.0}));
        // Smart Guides off: Snap to Point pulls within its distance (2 px) only.
        let c = ToolContext { smart_guides: false, ..cx(&d, &one, &p) };
        assert_eq!(drag(&mut t, &c, (300.0, 300.0), (201.0, 199.0), none).0, json!({"dx": -100.0, "dy": -100.0}));
        assert_eq!(drag(&mut t, &c, (300.0, 300.0), (203.0, 199.0), none).0, json!({"dx": -97.0, "dy": -101.0}));
        let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &one, &p) };
        assert_eq!(drag(&mut t, &c, (300.0, 300.0), (201.0, 199.0), none).0, json!({"dx": -99.0, "dy": -101.0}));
    }

    /// #2120: the end of a slanted straight line, dragged roughly along it, stays on its direction
    /// (Line Extension) and the guide runs along it from the other end.
    #[test]
    fn dragged_end_of_a_slanted_line_stays_on_its_direction() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::shapes::line(Point::new(300.0, 400.0), Point::new(380.0, 340.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let p = paint();
        let end = anchors_of(id, &[(0, 1)]);
        let c = cx(&d, &end, &p);
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 380.0, 340.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 421.5, 312.0));
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        let (dx, dy) = (v["dx"].as_f64().unwrap(), v["dy"].as_f64().unwrap());
        assert!((dx - 40.0).abs() < 1e-9 && (dy + 30.0).abs() < 1e-9, "{v}");
        let guides: Vec<(Point, Point)> =
            t.overlays(&c).into_iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((a, b)) } else { None }).collect();
        assert!(matches!(guides[..], [(a, b)] if a == Point::new(300.0, 400.0) && (b - Point::new(420.0, 310.0)).hypot() < 1e-9), "{guides:?}");
    }

    /// Group Selection (and Direct Selection on a whole path) moves objects snapping as the
    /// Selection tool does: B's left side lines up with A's right one.
    #[test]
    fn moved_objects_snap_with_smart_guides() {
        let (d, _, b) = two_squares();
        let p = paint();
        let mut s = Selection::default();
        s.set([b]);
        let c = cx(&d, &s, &p);
        for group in [true, false] {
            let mut t = DirectSelectionTool::new(group);
            let (v, _) = drag(&mut t, &c, (350.0, 350.0), (253.0, 351.0), Mods::default());
            assert_eq!(v["matrix"], json!([1.0, 0.0, 0.0, 1.0, -100.0, 1.0]), "group: {group}");
        }
    }

    /// #1816: Group Selection and Direct Selection move a whole path with Snap to Grid as the
    /// Selection tool does, landing an edge on the grid; the center does not snap. A 45 pt square at
    /// 90–135 on the 9 pt grid, dragged right by 22 to 112–157, lands its left edge on the gridline
    /// 108 and moves by 18. Its center, at 134.5 after the drag, is nearer the gridline 135 and
    /// does not snap.
    #[test]
    fn a_move_of_objects_lands_an_edge_on_the_grid() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(90.0, 90.0, 135.0, 135.0));
        d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        let p = paint();
        let mut s = Selection::default();
        s.set([id]);
        let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        for group in [true, false] {
            let mut t = DirectSelectionTool::new(group);
            let (v, _) = drag(&mut t, &c, (110.0, 110.0), (132.0, 110.0), Mods::default());
            assert_eq!(v["matrix"], json!([1.0, 0.0, 0.0, 1.0, 18.0, 0.0]), "group: {group}");
        }
    }

    /// Group Selection and Direct Selection on a whole path keep a Shift-constrained move on its
    /// line when it snaps, as the Selection tool does: level by 13.04 with Snap to Grid on, B's
    /// right side (413.04) lands on the gridline 414, and its top stays at 300, between 297 and
    /// 306.
    #[test]
    fn a_shift_move_of_objects_snaps_along_its_line() {
        let (d, _, b) = two_squares();
        let p = paint();
        let mut s = Selection::default();
        s.set([b]);
        let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let shift = Mods { shift: true, ..Mods::default() };
        for group in [true, false] {
            let mut t = DirectSelectionTool::new(group);
            let (v, _) = drag(&mut t, &c, (350.0, 350.0), (363.0, 351.0), shift);
            let (x, y) = (v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap());
            assert!((x - 14.0).abs() < 1e-9 && y.abs() < 1e-9, "group: {group}: {v}");
        }
    }

    /// Direct Selection's double-click on type where a layer's clipping mask edge runs through it
    /// edits the type, though a press there takes the mask's segment.
    #[test]
    fn a_double_click_on_type_at_a_layer_masks_edge_edits_it() {
        let (d, _, at) = doc_with_type_on_a_layer_masks_edge();
        let (s, p) = (Selection::default(), paint());
        let acts = DirectSelectionTool::new(false).pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::DoubleClick, at.x, at.y));
        assert_eq!(acts, vec![Action::SwitchTool("type".into())]);
    }

    #[test]
    fn double_clicking_type_switches_to_type_tool() {
        use std::sync::Arc;
        let (mut d, id) = doc_with_area_type();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let b = d.node(id).unwrap().geometric_bounds().unwrap();
        let mid = b.center();
        // Direct Selection (shortcut A, group == false) switches to the Type tool on double click.
        let mut t = DirectSelectionTool::new(false);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, mid.x, mid.y));
        assert_eq!(acts, vec![Action::SwitchTool("type".into())]);

        // Group Selection (group == true) does not switch to Type tool.
        let mut gt = DirectSelectionTool::new(true);
        let acts_group = gt.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, mid.x, mid.y));
        assert!(acts_group.is_empty(), "group selection shouldn't switch: {acts_group:?}");

        // Text inside a group also switches to Type tool with Direct Selection.
        let gid = d.alloc_id();
        let text_id = d.alloc_id();
        let text_node = Node::new(
            text_id,
            vectorcraft_doc::NodeKind::Text(Box::new(vectorcraft_doc::TextObject::point(Point::new(50.0, 50.0), "In group", Default::default()))),
        );
        d.insert(Some(d.layers[0].id), 2, Node::group(gid, vec![Arc::new(text_node)])).unwrap();
        let in_group_b = d.node(gid).unwrap().geometric_bounds().unwrap();
        let c2 = cx(&d, &s, &p);
        let acts_group_text = t.pointer(&c2, &PointerEvent::new(PointerKind::DoubleClick, in_group_b.center().x, in_group_b.center().y));
        assert_eq!(acts_group_text, vec![Action::SwitchTool("type".into())]);
    }

    #[test]
    fn direct_selection_click_and_drag_member_of_compound_path() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (outer, hole, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let rect = |id, a: f64, b: f64| {
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()))
        };
        let mut c = vectorcraft_doc::Node::new(
            compound,
            vectorcraft_doc::NodeKind::Compound {
                children: vec![rect(outer, 0.0, 200.0), rect(hole, 50.0, 150.0)],
                rule: vectorcraft_geom::FillRule::EvenOdd,
            },
        );
        c.appearance = vectorcraft_doc::Appearance::default_art().into();
        d.insert(Some(l), 0, c).unwrap();
        let p = paint();
        let none = Selection::default();
        let mut t = DirectSelectionTool::new(false);
        let ctx = cx(&d, &none, &p);
        let down = t.pointer(&ctx, &PointerEvent::new(PointerKind::Down, 25.0, 100.0));
        assert_eq!(
            down,
            vec![Action::Exec("select.set".into(), json!({"ids": [outer.0]}))],
            "Direct Selection selects child member, not compound parent"
        );
        let mut s = Selection::default();
        s.set([outer]);
        let ctx_sel = cx(&d, &s, &p);
        let drag = t.pointer(&ctx_sel, &PointerEvent::new(PointerKind::Drag, 35.0, 110.0));
        assert!(drag.iter().any(|a| matches!(a, Action::Preview(cmd, _) if cmd == "object.transform")), "drags the selected child member: {drag:?}");
        let up = t.pointer(&ctx_sel, &PointerEvent::new(PointerKind::Up, 35.0, 110.0));
        assert_eq!(up, vec![Action::Commit]);
    }

    #[test]
    fn shift_clicking_anchor_after_segment_pick_toggles_or_adds() {
        let (d, id) = doc_with_rect();
        let p = paint();
        let mut s = Selection::default();
        s.set([id]);
        s.segments.insert(id, [(0, 0)].into());
        s.anchors.insert(id, [(0, 0), (0, 1)].into());
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        let shift = Mods { shift: true, ..Default::default() };
        // Shift-click on anchor (0, 0) (top-left at 100, 100) toggles it off
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "toggle"}))]);
        // Shift-click on anchor (0, 2) (bottom-right at 200, 200) toggles it on (adds it)
        let a2 = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 200.0).with_mods(shift));
        assert_eq!(a2, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 2]], "mode": "toggle"}))]);
    }

    #[test]
    fn shift_clicking_segment_toggles_segment() {
        let (d, id) = doc_with_rect();
        let p = paint();
        let s = Selection::default();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        let shift = Mods { shift: true, ..Default::default() };
        // The rectangle is at 100..200. Segment 0 is the top edge (y=100, x between 100 and 200).
        // Shift-click at (150, 100) selects segment 0 with mode "toggle".
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 100.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("select.segments".into(), json!({"id": id.0, "segments": [[0, 0]], "mode": "toggle"}))]);
    }

    /// In normal viewmode, clicking the fill of an open path in front selects the open path's shape
    /// and anchors, and NEVER clicks through to select or grab a path behind it (even if the path
    /// behind has an anchor directly under the click).
    #[test]
    fn clicking_fill_of_open_path_in_front_never_selects_path_behind() {
        use vectorcraft_geom::{Anchor, SubPath};
        let (mut d, _behind) = doc_with_rect();
        // `behind` has an anchor at (100.0, 100.0).
        // Create an open path in front (id `front`), with a fill, covering (100.0, 100.0).
        let front = d.alloc_id();
        let l = d.layers[0].id;
        // Triangle from (50, 50) to (150, 50) to (150, 150), open (implicit closing edge from (150, 150) to (50, 50)).
        // (100.0, 100.0) is inside this filled triangle, but `front` has NO anchor at (100.0, 100.0).
        let anchors = vec![Anchor::corner(Point::new(50.0, 50.0)), Anchor::corner(Point::new(150.0, 50.0)), Anchor::corner(Point::new(150.0, 150.0))];
        let open_path = PathData::single(SubPath::new(anchors, false));
        let ap = vectorcraft_doc::Appearance::default_art(); // has solid white fill
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(front, open_path, ap)).unwrap();

        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);

        // Hovering over (100.0, 100.0) does NOT hover the hidden anchor of `behind`.
        assert_eq!(hovered_anchor(&c, Point::new(100.0, 100.0)), None);

        // Clicking at (100.0, 100.0) selects `front` (the open path in front), NOT `behind`.
        let mut t = DirectSelectionTool::new(false);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({"ids": [front.0]}))]);
    }

    /// A selected path's Bézier direction handle positioned over a background filled rectangle
    /// can be hit and dragged without being occluded by the background fill.
    #[test]
    fn selected_path_handle_over_background_rect_is_draggable() {
        use vectorcraft_geom::{Anchor, SubPath};
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        // Background rectangle covering 0..300 with default white fill
        let bg = d.alloc_id();
        let bg_rect = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 300.0));
        d.insert(Some(l), 0, vectorcraft_doc::Node::path(bg, bg_rect, vectorcraft_doc::Appearance::default_art())).unwrap();

        // Selected path with a Bézier curve whose out-handle is at (150, 150)
        let fg = d.alloc_id();
        let mut a0 = Anchor::corner(Point::new(100.0, 150.0));
        a0.h_out = Point::new(150.0, 150.0); // handle over the background rect
        let a1 = Anchor::corner(Point::new(200.0, 150.0));
        let curve = PathData::single(SubPath::new(vec![a0, a1], false));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(fg, curve, vectorcraft_doc::Appearance::default_art())).unwrap();

        let mut s = Selection::default();
        s.set([fg]);
        s.anchors.insert(fg, [(0, 0)].into());
        let p = paint();
        let c = cx(&d, &s, &p);

        // hit_handle on the handle at (150, 150) must find the handle of `fg`
        assert_eq!(hit_handle(&c, Point::new(150.0, 150.0), c.point_tol()), Some((fg, 0, 0, true)));

        // Direct Selection tool clicking on the handle initiates Reshape
        let mut t = DirectSelectionTool::new(false);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(acts, vec![Action::Begin("Reshape".into())]);
    }
}
