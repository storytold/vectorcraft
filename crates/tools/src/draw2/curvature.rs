//! The Curvature tool (Shift+~).
//!
//! Each click adds a point and the path curves smoothly through all points. Alt-click or
//! double-click a point toggles it between smooth and corner; drag a point to move it; click the
//! first point to close; Backspace/Delete removes the last touched point; Esc/Enter ends the path.
//! A rubber band shows the curve to the cursor (Enable Rubber Band for Curvature Tool). Each point
//! placed or dragged snaps to Smart Guides ([`DrawSnap`]), as the Pen's anchors do.
//!
//! It edits any selected path too, whatever drew it, keeping its shape except where edited
//! (#798). A click on one of its points makes that the current point (direct-selected); a drag
//! moves it, re-curving only the two segments at it ([`curvature_move`]); Alt-click or
//! double-click toggles it between smooth and corner and Backspace/Delete removes it. A click on
//! a segment adds a smooth point there without changing the shape ([`curvature_insert`]), and a
//! drag moves the new point at once. While the current point is an end of an open path, clicks go
//! on from it ([`curvature_extend`]): the old segments keep their shape, the new ones curve
//! through the new points, and a click on the path's other end closes it. Each edit is one
//! `path.curvatureEdit`, `path.convertAnchor` or `path.removeAnchor` (one undo step).

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, Selection};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, Point, SubPath, Vec2};

use super::{FEEDBACK, anchor_in, catmull_rom, editable_paths, remove_anchor, segment_in};
use crate::guides::{DrawSnap, Leave, Targets};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The command that edits a selected path.
const EDIT: &str = "path.curvatureEdit";

#[derive(Default)]
pub struct CurvatureTool {
    pts: Vec<(Point, bool)>,
    closed: bool,
    id: Option<NodeId>,
    /// The point being dragged, the pointer at the press, and whether the drag began.
    drag: Option<(Grab, Point, bool)>,
    /// Last touched point of the path being drawn (Backspace removes it).
    current: Option<usize>,
    hover: Option<Point>,
    /// The end the last click on a selected path placed.
    placed: Option<Placed>,
    snap: DrawSnap,
}

/// What a press grabbed for a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// A point of the path being drawn.
    Drawn(usize),
    /// An anchor of a selected path: (path, subpath, anchor).
    Anchor(NodeId, usize, usize),
    /// The anchor the press added on a segment of a selected path: (path, subpath, segment, t).
    Inserted(NodeId, usize, usize, f64),
}

/// The end a click on a selected path placed, while it still is that end (its subpath has as many
/// anchors as then): going on from it, it curves through its neighbours, or stays a corner when it
/// was Alt-clicked. Any other end keeps its curve.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    id: NodeId,
    si: usize,
    start: bool,
    anchors: usize,
    corner: bool,
}

/// The current point of a selected path while it is an end of an open subpath: where clicks go on
/// from (its start with `start`), and the other end, which a click closes the subpath on.
#[derive(Clone, Copy)]
struct End {
    id: NodeId,
    si: usize,
    start: bool,
    anchors: usize,
    p: Point,
    other: Point,
}

impl End {
    /// `end` for `path.curvatureEdit`.
    fn name(&self) -> &'static str {
        if self.start { "start" } else { "end" }
    }
}

/// How the end a path goes on from bends into the new segment ([`curvature_extend`],
/// [`curvature_close`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EndCurve {
    /// The segment it ends keeps its shape: the new handle goes on along its handle, or along the
    /// curve where it has none, and with both it is smooth.
    #[default]
    Keep,
    /// Smooth through its neighbours, as [`catmull_rom`] curves a point (a point the Curvature tool
    /// placed: its segment re-curves).
    Smooth,
    /// A corner, without handles.
    Corner,
}

impl EndCurve {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "keep" => Self::Keep,
            "smooth" => Self::Smooth,
            "corner" => Self::Corner,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Smooth => "smooth",
            Self::Corner => "corner",
        }
    }
}

/// The anchors before and after anchor `i` of `sp` (round the start of a closed one).
fn neighbours(sp: &SubPath, i: usize) -> [Option<usize>; 2] {
    let n = sp.anchors.len();
    let wrap = sp.closed && n > 1;
    let prev = if i > 0 { Some(i - 1) } else { wrap.then(|| n - 1) };
    let next = if i + 1 < n { Some(i + 1) } else { wrap.then_some(0) };
    [prev, next]
}

/// Move anchor `i` of `sp` to `q` as the Curvature tool drags a point: only the two segments at it
/// change. A smooth anchor turns along the line through its neighbours; the neighbours' handles on
/// those segments, and a corner's own handles, keep their directions, their lengths scaling with
/// their segment's chord, so the segments beyond don't move and a corner without handles keeps
/// none. False when there is no anchor `i`.
pub fn curvature_move(sp: &mut SubPath, i: usize, q: Point) -> bool {
    let Some(&a) = sp.anchors.get(i) else { return false };
    let mut own = [a.h_in - a.p, a.h_out - a.p];
    let mut ends = [None; 2];
    for (side, j) in neighbours(sp, i).into_iter().enumerate() {
        let Some(b) = j.and_then(|j| sp.anchors.get_mut(j)) else { continue };
        let old = a.p.distance(b.p);
        let s = if old > 1e-9 { q.distance(b.p) / old } else { 1.0 };
        let h = if side == 0 { &mut b.h_out } else { &mut b.h_in };
        *h = b.p + (*h - b.p) * s;
        own[side] *= s;
        ends[side] = Some(b.p);
    }
    let mut m = Anchor { p: q, h_in: q + own[0], h_out: q + own[1], kind: a.kind };
    if let (AnchorKind::Smooth, [Some(prev), Some(next)]) = (a.kind, ends) {
        let d = next - prev;
        let l = d.hypot();
        if l > 1e-9 {
            // A handle it lacks comes out a third of the way to that side's neighbour.
            let len = |h: Vec2, to: Point| if h.hypot() > 1e-9 { h.hypot() } else { q.distance(to) / 3.0 };
            m.h_in = q - d * (len(own[0], prev) / l);
            m.h_out = q + d * (len(own[1], next) / l);
        }
    }
    if let Some(x) = sp.anchors.get_mut(i) {
        *x = m;
    }
    true
}

/// Add a smooth anchor at `t` on segment `seg` of `sp` without changing its shape (on a straight
/// segment its handles lie along it, so a drag curves it): the new anchor's index. None when there
/// is no such segment.
pub fn curvature_insert(sp: &mut SubPath, seg: usize, t: f64) -> Option<usize> {
    if seg >= sp.segment_count() {
        return None;
    }
    let line = sp.segment_is_line(seg);
    let i = sp.insert_anchor(seg, if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 });
    if line {
        sp.smooth_anchor(i);
    }
    Some(i)
}

/// Go on from an end of the open subpath `sp` (its start with `start`) to a new end at `p`, which
/// has no handles; the old end bends into the new segment as `how` says.
pub fn curvature_extend(sp: &mut SubPath, start: bool, p: Point, how: EndCurve) {
    if start {
        sp.reverse();
    }
    bend_last(sp, p, how);
    sp.anchors.push(Anchor::corner(p));
    if start {
        sp.reverse();
    }
}

/// Close the open subpath `sp` from its end (its start with `start`): that end bends into the
/// closing segment as `how` says, and the other keeps its curve.
pub fn curvature_close(sp: &mut SubPath, start: bool, how: EndCurve) {
    if start {
        sp.reverse();
    }
    if let (Some(first), Some(last)) = (sp.anchors.first().map(|a| a.p), sp.anchors.last().map(|a| a.p)) {
        bend_last(sp, first, how);
        sp.reverse();
        bend_last(sp, last, EndCurve::Keep);
        sp.reverse();
        sp.closed = true;
    }
    if start {
        sp.reverse();
    }
}

/// Bend the last anchor of `sp` into a new segment on to `to`, as `how` says.
fn bend_last(sp: &mut SubPath, to: Point, how: EndCurve) {
    let prev = sp.anchors.len().checked_sub(2).and_then(|j| sp.anchors.get(j)).copied();
    let Some(e) = sp.anchors.last_mut() else { return };
    match how {
        EndCurve::Corner => e.retract(),
        EndCurve::Smooth => {
            let t = prev.map_or(Vec2::ZERO, |b| (to - b.p) / 6.0);
            if t.hypot() > 1e-9 {
                *e = Anchor { p: e.p, h_in: e.p - t, h_out: e.p + t, kind: AnchorKind::Smooth };
            } else {
                e.retract();
            }
        }
        EndCurve::Keep => {
            // The way the curve comes in: from its handle, or else the segment's tangent there.
            let back = if e.has_in() { Some(e.h_in) } else { prev.map(|b| if b.h_out.distance(e.p) > 1e-9 { b.h_out } else { b.p }) };
            let d = back.map_or(Vec2::ZERO, |b| e.p - b);
            let l = d.hypot();
            e.h_out = if l > 1e-9 { e.p + d * (e.p.distance(to) / 3.0 / l) } else { e.p };
            if e.has_in() && e.has_out() {
                e.kind = AnchorKind::Smooth;
            }
        }
    }
}

/// Subpath `si` of `id`, a selected path the tool edits.
fn subpath<'a>(cx: &ToolContext<'a>, id: NodeId, si: usize) -> Option<&'a SubPath> {
    if !editable_paths(cx).any(|e| e == id) {
        return None;
    }
    cx.doc.node(id)?.path_data()?.subpaths.get(si)
}

/// The current point of a selected path: its one direct-selected anchor (path, subpath, anchor).
fn current_point(cx: &ToolContext) -> Option<(NodeId, usize, usize)> {
    let mut sets = cx.selection.anchors.iter().filter(|(_, set)| !set.is_empty());
    let (&id, set) = sets.next()?;
    if set.len() != 1 || sets.next().is_some() {
        return None;
    }
    let &(si, ai) = set.first()?;
    subpath(cx, id, si).filter(|sp| ai < sp.anchors.len()).map(|_| (id, si, ai))
}

/// The current point while it is an end of an open subpath.
fn current_end(cx: &ToolContext) -> Option<End> {
    let (id, si, ai) = current_point(cx)?;
    let sp = subpath(cx, id, si)?;
    let n = sp.anchors.len();
    let (first, last) = (sp.anchors.first()?.p, sp.anchors.last()?.p);
    if sp.closed || (ai > 0 && ai + 1 < n) {
        return None;
    }
    // A one-anchor subpath goes on from its end.
    let start = ai == 0 && n > 1;
    let (p, other) = if start { (first, last) } else { (last, first) };
    Some(End { id, si, start, anchors: n, p, other })
}

/// Make anchor `(id, si, ai)` the current point.
fn select_point((id, si, ai): (NodeId, usize, usize)) -> Action {
    Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "set"}))
}

/// Toggle anchor `(id, si, ai)` of a selected path between smooth and corner.
fn toggle(cx: &ToolContext, (id, si, ai): (NodeId, usize, usize)) -> Option<Action> {
    let a = subpath(cx, id, si)?.anchors.get(ai)?;
    let to = if a.kind == AnchorKind::Smooth { "corner" } else { "smooth" };
    Some(Action::Exec("path.convertAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai, "to": to})))
}

/// Snap targets for dragging anchors `anchors` of subpath `si` of `id` (the rest of the path
/// pulls).
fn hold_targets(snap: &mut DrawSnap, cx: &ToolContext, id: NodeId, si: usize, anchors: &[usize]) {
    let sel = Selection { objects: vec![id], anchors: [(id, anchors.iter().map(|&ai| (si, ai)).collect())].into(), ..Selection::default() };
    snap.hold(cx, || Targets::for_anchor_drag(cx.doc, &sel));
}

/// How near a press picks a point.
fn point_tol(cx: &ToolContext) -> f64 {
    cx.tol(5.0).max(cx.point_tol())
}

impl CurvatureTool {
    /// The path being drawn, if it is still the single selected object and matches our points.
    fn active(&self, cx: &ToolContext) -> Option<NodeId> {
        let id = self.id?;
        if cx.selection.objects != [id] {
            return None;
        }
        let pd = cx.doc.node(id)?.path_data()?;
        (pd.subpaths.len() == 1 && pd.anchor_count() == self.pts.len()).then_some(id)
    }

    fn params(&self) -> Value {
        let pts: Vec<Value> = self.pts.iter().map(|(p, c)| json!({"x": p.x, "y": p.y, "corner": c})).collect();
        let mut v = json!({"points": pts, "closed": self.closed});
        if let Some(id) = self.id {
            v["id"] = json!(id.0);
        }
        v
    }

    fn hit_point(&self, cx: &ToolContext, p: Point) -> Option<usize> {
        let tol = cx.tol(5.0);
        self.pts.iter().position(|(q, _)| q.distance(p) <= tol)
    }

    fn reset(&mut self) {
        self.pts.clear();
        self.closed = false;
        self.id = None;
        self.drag = None;
        self.current = None;
        self.placed = None;
        self.snap.clear();
    }

    /// The path the next click goes on (left out of the snap targets, its anchors still pulling)
    /// and the segment to it: the path being drawn, or the selected one whose end is current.
    fn leave(&self, cx: &ToolContext) -> Option<(NodeId, Leave)> {
        match self.active(cx) {
            Some(id) => self.pts.last().map(|(p, _)| (id, Leave::segment(cx, *p, false))),
            None => current_end(cx).map(|e| (e.id, Leave::segment(cx, e.p, false))),
        }
    }

    /// How end `e` bends into the next segment: as [`Placed`] says for the end the last click
    /// placed, else it keeps its curve.
    fn how(&self, e: &End) -> EndCurve {
        match self.placed {
            Some(pl) if (pl.id, pl.si, pl.start, pl.anchors) == (e.id, e.si, e.start, e.anchors) => {
                if pl.corner {
                    EndCurve::Corner
                } else {
                    EndCurve::Smooth
                }
            }
            _ => EndCurve::Keep,
        }
    }

    /// A press on a selected path the tool isn't drawing: on the other end of the open path going
    /// on (it closes), on a point, anywhere while an end is current (the path goes on), or on a
    /// segment. None elsewhere: a new path starts.
    fn press_selected(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let (p, tol) = (ev.pos, point_tol(cx));
        let end = current_end(cx);
        if let Some(e) = end.filter(|e| e.anchors >= 3 && p.distance(e.other) <= tol) {
            let how = self.how(&e);
            self.placed = None;
            let v = json!({"id": e.id.0, "subpath": e.si, "op": "close", "end": e.name(), "from": how.name()});
            return Some(vec![Action::Exec(EDIT.into(), v)]);
        }
        if let Some(hit) = anchor_in(cx, editable_paths(cx), p, tol) {
            let mut out = vec![select_point(hit)];
            if ev.mods.alt {
                out.extend(toggle(cx, hit));
                return Some(out);
            }
            let (id, si, ai) = hit;
            hold_targets(&mut self.snap, cx, id, si, &[ai]);
            self.drag = Some((Grab::Anchor(id, si, ai), p, false));
            return Some(out);
        }
        if let Some(e) = end {
            let q = self.snap.press(cx, p, &[e.id], Some(&Leave::segment(cx, e.p, false)));
            let how = self.how(&e);
            self.placed = Some(Placed { id: e.id, si: e.si, start: e.start, anchors: e.anchors + 1, corner: ev.mods.alt });
            let v = json!({"id": e.id.0, "subpath": e.si, "op": "extend", "end": e.name(), "x": q.x, "y": q.y, "from": how.name()});
            return Some(vec![Action::Exec(EDIT.into(), v)]);
        }
        let (id, si, seg, t) = segment_in(cx, editable_paths(cx), p, tol)?;
        let n = subpath(cx, id, si)?.anchors.len().max(1);
        hold_targets(&mut self.snap, cx, id, si, &[seg, (seg + 1) % n]);
        self.drag = Some((Grab::Inserted(id, si, seg, t), p, true));
        let v = json!({"id": id.0, "subpath": si, "op": "insert", "segment": seg, "t": t});
        Some(vec![Action::Begin("Curvature".into()), Action::Preview(EDIT.into(), v)])
    }
}

impl Tool for CurvatureTool {
    fn id(&self) -> &'static str {
        "curvature"
    }
    fn busy(&self) -> bool {
        self.drag.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let active = self.active(cx);
        if active.is_none() && self.id.is_some() && matches!(ev.kind, PointerKind::Down) {
            self.reset();
        }
        match ev.kind {
            PointerKind::Move => {
                let (on, from) = self.leave(cx).unzip();
                self.hover = Some(self.snap.hover(cx, p, on.as_slice(), from.as_ref()));
                vec![]
            }
            PointerKind::Down => {
                if let Some(id) = active {
                    if let Some(i) = self.hit_point(cx, p) {
                        self.current = Some(i);
                        if ev.mods.alt {
                            self.pts[i].1 = !self.pts[i].1;
                            return vec![Action::Exec("path.curvature".into(), self.params())];
                        }
                        if i == 0 && !self.closed && self.pts.len() >= 3 {
                            self.closed = true;
                            return vec![Action::Exec("path.curvature".into(), self.params())];
                        }
                        // The point dragged and the curve through it move: the rest pull.
                        hold_targets(&mut self.snap, cx, id, 0, &[i]);
                        self.drag = Some((Grab::Drawn(i), p, false));
                        return vec![];
                    }
                    if !self.closed {
                        let from = self.leave(cx).map(|l| l.1);
                        let p = self.snap.press(cx, p, &[id], from.as_ref());
                        self.pts.push((p, ev.mods.alt));
                        self.current = Some(self.pts.len() - 1);
                        return vec![Action::Exec("path.curvature".into(), self.params())];
                    }
                }
                if let Some(out) = self.press_selected(cx, ev) {
                    return out;
                }
                // Start a new path.
                self.reset();
                let p = self.snap.press(cx, p, &[], None);
                self.pts.push((p, false));
                self.current = Some(0);
                vec![Action::Exec("path.curvature".into(), self.params()), Action::Notify("created".into())]
            }
            PointerKind::Drag => {
                let Some((grab, start, began)) = self.drag else { return vec![] };
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Curvature".into()));
                    self.drag = Some((grab, start, true));
                }
                let p = self.snap.drag(cx, p, None);
                out.push(match grab {
                    Grab::Drawn(i) => {
                        if let Some(pt) = self.pts.get_mut(i) {
                            pt.0 = p;
                        }
                        Action::Preview("path.curvature".into(), self.params())
                    }
                    Grab::Anchor(id, si, ai) => {
                        Action::Preview(EDIT.into(), json!({"id": id.0, "subpath": si, "op": "move", "anchor": ai, "x": p.x, "y": p.y}))
                    }
                    Grab::Inserted(id, si, seg, t) => {
                        Action::Preview(EDIT.into(), json!({"id": id.0, "subpath": si, "op": "insert", "segment": seg, "t": t, "x": p.x, "y": p.y}))
                    }
                });
                out
            }
            PointerKind::Up => {
                self.snap.clear();
                match self.drag.take() {
                    Some((_, _, true)) => vec![Action::Commit],
                    _ => vec![],
                }
            }
            PointerKind::DoubleClick => {
                if active.is_none() {
                    return anchor_in(cx, editable_paths(cx), p, point_tol(cx)).and_then(|hit| toggle(cx, hit)).into_iter().collect();
                }
                let Some(i) = self.hit_point(cx, p) else { return vec![] };
                self.pts[i].1 = !self.pts[i].1;
                vec![Action::Exec("path.curvature".into(), self.params())]
            }
        }
    }
    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "created" && cx.selection.objects.len() == 1 {
            self.id = Some(cx.selection.objects[0]);
        }
    }
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Backspace | ToolKey::Delete)
            && self.drag.is_none()
            && if self.active(cx).is_some() { self.pts.len() > 1 } else { current_point(cx).is_some() }
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape | ToolKey::Enter => {
                let dragging = self.drag.is_some_and(|d| d.2);
                let drawing = self.active(cx).is_some();
                self.reset();
                if dragging {
                    return vec![Action::Cancel];
                }
                // A selected path's current point goes: clicks no longer go on from it.
                if drawing || current_point(cx).is_none() {
                    return vec![];
                }
                vec![Action::Exec("select.set".into(), json!({"ids": crate::json_ids(&cx.selection.objects)}))]
            }
            ToolKey::Backspace | ToolKey::Delete if self.claims_key(cx, key) => {
                if self.active(cx).is_none() {
                    return current_point(cx).map(remove_anchor).into_iter().collect();
                }
                let i = self.current.unwrap_or(self.pts.len() - 1).min(self.pts.len() - 1);
                self.pts.remove(i);
                if self.pts.len() < 3 {
                    self.closed = false;
                }
                self.current = None;
                vec![Action::Exec("path.curvature".into(), self.params())]
            }
            _ => vec![],
        }
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let dragging = self.drag.is_some_and(|d| d.2);
        self.reset();
        if dragging { vec![Action::Commit] } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        let band = cx.curvature_rubber_band && self.drag.is_none();
        let mut bp = BezPath::new();
        if self.active(cx).is_none() {
            // The curve on from the current end of a selected path: its last segment and the next.
            if band
                && let (Some(e), Some(h)) = (current_end(cx), self.hover)
                && let Some(sp) = subpath(cx, e.id, e.si)
            {
                let n = sp.anchors.len();
                let near = if e.start {
                    sp.anchors.iter().take(2).rev().map(Anchor::reversed).collect()
                } else {
                    sp.anchors.iter().skip(n.saturating_sub(2)).copied().collect()
                };
                let mut tail = SubPath::new(near, false);
                curvature_extend(&mut tail, false, h, self.how(&e));
                tail.to_bezpath_into(&mut bp);
                o.push(Overlay::Path { path: bp, color: FEEDBACK, width: 1.0, dashed: false });
            }
            return o;
        }
        if band
            && !self.closed
            && let Some(h) = self.hover
        {
            let mut pts = self.pts.clone();
            pts.push((h, false));
            catmull_rom(&pts, false).to_bezpath_into(&mut bp);
            o.push(Overlay::Path { path: bp, color: FEEDBACK, width: 1.0, dashed: false });
        }
        for (i, (p, _)) in self.pts.iter().enumerate() {
            o.push(Overlay::Anchor { p: *p, color: FEEDBACK, filled: Some(i) == self.current, size: 6.0 });
        }
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if self.active(cx).is_some() {
            return match self.hit_point(cx, p) {
                Some(0) if !self.closed && self.pts.len() >= 3 => Cursor::PenClose,
                Some(_) if m.alt => Cursor::PenConvert,
                Some(_) => Cursor::Move,
                None => Cursor::Pen,
            };
        }
        let tol = point_tol(cx);
        let end = current_end(cx);
        if end.is_some_and(|e| e.anchors >= 3 && p.distance(e.other) <= tol) {
            return Cursor::PenClose;
        }
        if let Some((id, si, ai)) = anchor_in(cx, editable_paths(cx), p, tol) {
            let open_end = subpath(cx, id, si).is_some_and(|sp| !sp.closed && (ai == 0 || ai + 1 == sp.anchors.len()));
            return if m.alt {
                Cursor::PenConvert
            } else if open_end {
                Cursor::PenContinue
            } else {
                Cursor::Move
            };
        }
        if end.is_none() && segment_in(cx, editable_paths(cx), p, tol).is_some() {
            return Cursor::PenAdd;
        }
        Cursor::Pen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::{ParamCurve, PathData};

    #[test]
    fn first_click_creates_then_notify_tracks_path() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CurvatureTool::default();
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.curvature" && v.get("id").is_none() && v["points"].as_array().unwrap().len() == 1));
        assert_eq!(a[1], Action::Notify("created".into()));
        // Pretend the engine created `id` with one anchor: it is a rect (4 anchors) so it doesn't match.
        let mut s2 = Selection::default();
        s2.set([id]);
        let cx2 = crate::testutil::cx(&d, &s2, &p);
        t.notify(&cx2, "created");
        assert_eq!(t.id, Some(id));
        assert!(t.active(&cx2).is_none());
        // With four points it matches, so a click appends a fifth.
        t.pts = vec![(Point::new(100.0, 100.0), false); 4];
        let a = t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 400.0, 400.0));
        assert!(matches!(&a[0], Action::Exec(_, v) if v["id"] == id.0 && v["points"].as_array().unwrap().len() == 5));
    }

    /// Points snap to Smart Guides (#506): the first lands on another object's anchor and says so.
    #[test]
    fn points_snap_to_smart_guides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CurvatureTool::default();
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 202.0, 99.0));
        assert!(matches!(&a[0], Action::Exec(_, v) if v["points"][0]["x"] == 200.0 && v["points"][0]["y"] == 100.0), "{a:?}");
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "anchor")));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 202.0, 99.0));
        assert!(t.overlays(&cx).is_empty());
    }

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    /// An open pen-style path: a corner start, two smooth anchors with uneven handles, a cusp and
    /// an end with only an incoming handle.
    fn wave() -> SubPath {
        SubPath::new(
            vec![
                Anchor::corner(pt(0.0, 100.0)),
                Anchor::with_handles(pt(100.0, 0.0), pt(60.0, 10.0), pt(160.0, -15.0)),
                Anchor::with_handles(pt(250.0, 80.0), pt(220.0, 30.0), pt(280.0, 130.0)),
                Anchor::with_handles(pt(350.0, 150.0), pt(320.0, 160.0), pt(380.0, 100.0)),
                Anchor::with_handles(pt(450.0, 60.0), pt(420.0, 90.0), pt(450.0, 60.0)),
            ],
            false,
        )
    }

    /// Every point of `a` lies on `b` (within 1e-6), sampled.
    fn lies_on(a: &SubPath, b: &SubPath) -> bool {
        let b = PathData::single(b.clone());
        (0..a.segment_count()).all(|i| (0..=20).all(|k| b.nearest(a.segment(i).eval(k as f64 / 20.0)).is_some_and(|n| n.4 < 1e-6)))
    }

    fn collinear(a: Vec2, b: Vec2) -> bool {
        a.cross(b).abs() < 1e-6 * a.hypot().max(1.0) * b.hypot().max(1.0)
    }

    #[test]
    fn moving_a_point_recurves_only_its_two_segments() {
        let before = wave();
        let mut sp = before.clone();
        assert_eq!(sp.anchors[2].kind, AnchorKind::Smooth);
        assert!(curvature_move(&mut sp, 2, pt(260.0, 140.0)));
        assert_eq!(sp.anchors[2].p, pt(260.0, 140.0));
        // The segments beyond its neighbours are bit-identical.
        assert_eq!(sp.segment(0), before.segment(0));
        assert_eq!(sp.segment(3), before.segment(3));
        // The neighbours keep their handle directions on its segments, and their other handles.
        assert!(collinear(sp.anchors[1].h_out - sp.anchors[1].p, before.anchors[1].h_out - before.anchors[1].p));
        assert!(collinear(sp.anchors[3].h_in - sp.anchors[3].p, before.anchors[3].h_in - before.anchors[3].p));
        assert_eq!((sp.anchors[1].h_in, sp.anchors[3].h_out), (before.anchors[1].h_in, before.anchors[3].h_out));
        // The moved smooth point turns along the line through its neighbours.
        let a = sp.anchors[2];
        assert_eq!(a.kind, AnchorKind::Smooth);
        assert!(collinear(a.h_out - a.p, sp.anchors[3].p - sp.anchors[1].p) && collinear(a.h_in - a.p, a.h_out - a.p));
        assert!((a.h_out - a.p).dot(sp.anchors[3].p - sp.anchors[1].p) > 0.0);
        // A cusp keeps its handle directions; a moved end changes its one segment only.
        assert_eq!(sp.anchors[3].kind, AnchorKind::Corner);
        assert!(curvature_move(&mut sp, 3, pt(360.0, 170.0)));
        assert!(collinear(sp.anchors[3].h_out - sp.anchors[3].p, before.anchors[3].h_out - before.anchors[3].p));
        let mut end = before.clone();
        assert!(curvature_move(&mut end, 4, pt(470.0, 20.0)));
        assert_eq!(end.segment(2), before.segment(2));
        assert!(!curvature_move(&mut end, 5, pt(0.0, 0.0)));
    }

    #[test]
    fn moving_a_corner_without_handles_keeps_its_sides_straight() {
        let (d, id) = doc_with_rect();
        let mut sp = d.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
        assert!(curvature_move(&mut sp, 0, pt(80.0, 70.0)));
        assert!((0..sp.segment_count()).all(|i| sp.segment_is_line(i)));
        assert_eq!(sp.anchors[0], Anchor::corner(pt(80.0, 70.0)));
    }

    #[test]
    fn going_on_from_an_end_keeps_the_old_segments() {
        let before = wave();
        let mut sp = before.clone();
        curvature_extend(&mut sp, false, pt(550.0, 120.0), EndCurve::Keep);
        assert_eq!(sp.anchors.len(), 6);
        assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
        // The old end turns smooth, going on along its incoming handle, a third of the chord out.
        let e = sp.anchors[4];
        assert_eq!(e.kind, AnchorKind::Smooth);
        assert!(collinear(e.h_out - e.p, e.p - e.h_in) && (e.h_out - e.p).dot(e.p - e.h_in) > 0.0);
        assert!((e.h_out.distance(e.p) - e.p.distance(pt(550.0, 120.0)) / 3.0).abs() < 1e-9);
        assert_eq!(sp.anchors[5], Anchor::corner(pt(550.0, 120.0)));
        // The point placed then curves through its neighbours as a drawn path's points do.
        curvature_extend(&mut sp, false, pt(650.0, 60.0), EndCurve::Smooth);
        let drawn = catmull_rom(&[(pt(450.0, 60.0), false), (pt(550.0, 120.0), false), (pt(650.0, 60.0), false)], false);
        assert_eq!((sp.anchors[5].h_in, sp.anchors[5].h_out), (drawn.anchors[1].h_in, drawn.anchors[1].h_out));
        assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
        // A corner stays one.
        curvature_extend(&mut sp, false, pt(700.0, 100.0), EndCurve::Corner);
        assert!(!sp.anchors[6].has_in() && !sp.anchors[6].has_out());
        // From the start: the old segments come one later, and the new end is the first anchor.
        let mut sp = before.clone();
        curvature_extend(&mut sp, true, pt(-100.0, 50.0), EndCurve::Keep);
        assert_eq!(sp.anchors[0], Anchor::corner(pt(-100.0, 50.0)));
        assert!((0..4).all(|i| sp.segment(i + 1) == before.segment(i)));
        // A corner end without handles goes on along its segment's tangent.
        let old = sp.anchors[1];
        assert!(old.has_in() && !old.has_out());
        assert!(collinear(old.h_in - old.p, old.p - before.anchors[1].h_in) && (old.h_in - old.p).dot(old.p - before.anchors[1].h_in) > 0.0);
    }

    #[test]
    fn closing_keeps_the_old_segments() {
        let before = wave();
        let mut sp = before.clone();
        curvature_close(&mut sp, false, EndCurve::Keep);
        assert!(sp.closed && sp.anchors.len() == 5);
        assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
        // The first anchor gains an incoming handle along its tangent: no kink there.
        let f = sp.anchors[0];
        assert!(f.has_in() && collinear(f.p - f.h_in, before.anchors[1].h_in - f.p));
    }

    #[test]
    fn inserting_a_point_keeps_the_shape() {
        let before = wave();
        let mut sp = before.clone();
        assert_eq!(curvature_insert(&mut sp, 1, 0.3), Some(2));
        assert_eq!(sp.anchors.len(), 6);
        assert_eq!(sp.anchors[2].kind, AnchorKind::Smooth);
        assert!(lies_on(&sp, &before) && lies_on(&before, &sp));
        // On a straight side the point is smooth too, its handles along the side.
        let (d, id) = doc_with_rect();
        let rect = d.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
        let mut sp = rect.clone();
        let i = curvature_insert(&mut sp, 0, 0.5).unwrap();
        assert_eq!(sp.anchors[i].kind, AnchorKind::Smooth);
        assert!(sp.anchors[i].has_in() && sp.anchors[i].has_out());
        assert!(lies_on(&sp, &rect) && lies_on(&rect, &sp));
        assert_eq!(curvature_insert(&mut sp, 99, 0.5), None);
    }

    /// A document with one path, `sp`.
    fn doc_with(sp: SubPath) -> (Document, NodeId) {
        let mut d = Document::new(800.0, 600.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, PathData::single(sp), Appearance::default_art())).unwrap();
        (d, id)
    }

    /// Path `id` selected, anchor `current` of its first subpath direct-selected (or none).
    fn selected(id: NodeId, current: Option<usize>) -> Selection {
        let mut s = Selection::default();
        s.set([id]);
        if let Some(ai) = current {
            s.anchors.insert(id, [(0, ai)].into());
        }
        s
    }

    /// A context without snapping.
    fn quiet<'a>(d: &'a Document, s: &'a Selection, p: &'a crate::PaintDefaults) -> ToolContext<'a> {
        ToolContext { smart_guides: false, snap_to_point: false, ..cx(d, s, p) }
    }

    fn press(t: &mut CurvatureTool, cx: &ToolContext, x: f64, y: f64, alt: bool) -> Vec<Action> {
        let ev = PointerEvent::new(PointerKind::Down, x, y).with_mods(Mods { alt, ..Mods::default() });
        let mut a = t.pointer(cx, &ev);
        a.extend(t.pointer(cx, &PointerEvent::new(PointerKind::Up, x, y)));
        a
    }

    /// The op and params of a `path.curvatureEdit` action.
    fn edit_op(a: &Action) -> Option<(&str, &Value)> {
        match a {
            Action::Exec(c, v) | Action::Preview(c, v) if c == EDIT => Some((v["op"].as_str()?, v)),
            _ => None,
        }
    }

    #[test]
    fn a_press_on_a_point_selects_it_and_a_drag_moves_it() {
        let (d, id) = doc_with(wave());
        let (s, p) = (selected(id, None), paint());
        let cx = quiet(&d, &s, &p);
        let mut t = CurvatureTool::default();
        assert_eq!(t.cursor(&cx, pt(250.0, 81.0), Mods::default()), Cursor::Move);
        assert_eq!(t.cursor(&cx, pt(450.0, 61.0), Mods::default()), Cursor::PenContinue);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 251.0, 81.0));
        assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"}))]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 260.0, 140.0));
        assert_eq!(a[0], Action::Begin("Curvature".into()));
        let moved = edit_op(&a[1]).map(|(op, v)| (op, v["anchor"].clone(), v["x"].clone(), v["y"].clone()));
        assert_eq!(moved, Some(("move", json!(2), json!(260.0), json!(140.0))));
        assert!(t.busy());
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 260.0, 140.0)), vec![Action::Commit]);
        // A press without a drag only selects.
        assert_eq!(press(&mut t, &cx, 100.0, 0.0, false).len(), 1);
    }

    #[test]
    fn alt_click_or_double_click_toggles_and_delete_removes() {
        let (d, id) = doc_with(wave());
        let p = paint();
        let s = selected(id, None);
        let cx = quiet(&d, &s, &p);
        let mut t = CurvatureTool::default();
        let to = |a: &[Action]| a.iter().find(|a| matches!(a, Action::Exec(c, _) if c == "path.convertAnchor")).cloned();
        let a = press(&mut t, &cx, 100.0, 0.0, true);
        assert!(matches!(to(&a), Some(Action::Exec(_, v)) if v["anchor"] == 1 && v["to"] == "corner"), "{a:?}");
        assert_eq!(t.cursor(&cx, pt(100.0, 0.0), Mods { alt: true, ..Mods::default() }), Cursor::PenConvert);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 0.0, 100.0));
        assert!(matches!(to(&a), Some(Action::Exec(_, v)) if v["anchor"] == 0 && v["to"] == "smooth"), "{a:?}");
        // Delete is the tool's while a point is current, and removes it.
        assert!(!t.claims_key(&cx, ToolKey::Delete));
        let s = selected(id, Some(2));
        let cx = quiet(&d, &s, &p);
        assert!(t.claims_key(&cx, ToolKey::Backspace));
        assert_eq!(t.key(&cx, ToolKey::Delete, Mods::default()), vec![remove_anchor((id, 0, 2))]);
        // Esc lets go of the point.
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    }

    #[test]
    fn a_press_on_a_segment_adds_a_point_and_a_drag_moves_it_in_one_step() {
        let (d, id) = doc_with(wave());
        let (s, p) = (selected(id, None), paint());
        let cx = quiet(&d, &s, &p);
        let mut t = CurvatureTool::default();
        let on = wave().segment(1).eval(0.5);
        assert_eq!(t.cursor(&cx, on, Mods::default()), Cursor::PenAdd);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, on.x, on.y));
        assert_eq!(a[0], Action::Begin("Curvature".into()));
        assert!(
            matches!(edit_op(&a[1]), Some(("insert", v)) if v["segment"] == 1 && (v["t"].as_f64().unwrap() - 0.5).abs() < 1e-3 && v.get("x").is_none())
        );
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, on.x + 1.0, on.y + 40.0));
        assert!(matches!(edit_op(&a[0]), Some(("insert", v)) if v["segment"] == 1 && v["y"] == on.y + 40.0), "{a:?}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, on.x, on.y + 40.0)), vec![Action::Commit]);
    }

    #[test]
    fn clicks_go_on_from_a_current_end_and_the_other_end_closes() {
        let p = paint();
        let mut sp = wave();
        let mut t = CurvatureTool::default();
        let (d, id) = doc_with(sp.clone());
        let s = selected(id, Some(4));
        let a = press(&mut t, &quiet(&d, &s, &p), 550.0, 120.0, false);
        assert!(matches!(edit_op(&a[0]), Some(("extend", v)) if v["end"] == "end" && v["from"] == "keep" && v["x"] == 550.0), "{a:?}");
        // As the engine does it: the new end is current, and the next click curves through it.
        curvature_extend(&mut sp, false, pt(550.0, 120.0), EndCurve::Keep);
        let (d, id) = doc_with(sp.clone());
        let s = selected(id, Some(5));
        let cx = quiet(&d, &s, &p);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 650.0, 60.0));
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { .. })), "the rubber band shows");
        let a = press(&mut t, &cx, 650.0, 60.0, true);
        assert!(matches!(edit_op(&a[0]), Some(("extend", v)) if v["from"] == "smooth"), "{a:?}");
        curvature_extend(&mut sp, false, pt(650.0, 60.0), EndCurve::Smooth);
        let (d, id) = doc_with(sp.clone());
        let s = selected(id, Some(6));
        let cx = quiet(&d, &s, &p);
        // The Alt-clicked point stays a corner; a click on the first point closes.
        assert_eq!(t.cursor(&cx, pt(0.0, 100.0), Mods::default()), Cursor::PenClose);
        let a = press(&mut t, &cx, 0.0, 100.0, false);
        assert!(matches!(edit_op(&a[0]), Some(("close", v)) if v["from"] == "corner" && v["end"] == "end"), "{a:?}");
        // Any other end keeps its curve; the start goes on from the start.
        let s = selected(id, Some(0));
        let a = press(&mut t, &quiet(&d, &s, &p), -50.0, 150.0, false);
        assert!(matches!(edit_op(&a[0]), Some(("extend", v)) if v["end"] == "start" && v["from"] == "keep"), "{a:?}");
    }
}
