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
//! type's frame reshapes the type area (`text.reshapeArea`): the text reflows at its size; dragging
//! an anchor of type on a path's path reshapes the path (`text.reshapePath`): the text reflows along it.
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
use crate::select::{MoveSnap, hold_point, is_area_type, is_path_type, matrix_json, release_guide_at};
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
    /// Dragging anchors of area type's frame (a corner, or the two ends of an edge), or an anchor
    /// of type on a path's path.
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

/// The anchors Direct Selection edits on `n`, in document space: a path's, area type's frame or
/// type on a path's path.
fn editable_path(n: &Node) -> Option<Cow<'_, PathData>> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(Cow::Borrowed(path)),
        NodeKind::Text(t) => t.area_frame().or_else(|| t.type_path()).map(Cow::Owned),
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
    let mut owners = anchor_owners(cx, |n| matches!(n.kind, NodeKind::Path { .. }) || is_area_type(n) || is_path_type(n));
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

/// Is `p` within a press's reach of an anchor of type on a path's path? The anchor wins over a
/// bracket standing on it (the end bracket on the last anchor, #3980); the bracket's stem above it
/// still drags the bracket.
fn over_path_type_anchor(cx: &ToolContext, p: Point) -> bool {
    hit_anchor(cx, p, cx.point_tol()).is_some_and(|(id, ..)| cx.doc.node(id).is_some_and(is_path_type))
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
                if !over_path_type_anchor(cx, p)
                    && let Some(b) = BracketDrag::hit(cx, ev)
                {
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
                    if cx.doc.node(id).is_some_and(|n| is_area_type(n) || is_path_type(n)) {
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
                let on_path = cx.doc.node(id).is_some_and(is_path_type);
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin(if on_path { "Reshape Type Path" } else { "Reshape Type Area" }.into()));
                }
                let d = move_delta(start, p, ev.mods.shift, cx.constrain_angle);
                let cmd = if on_path { "text.reshapePath" } else { "text.reshapeArea" };
                out.push(Action::Preview(cmd.into(), json!({"id": id.0, "anchors": anchors_json(&anchors), "dx": d.x, "dy": d.y})));
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
        if !self.group && (matches!(self.state, State::Bracket(_)) || over_bracket(cx, p) && !over_path_type_anchor(cx, p)) {
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

/// The frame anchors of the selected area type and the path anchors of the selected type on a
/// path, which Direct Selection drags.
fn frame_overlays(cx: &ToolContext) -> Vec<Overlay> {
    let mut out = vec![];
    for id in &cx.selection.objects {
        let Some(frame) = cx.doc.node(*id).filter(|n| is_area_type(n) || is_path_type(n)).and_then(editable_path) else { continue };
        let color = cx.doc.layer_color(*id);
        out.extend(frame.anchors().map(|(_, _, a)| Overlay::Anchor { p: a.p, color, filled: false, size: 5.0 }));
    }
    out
}

impl DirectSelectionTool {
    /// Press on area type's frame anchors (a corner, or an edge's two ends) or an anchor of type
    /// on a path's path: select the text and get ready to drag them.
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
mod tests;
