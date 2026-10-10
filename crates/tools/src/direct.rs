//! Direct Selection (A) and Group Selection tools.
//!
//! Direct Selection: click an anchor to select it (Shift toggles), click a segment to select the
//! path's anchors on that segment, drag to move selected anchors (the one pressed on snaps to
//! anchors, segments and Smart Guides, or with them off Snap to Point; Shift keeps the move at 45°
//! steps), drag a direction handle to
//! reshape (Shift keeps it at 45° steps round its anchor, Alt moves it alone; smart guides snap
//! it), marquee to select anchors (Shift-drag toggles them: the selected ones inside are
//! deselected, the others selected), drag a corner widget of any path (a star, a pen path) to round its corners
//! (the selected ones when anchors are selected; Alt-click cycles their kind, double-click opens
//! the Corners dialog).
//! Group Selection: click selects the leaf; each further click on it adds the next enclosing group;
//! its marquee works as Direct Selection's.
//! Both move objects selected as a whole as the Selection tool does, snapping alike.
//! Both pick the key objects of a blend, and click or drag ruler guides ([`crate::rulerguide`]).
//! Direct Selection also edits a blend's spine: drag its points (a key object on a point moves
//! with it) and, once a point is clicked, its handles; and the points of selected gradient meshes
//! and mesh envelopes and their handles ([`MeshEdit`]). Dragging a corner or an edge of area
//! type's frame reshapes the type area (`text.reshapeArea`): the text reflows at its size.
//! Dragging the brackets of selected type on a path moves or flips it ([`crate::pathtype`]).
//!
//! A press on the stroke of a path that isn't selected as a whole selects that segment's two anchors
//! (the fill, or Alt, selects the whole path); dragging the segment bends it if it's curved, else
//! moves its anchors.

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_doc::hit::{HitKind, hit_test};
use vectorcraft_doc::{AnchorRef, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, PathData, Point, Rect};

use crate::bbox::move_delta;
use crate::corners::{self, CornerDrag, over_widget};
use crate::guides::{HandleSnap, Leave, PointSnap, Targets};
use crate::meshedit::MeshEdit;
use crate::pathtype::{self, BracketDrag, over_bracket};
use crate::rulerguide::GuideEdit;
use crate::select::{MoveSnap, is_area_type, matrix_json};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Clone, Debug)]
enum State {
    Idle,
    /// Dragging the selected anchors, which move alike: `grab` is where the anchor pressed on (or
    /// the pointer on a segment) was, the point that snaps.
    MoveAnchors {
        start: Point,
        grab: Point,
        began: bool,
    },
    MoveObject {
        start: Point,
        began: bool,
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
    /// What the dragged handle snaps to.
    handle_snap: HandleSnap,
    /// What the objects being moved snap to, gathered when the move begins.
    move_snap: Option<MoveSnap>,
    /// The anchor under the pointer (Highlight anchors on mouse over): its path, where it is.
    hover: Option<(NodeId, Point)>,
    guide: GuideEdit,
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
            handle_snap: HandleSnap::default(),
            move_snap: None,
            hover: None,
            guide: GuideEdit::default(),
        }
    }
}

/// The spines of the editable blends, topmost first: (blend, spine).
pub(crate) fn spines(cx: &ToolContext) -> Vec<(NodeId, PathData)> {
    let mut v = vec![];
    cx.doc.walk(|n| {
        if let NodeKind::Blend { children, spec } = &n.kind
            && let Some((path, _)) = vectorcraft_doc::live::blend_spine(children, spec)
        {
            v.push((n.id, path));
        }
    });
    v.reverse();
    v.retain(|(id, _)| cx.doc.is_editable(*id));
    v
}

/// The spine point under `p`: (blend, anchor, where it is).
fn hit_spine_point(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, Point)> {
    spines(cx).into_iter().find_map(|(id, path)| {
        let (_, ai, a) = path.anchors().find(|(_, _, a)| a.p.distance(p) <= tol)?;
        Some((id, ai, a.p))
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
    cx.doc.walk(|n| {
        if f(n) {
            v.push(n.id)
        }
    });
    v.reverse();
    v.retain(|id| cx.doc.is_editable(*id));
    v
}

/// The anchor within `tol` of `p` nearest to it, of the topmost visible path or area type frame
/// that has one: (id, si, ai, where it is).
fn hit_anchor(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, Point)> {
    let owners = anchor_owners(cx, |n| matches!(n.kind, NodeKind::Path { .. }) || is_area_type(n));
    owners.into_iter().find_map(|id| {
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
        v.extend(pd.anchors().filter(|(si, ai, _)| shown.is_none_or(|set| set.contains(&(*si, *ai)))).map(|(si, ai, a)| (id, si, ai, *a)));
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
    /// The indexes of the segment's two anchors in its subpath.
    anchors: [usize; 2],
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
    Some(SegmentHit { si, seg, t, anchors: [i0, i1], curved: a.has_out() || b.has_in() })
}

impl Tool for DirectSelectionTool {
    fn id(&self) -> &'static str {
        if self.group { "groupSelection" } else { "directSelection" }
    }
    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle) || self.guide.busy()
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
                let Some(h) = hit_test(cx.doc, p, cx.hit_options()) else {
                    self.state = State::Marquee { start: p, cur: p, toggle: ev.mods.shift };
                    return vec![];
                };
                // Walk up from the leaf: select the first ancestor not yet selected (below the layer).
                let chain: Vec<NodeId> = h.ancestry.iter().skip(1).rev().copied().collect();
                let target = if cx.selection.contains(h.leaf) {
                    chain.iter().find(|id| !cx.selection.contains(**id)).copied().unwrap_or(h.leaf)
                } else {
                    h.leaf
                };
                self.state = State::MoveObject { start: p, began: false };
                if cx.selection.contains(target) {
                    vec![]
                } else if ev.mods.shift || cx.selection.contains(h.leaf) {
                    vec![Action::Exec("select.add".into(), json!({"ids": [target.0]}))]
                } else {
                    vec![Action::Exec("select.set".into(), json!({"ids": [target.0]}))]
                }
            }
            (PointerKind::DoubleClick, _) if !self.group => corners::double_click(cx, p, true).into_iter().collect(),
            (PointerKind::Move, State::Idle) => {
                self.hover = if self.group || !cx.highlight_anchors { None } else { hovered_anchor(cx, p) };
                vec![]
            }
            (PointerKind::Down, _) => {
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
                if let Some(g) = self.mesh.hit(cx, &cx.selection.objects, p) {
                    self.state = State::Mesh;
                    return self.mesh.press(g);
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
                    let already = cx.selection.partial(id).is_some_and(|s| s.contains(&(si, ai)));
                    self.state = State::MoveAnchors { start: p, grab, began: false };
                    if ev.mods.shift {
                        return vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "toggle"}))];
                    }
                    if !already {
                        return vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "set"}))];
                    }
                    return vec![];
                }
                if let Some((id, anchors)) = hit_frame_edge(cx, p, tol) {
                    return self.press_type_area(cx, id, anchors, p, ev.mods.shift);
                }
                if let Some(out) = self.guide.press(cx, ev) {
                    self.state = State::Idle;
                    return out;
                }
                if let Some(h) = hit_test(cx.doc, p, cx.hit_options()) {
                    // A segment of a path that isn't selected as a whole: its two anchors get
                    // selected, and dragging it reshapes it if it's curved, else moves it. Alt picks
                    // the whole path (and Alt-drag copies it), as with Group Selection.
                    let whole = cx.selection.contains(h.leaf) && cx.selection.partial(h.leaf).is_none();
                    if !whole
                        && !ev.mods.shift
                        && !ev.mods.alt
                        && matches!(h.kind, HitKind::Stroke | HitKind::Outline)
                        && let Some(s) = segment_at(cx, h.leaf, p)
                    {
                        let selected = cx.selection.partial(h.leaf).is_some_and(|sel| s.anchors.iter().all(|&ai| sel.contains(&(s.si, ai))));
                        self.state = if s.curved {
                            State::Segment { id: h.leaf, si: s.si, seg: s.seg, t: s.t, start: p, began: false }
                        } else {
                            State::MoveAnchors { start: p, grab: p, began: false }
                        };
                        if selected {
                            return vec![];
                        }
                        let anchors: Vec<Value> = s.anchors.iter().map(|ai| json!([s.si, ai])).collect();
                        return vec![Action::Exec("select.anchors".into(), json!({"id": h.leaf.0, "anchors": anchors, "mode": "set"}))];
                    }
                    // Clicking the fill selects the whole leaf path (all anchors).
                    self.state = State::MoveObject { start: p, began: false };
                    if ev.mods.shift {
                        return vec![Action::Exec("select.toggle".into(), json!({"id": h.leaf.0}))];
                    }
                    if !cx.selection.contains(h.leaf) || cx.selection.partial(h.leaf).is_some() {
                        return vec![Action::Exec("select.set".into(), json!({"ids": [h.leaf.0]}))];
                    }
                    return vec![];
                }
                self.state = State::Marquee { start: p, cur: p, toggle: ev.mods.shift };
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
                let mut d = move_delta(start, p, ev.mods.shift);
                // The grabbed anchor snaps and the others follow it. Shift keeps the move at its
                // angle, and the anchor slides along it onto what Smart Guides find there (#886).
                self.guides.clear();
                if let Some(snap) = &self.anchor_snap {
                    let shift = ev.mods.shift.then(|| Leave::segment(cx, grab, true));
                    let (q, guides) = snap.snap_from(cx, grab + (p - start), shift.as_ref());
                    (d, self.guides) = (q - grab, guides);
                }
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
                    json!({"id": id.0, "subpath": si, "segment": seg, "t": t, "dx": d.x, "dy": d.y}),
                ));
                out
            }
            (PointerKind::Drag, State::MoveObject { start, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Move".into()));
                    self.state = State::MoveObject { start, began: true };
                    self.move_snap = Some(MoveSnap::new(cx));
                }
                let mut d = move_delta(start, p, ev.mods.shift);
                if let Some(snap) = &self.move_snap {
                    (d, self.guides) = snap.snap(cx, start, d);
                }
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
                let d = move_delta(start, p, ev.mods.shift);
                out.push(Action::Preview("text.reshapeArea".into(), json!({"id": id.0, "anchors": anchors_json(&anchors), "dx": d.x, "dy": d.y})));
                self.state = State::TypeArea { id, anchors, start, began: true };
                out
            }
            (PointerKind::Drag, State::Handle { id, si, ai, out }) => {
                let (q, guides) = self.handle_snap.snap(cx, (id, si, ai), p, ev.mods.shift);
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
                let q = from + move_delta(start, p, ev.mods.shift);
                out.push(Action::Preview("object.blend.spine.moveAnchor".into(), json!({"id": id.0, "anchor": anchor, "x": q.x, "y": q.y})));
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
            (PointerKind::Drag, State::Marquee { start, toggle, .. }) => {
                self.state = State::Marquee { start, cur: p, toggle };
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
                | State::TypeArea { began, .. },
            ) => {
                self.state = State::Idle;
                self.guides.clear();
                (self.anchor_snap, self.move_snap) = (None, None);
                if began { vec![Action::Commit] } else { vec![] }
            }
            (PointerKind::Up, State::Handle { .. } | State::SpineHandle { .. }) => {
                self.state = State::Idle;
                self.guides.clear();
                vec![Action::Commit]
            }
            (PointerKind::Up, State::Marquee { start, toggle, .. }) => {
                self.state = State::Idle;
                let r = Rect::from_points(start, p);
                if r.width() < cx.tol(3.0) && r.height() < cx.tol(3.0) {
                    return if toggle { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
                }
                // Collect anchors inside the rect for every editable path.
                let mut sel: Vec<(NodeId, Vec<AnchorRef>)> = vec![];
                cx.doc.walk(|n| {
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
                sel.retain(|(id, _)| cx.doc.is_editable(*id));
                let items: Vec<Value> = sel.iter().map(|(id, v)| json!({"id": id.0, "anchors": anchors_json(v)})).collect();
                vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": if toggle { "toggle" } else { "set" }}))]
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
    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if !self.group && (matches!(self.state, State::Corner(_)) || over_widget(cx, p, true)) {
            return Cursor::CornerRadius;
        }
        if !self.group && (matches!(self.state, State::Bracket(_)) || over_bracket(cx, p)) {
            return Cursor::PathBracket;
        }
        // A press on the highlighted anchor picks it, not the guide.
        self.guide.cursor(cx, p).filter(|_| self.hover.is_none() || self.guide.busy()).unwrap_or(Cursor::ArrowHollow)
    }
}

/// The anchor within a press's reach of `p` ([`ToolContext::point_tol`]), of a selected path or the
/// path under `p`: what Highlight anchors on mouse over marks (a press there picks it).
fn hovered_anchor(cx: &ToolContext, p: Point) -> Option<(NodeId, Point)> {
    let under = hit_test(cx.doc, p, cx.hit_options()).map(|h| h.leaf);
    let tol = cx.point_tol();
    cx.selection.objects.iter().copied().chain(under).find_map(|id| {
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

    #[test]
    fn dragging_a_curved_segment_reshapes_it_and_a_straight_one_moves_its_anchors() {
        let (d, arch) = arch_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        // The arch: its two anchors get selected, and the drag bends it.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
        assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": arch.0, "anchors": [[0, 0], [0, 1]], "mode": "set"}))]);
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
        assert!(matches!(&a[..], [Action::Exec(c, v)] if c == "select.anchors" && v["anchors"] == json!([[0, 0], [0, 1]])), "{a:?}");
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

    /// [`doc_with_rect`] (A, 100..200) and a second square B, 300..400: (A, B).
    fn two_squares() -> (vectorcraft_doc::Document, NodeId, NodeId) {
        let (mut d, a) = doc_with_rect();
        let l = d.layers[0].id;
        let b = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
        (d, a, b)
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
        let s = move_delta(Point::new(300.0, 300.0), Point::new(380.0, 302.0), true);
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
}
