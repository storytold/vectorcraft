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
//! double-click toggles it between smooth and corner and Backspace/Delete removes it, the curve
//! where it was then following the remaining points ([`curvature_remove`]). A click on
//! a segment adds a smooth point there without changing the shape ([`curvature_insert`]), and a
//! drag moves the new point at once. While the current point is an end of an open path, clicks go
//! on from it ([`curvature_extend`]): the old segments keep their shape, the new ones curve
//! through the new points, and a click on the path's other end closes it. Each edit is one
//! `path.curvatureEdit`, `path.convertAnchor` or `path.removeAnchor` (one undo step).

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, Selection};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, Point, SubPath, Vec2};

use super::{FEEDBACK, anchor_in, curvature_spline, editable_paths, end_handle, segment_in, smooth_handles};
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
    /// Smooth through its neighbours, as [`curvature_spline`] curves a point (a point the Curvature
    /// tool placed: its segment re-curves).
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
/// change. A smooth anchor takes the Curvature tool's own handles ([`smooth_handles`]: along the
/// circle through it and its neighbours; at an end of an open path the one [`end_handle`] into the
/// curve). The neighbours' handles on those segments, and a corner's own
/// handles, keep their directions, their lengths scaling with their segment's chord, so the
/// segments beyond don't move and a corner without handles keeps none.
///
/// The neighbours' tangents are not re-curved: a selected path may come from the Pen or any other
/// tool, and turning its smooth points would reshape what the user drew. The path the tool is
/// drawing ([`Grab::Drawn`]) is rebuilt by [`curvature_spline`] from its points, so there the
/// neighbours do turn with the dragged point. False when there is no anchor `i`.
pub fn curvature_move(sp: &mut SubPath, i: usize, q: Point) -> bool {
    let Some(&a) = sp.anchors.get(i) else { return false };
    let mut own = [a.h_in - a.p, a.h_out - a.p];
    // Each neighbour, its handle on the segment to `a` scaled.
    let mut ends: [Option<Anchor>; 2] = [None; 2];
    for (side, j) in neighbours(sp, i).into_iter().enumerate() {
        let Some(b) = j.and_then(|j| sp.anchors.get_mut(j)) else { continue };
        let old = a.p.distance(b.p);
        let s = if old > 1e-9 { q.distance(b.p) / old } else { 1.0 };
        let h = if side == 0 { &mut b.h_out } else { &mut b.h_in };
        *h = b.p + (*h - b.p) * s;
        own[side] *= s;
        ends[side] = Some(*b);
    }
    let mut m = Anchor { p: q, h_in: q + own[0], h_out: q + own[1], kind: a.kind };
    if a.kind == AnchorKind::Smooth {
        match ends {
            [Some(prev), Some(next)] => {
                if let Some((h_in, h_out)) = smooth_handles(prev.p, q, next.p) {
                    (m.h_in, m.h_out) = (h_in, h_out);
                }
            }
            // An end of an open path has one segment: its handle only, and a corner like the
            // ends `curvature_spline` makes.
            [None, Some(next)] => m = Anchor { h_out: end_handle(q, &next, true).unwrap_or(q), h_in: q, kind: AnchorKind::Corner, ..m },
            [Some(prev), None] => m = Anchor { h_in: end_handle(q, &prev, false).unwrap_or(q), h_out: q, kind: AnchorKind::Corner, ..m },
            [None, None] => {}
        }
    }
    if let Some(x) = sp.anchors.get_mut(i) {
        *x = m;
    }
    true
}

/// Add a smooth anchor at `t` on segment `seg` of `sp` without changing its shape: the new anchor's
/// index. On a straight segment its handles lie along it, each a third of its own part of the
/// segment ([`curvature_smooth`]: a line is a circle's limit), so a drag bends the line evenly; a drag then moves it as a
/// smooth point ([`curvature_move`]). None when there is no such segment.
pub fn curvature_insert(sp: &mut SubPath, seg: usize, t: f64) -> Option<usize> {
    if seg >= sp.segment_count() {
        return None;
    }
    let line = sp.segment_is_line(seg);
    let i = sp.insert_anchor(seg, if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 });
    if line {
        curvature_smooth(sp, i);
    }
    Some(i)
}

/// Make anchor `i` of `sp` smooth as the Curvature tool curves a point: [`smooth_handles`] from its
/// neighbours, or at an end of an open path the one [`end_handle`] into the curve (no outer handle;
/// it stays a corner, as the ends [`curvature_spline`] makes). False, and `sp` unchanged, when
/// there is no anchor `i` or no direction: the neighbours coincide with it or fold back on it, or
/// an end's neighbour has no handle to take a tangent from.
pub fn curvature_smooth(sp: &mut SubPath, i: usize) -> bool {
    let at = |j: Option<usize>| j.and_then(|j| sp.anchors.get(j)).copied();
    let [prev, next] = neighbours(sp, i).map(at);
    let Some(a) = sp.anchors.get(i).copied() else { return false };
    let m = match (prev, next) {
        (Some(b), Some(c)) => smooth_handles(b.p, a.p, c.p).map(|(h_in, h_out)| Anchor { p: a.p, h_in, h_out, kind: AnchorKind::Smooth }),
        (None, Some(c)) => end_handle(a.p, &c, true).map(|h_out| Anchor { h_in: a.p, h_out, ..Anchor::corner(a.p) }),
        (Some(b), None) => end_handle(a.p, &b, false).map(|h_in| Anchor { h_in, h_out: a.p, ..Anchor::corner(a.p) }),
        (None, None) => None,
    };
    match (m, sp.anchors.get_mut(i)) {
        (Some(m), Some(x)) => {
            *x = m;
            true
        }
        _ => false,
    }
}

/// Remove anchor `i` of `sp` as the Curvature tool deletes a point: the curve where it was follows
/// the remaining points. Each neighbor that curves by the tool's rule (a smooth anchor, or an end
/// of an open path with a handle into its segment) takes new handles from its new neighbors
/// ([`smooth_handles`]; at an end, the one [`end_handle`] into the curve), and so does an end beyond
/// one of them whose handle mirrors its tangent. A corner neighbor keeps its handles' directions,
/// the one on the new segment scaled with its chord, as [`curvature_move`] scales them. With two
/// anchors left the subpath is open, and those that curve by the tool's rule lose their handles, as
/// [`curvature_spline`] draws two points as a line; a corner keeps its handles. On a path the tool
/// drew, the result is the path it draws through the remaining points, except that an Alt-clicked
/// end beside a corner without handles curves (the path stores it like any end there). False, and
/// `sp` unchanged, when there is no anchor `i`.
pub fn curvature_remove(sp: &mut SubPath, i: usize) -> bool {
    let n = sp.anchors.len();
    let Some(a) = sp.anchors.get(i).copied() else { return false };
    let closed = sp.closed;
    let is_end = |j: usize| !closed && (j == 0 || j + 1 == n);
    // An end without a handle beside a corner without handles is stored the same whether it was
    // Alt-clicked or not ([`curvature_spline`] gives it no handle either way), so it curves, as an
    // end the tool places does by default. Beside a tangent, a missing handle means a corner.
    let flags: Vec<bool> = sp
        .anchors
        .iter()
        .enumerate()
        .map(|(j, b)| {
            if !is_end(j) {
                return b.kind == AnchorKind::Smooth;
            }
            let near = sp.anchors.get(if j == 0 { 1 } else { n.saturating_sub(2) });
            b.has_in() || b.has_out() || near.is_none_or(|q| end_handle(b.p, q, j == 0).is_none())
        })
        .collect();
    let curved = |j: usize| flags.get(j).copied().unwrap_or(false);
    let [prev, next] = neighbours(sp, i);
    // A corner on both sides of the removed point: its handle on the new segment scales with it.
    if let (Some(pi), Some(ni)) = (prev, next)
        && pi != ni
    {
        for (j, other, out) in [(pi, ni, true), (ni, pi, false)] {
            let to = sp.anchors.get(other).map(|o| o.p);
            if curved(j) {
                continue;
            }
            let Some((b, to)) = sp.anchors.get_mut(j).zip(to) else { continue };
            let old = b.p.distance(a.p);
            let s = if old > 1e-9 { b.p.distance(to) / old } else { 1.0 };
            let h = if out { &mut b.h_out } else { &mut b.h_in };
            *h = b.p + (*h - b.p) * s;
        }
    }
    let mut again: Vec<usize> = [prev, next].into_iter().flatten().filter(|&j| curved(j)).collect();
    // An end beside a smooth neighbor mirrors that neighbor's tangent, which changes.
    for j in again.clone() {
        if !is_end(j) {
            again.extend(neighbours(sp, j).into_iter().flatten().filter(|&k| k != i && is_end(k) && curved(k)));
        }
    }
    sp.anchors.remove(i);
    let mut again: Vec<usize> = again.into_iter().map(|j| if j > i { j - 1 } else { j }).collect();
    again.sort_unstable();
    again.dedup();
    let m = sp.anchors.len();
    if m < 3 {
        sp.closed = false;
    }
    if !sp.closed {
        // A new end has nothing beyond it.
        if let Some(e) = sp.anchors.first_mut() {
            e.h_in = e.p;
        }
        if let Some(e) = sp.anchors.last_mut() {
            e.h_out = e.p;
        }
    }
    let end = |j: usize| !sp.closed && (j == 0 || j + 1 == m);
    let (ends, inner): (Vec<usize>, Vec<usize>) = again.into_iter().partition(|&j| end(j));
    // The inner anchors first: an end's handle mirrors its neighbor's tangent.
    for j in inner {
        let [b, c] = neighbours(sp, j).map(|k| k.and_then(|k| sp.anchors.get(k)).map(|x| x.p));
        let Some(x) = sp.anchors.get(j).map(|x| x.p) else { continue };
        let fresh = match (b, c) {
            (Some(b), Some(c)) => smooth_handles(b, x, c).map(|(h_in, h_out)| Anchor { p: x, h_in, h_out, kind: AnchorKind::Smooth }),
            _ => None,
        };
        if let Some(y) = sp.anchors.get_mut(j) {
            *y = fresh.unwrap_or_else(|| Anchor::corner(x));
        }
    }
    for j in ends {
        let first = j == 0;
        let near = if first { sp.anchors.get(1) } else { j.checked_sub(1).and_then(|k| sp.anchors.get(k)) }.copied();
        let Some(x) = sp.anchors.get(j).map(|x| x.p) else { continue };
        // Two points are a line.
        let h = near.filter(|_| m >= 3).and_then(|q| end_handle(x, &q, first)).unwrap_or(x);
        if let Some(y) = sp.anchors.get_mut(j) {
            *y = if first { Anchor { h_out: h, ..Anchor::corner(x) } } else { Anchor { h_in: h, ..Anchor::corner(x) } };
        }
    }
    true
}

/// Go on from an end of the open subpath `sp` (its start with `start`) to a new end at `p`; the old
/// end bends into the new segment as `how` says. The new end has no handle on the outer side and,
/// unless it is a `corner` (Alt-clicked), the one [`end_handle`] into the curve, so its segment
/// curves as the last one of a path drawn from scratch does.
pub fn curvature_extend(sp: &mut SubPath, start: bool, p: Point, how: EndCurve, corner: bool) {
    if start {
        sp.reverse();
    }
    bend_last(sp, p, how);
    let mut end = Anchor::corner(p);
    if let (false, Some(old)) = (corner, sp.anchors.last()) {
        end.h_in = end_handle(p, old, false).unwrap_or(p);
    }
    sp.anchors.push(end);
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
        EndCurve::Smooth => match prev.and_then(|b| smooth_handles(b.p, e.p, to)) {
            Some((h_in, h_out)) => *e = Anchor { p: e.p, h_in, h_out, kind: AnchorKind::Smooth },
            // Nothing before it, or no direction: the new point has no handle yet for an end
            // handle to aim at, so the segment stays straight (two points are a line).
            None => e.retract(),
        },
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

/// Delete anchor `(id, si, ai)` of a selected path, the curve where it was following the remaining
/// points ([`curvature_remove`]).
fn delete_point((id, si, ai): (NodeId, usize, usize)) -> Action {
    Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai, "curvature": true}))
}

/// Toggle anchor `(id, si, ai)` of a selected path between smooth and corner. An end of an open
/// path counts as smooth while it has a handle into its segment (the ends the tool curves are
/// corners with one handle).
fn toggle(cx: &ToolContext, (id, si, ai): (NodeId, usize, usize)) -> Option<Action> {
    let sp = subpath(cx, id, si)?;
    let a = sp.anchors.get(ai)?;
    let open_end = !sp.closed && (ai == 0 || ai + 1 == sp.anchors.len());
    let smooth = if open_end { a.has_in() || a.has_out() } else { a.kind == AnchorKind::Smooth };
    let to = if smooth { "corner" } else { "smooth" };
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
            Some(id) => self.pts.last().map(|(p, _)| (id, Leave::segment(cx, *p, false).without_contact())),
            None => current_end(cx).map(|e| (e.id, Leave::segment(cx, e.p, false).without_contact())),
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
            let q = self.snap.press(cx, p, &[e.id], Some(&Leave::segment(cx, e.p, false).without_contact()));
            let how = self.how(&e);
            self.placed = Some(Placed { id: e.id, si: e.si, start: e.start, anchors: e.anchors + 1, corner: ev.mods.alt });
            let v = json!({"id": e.id.0, "subpath": e.si, "op": "extend", "end": e.name(), "x": q.x, "y": q.y, "from": how.name(), "corner": ev.mods.alt});
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
                    return current_point(cx).map(delete_point).into_iter().collect();
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
                curvature_extend(&mut tail, false, h, self.how(&e), false);
                tail.to_bezpath_into(&mut bp);
                o.push(Overlay::Path { path: bp, color: FEEDBACK, width: 1.0, dashed: false });
                if cx.measurement_labels && e.p.distance(h) > 1e-9 {
                    o.push(Overlay::Measure { p: h + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.segment_measure_label(e.p, h) });
                }
            }
            return o;
        }
        if band
            && !self.closed
            && let Some(h) = self.hover
        {
            let mut pts = self.pts.clone();
            pts.push((h, false));
            curvature_spline(&pts, false).to_bezpath_into(&mut bp);
            o.push(Overlay::Path { path: bp, color: FEEDBACK, width: 1.0, dashed: false });
            if let Some((from, _)) = self.pts.last()
                && cx.measurement_labels
                && from.distance(h) > 1e-9
            {
                o.push(Overlay::Measure { p: h + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.segment_measure_label(*from, h) });
            }
        }
        for (i, (p, _)) in self.pts.iter().enumerate() {
            o.push(Overlay::CurvePoint { p: *p, color: FEEDBACK, filled: Some(i) == self.current, size: 6.0 });
        }
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if self.active(cx).is_some() {
            return match self.hit_point(cx, p) {
                Some(0) if !self.closed && self.pts.len() >= 3 => Cursor::PenClose,
                Some(_) if m.alt => Cursor::PenConvert,
                Some(_) => Cursor::Move,
                None => Cursor::Curvature,
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
        Cursor::Curvature
    }
}

#[cfg(test)]
mod tests;
