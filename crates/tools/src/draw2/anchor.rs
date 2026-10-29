//! Add Anchor Point (+), Delete Anchor Point (−), Anchor Point (Shift+C) and Scissors (C).
//!
//! Add: click a segment to insert an anchor without changing the shape (Alt = delete).
//! Delete: click an anchor to remove it, re-fitting the neighbouring curve (Alt = add).
//! Anchor Point: click a smooth anchor → corner; drag an anchor → pull out smooth handles; click a
//! handle end → retract that handle only (#2267); drag a handle → move it independently (Shift: at
//! 45° steps round its anchor); drag a segment → reshape the curve, a selected blend's spine's too,
//! straight or curved (#1111; Shift: its handles at right angles to the line between its anchors,
//! at one length). Shift held at the press on an anchor moves it instead, handles and all (Shift
//! still held: at 45° steps from where it was). Space held during a convert- or handle-drag moves
//! the anchor, handles and all, as while drawing with the Pen. Blend spine points and handles
//! follow the same conversions (#2544).
//! Scissors: click a path to split it there (Alt = add, as the Add Anchor Point tool does).

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::{hit_anchor, hit_segment, insert_anchor, remove_anchor};
use crate::direct::hit_handle;
use crate::guides::HandleSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
use vectorcraft_geom::constrain_angle_from;

#[derive(Clone, Copy, Debug)]
enum State {
    Idle,
    Convert {
        id: NodeId,
        si: usize,
        ai: usize,
        start: Point,
        began: bool,
    },
    Handle {
        id: NodeId,
        si: usize,
        ai: usize,
        out: bool,
        start: Point,
        began: bool,
    },
    /// Shift-dragging an anchor (`from`, where it was): it moves, handles and all.
    Move {
        id: NodeId,
        si: usize,
        ai: usize,
        from: Point,
        start: Point,
        began: bool,
    },
    /// Shift-dragging a selected blend's spine point: it moves, handles and all.
    SpineMove {
        id: NodeId,
        ai: usize,
        from: Point,
        start: Point,
        began: bool,
    },
    /// Dragging a selected blend's spine point into a symmetric smooth point.
    SpineConvert {
        id: NodeId,
        ai: usize,
        start: Point,
        began: bool,
    },
    /// Dragging or retracting one of a selected blend's spine handles.
    SpineHandle {
        id: NodeId,
        ai: usize,
        out: bool,
        start: Point,
        began: bool,
    },
    /// Bending a segment: a path's, or (`spine`) a selected blend's spine's.
    Reshape {
        id: NodeId,
        si: usize,
        seg: usize,
        t: f64,
        start: Point,
        began: bool,
        spine: bool,
    },
}

/// Gesture to resume after Space is released mid-drag (#3694).
#[derive(Clone, Copy, Debug)]
enum SpaceResume {
    Convert { id: NodeId, si: usize, ai: usize },
    Handle { id: NodeId, si: usize, ai: usize, out: bool },
    SpineConvert { id: NodeId, ai: usize },
    SpineHandle { id: NodeId, ai: usize, out: bool },
}

pub struct AnchorTool {
    id: &'static str,
    state: State,
    /// Smart guides of the handle being dragged, and what it snaps to.
    guides: Vec<Overlay>,
    snap: HandleSnap,
    /// Move offset while Shift-dragging an anchor or spine point (pointer, delta).
    measure: Option<(Point, f64, f64)>,
    /// While Space is held during a convert/handle drag, the shape gesture to return to.
    space_resume: Option<SpaceResume>,
}

impl AnchorTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "deleteAnchor" => "deleteAnchor",
            "anchorPoint" => "anchorPoint",
            "scissors" => "scissors",
            _ => "addAnchor",
        };
        Self { id, state: State::Idle, guides: vec![], snap: HandleSnap::default(), measure: None, space_resume: None }
    }

    fn anchor_pos(cx: &ToolContext, id: NodeId, si: usize, ai: usize) -> Option<Point> {
        cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai).map(|a| a.p))
    }

    /// Nearest point of this eligible spine, in document coordinates.
    fn spine_point(path: &vectorcraft_geom::PathData, p: Point, tol: f64) -> Option<usize> {
        path.anchors()
            .filter(|(si, _, a)| *si == 0 && a.p.distance(p) <= tol)
            .min_by(|a, b| a.2.p.distance(p).total_cmp(&b.2.p.distance(p)))
            .map(|(_, ai, _)| ai)
    }

    /// Nearest shown handle, only when closer than its own anchor (short handles).
    fn spine_handle(cx: &ToolContext, path: &vectorcraft_geom::PathData, p: Point, tol: f64) -> Option<(usize, bool)> {
        let anchors: Vec<_> = path.anchors().filter(|(si, _, _)| *si == 0).collect();
        if !cx.handles_multiple && anchors.len() > 1 {
            return None;
        }
        let mut best: Option<((usize, bool), f64)> = None;
        for (_, ai, a) in anchors {
            for (out, has, h) in [(true, a.has_out(), a.h_out), (false, a.has_in(), a.h_in)] {
                let d = h.distance(p);
                if has && d <= tol && d < a.p.distance(p) && best.is_none_or(|b| d < b.1) {
                    best = Some(((ai, out), d));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Preserve Path hit semantics; compare Path and Blend owners by selection and stacking.
    /// Hover and pointer-down share the same decision.
    fn press_target(cx: &ToolContext, p: Point) -> State {
        let (point, tol) = (cx.point_tol(), cx.pick_tol());
        let mut target = if let Some((id, si, ai, out)) = hit_handle(cx, p, point) {
            State::Handle { id, si, ai, out, start: p, began: false }
        } else if let Some((id, si, ai)) = hit_anchor(cx, p, point) {
            State::Convert { id, si, ai, start: p, began: false }
        } else if let Some((id, si, seg, t)) = hit_segment(cx, p, tol) {
            State::Reshape { id, si, seg, t, start: p, began: false, spine: false }
        } else {
            State::Idle
        };
        let rank = |state| match state {
            State::Idle => None,
            State::Move { id, .. } | State::SpineMove { id, .. } => Some((cx.selection.contains(id), cx.doc.index_path(id))),
            State::Convert { id, .. }
            | State::Handle { id, .. }
            | State::SpineConvert { id, .. }
            | State::SpineHandle { id, .. }
            | State::Reshape { id, .. } => Some((cx.selection.contains(id), cx.doc.index_path(id))),
        };
        for (id, path) in crate::direct::spines(cx).into_iter().filter(|(id, _)| cx.selection.contains(*id)) {
            let candidate = if let Some((ai, out)) = Self::spine_handle(cx, &path, p, point) {
                State::SpineHandle { id, ai, out, start: p, began: false }
            } else if let Some(ai) = Self::spine_point(&path, p, point) {
                State::SpineConvert { id, ai, start: p, began: false }
            } else if let Some((_, seg, t, _, dist)) = path.nearest(p) {
                if dist > tol {
                    continue;
                }
                State::Reshape { id, si: 0, seg, t, start: p, began: false, spine: true }
            } else {
                continue;
            };
            if rank(candidate) > rank(target) {
                target = candidate;
            }
        }
        target
    }

    /// Unlike Direct Selection, Anchor Point has no remembered focus after a
    /// tool switch. Show the handles on selected blends so they stay editable.
    fn spine_overlays(cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        for (id, path) in crate::direct::spines(cx).into_iter().filter(|(id, _)| cx.selection.contains(*id)) {
            let anchors: Vec<_> = path.anchors().filter(|(si, _, _)| *si == 0).collect();
            if !cx.handles_multiple && anchors.len() > 1 {
                continue;
            }
            let color = cx.doc.layer_color(id);
            for (_, _, a) in anchors {
                for h in [a.h_in, a.h_out].into_iter().filter(|h| h.distance(a.p) > 1e-6) {
                    out.push(Overlay::Line { a: a.p, b: h, color, dashed: false });
                    out.push(Overlay::Handle { p: h, color });
                }
            }
        }
        out
    }

    /// The press on an anchor (or a selected blend's spine point) with Shift held moves it
    /// instead of converting it. Any other press is kept.
    fn moving(cx: &ToolContext, st: State) -> State {
        match st {
            State::Convert { id, si, ai, start, .. } => {
                match cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai).map(|a| a.p)) {
                    Some(from) => State::Move { id, si, ai, from, start, began: false },
                    None => st,
                }
            }
            State::SpineConvert { id, ai, start, .. } => {
                match crate::direct::spines(cx)
                    .into_iter()
                    .find(|(blend, _)| *blend == id)
                    .and_then(|(_, path)| path.subpaths.first()?.anchors.get(ai).map(|a| a.p))
                {
                    Some(from) => State::SpineMove { id, ai, from, start, began: false },
                    None => st,
                }
            }
            _ => st,
        }
    }

    fn space_move_from(cx: &ToolContext, st: State, p: Point, began: bool) -> (State, Option<SpaceResume>) {
        match st {
            State::Convert { id, si, ai, .. } => match Self::anchor_pos(cx, id, si, ai) {
                Some(from) => (State::Move { id, si, ai, from, start: p, began }, Some(SpaceResume::Convert { id, si, ai })),
                None => (st, None),
            },
            State::Handle { id, si, ai, out, .. } => match Self::anchor_pos(cx, id, si, ai) {
                Some(from) => (State::Move { id, si, ai, from, start: p, began }, Some(SpaceResume::Handle { id, si, ai, out })),
                None => (st, None),
            },
            State::SpineConvert { id, ai, .. } => {
                match crate::direct::spines(cx)
                    .into_iter()
                    .find(|(blend, _)| *blend == id)
                    .and_then(|(_, path)| path.subpaths.first()?.anchors.get(ai).map(|a| a.p))
                {
                    Some(from) => (State::SpineMove { id, ai, from, start: p, began }, Some(SpaceResume::SpineConvert { id, ai })),
                    None => (st, None),
                }
            }
            State::SpineHandle { id, ai, out, .. } => {
                match crate::direct::spines(cx)
                    .into_iter()
                    .find(|(blend, _)| *blend == id)
                    .and_then(|(_, path)| path.subpaths.first()?.anchors.get(ai).map(|a| a.p))
                {
                    Some(from) => (State::SpineMove { id, ai, from, start: p, began }, Some(SpaceResume::SpineHandle { id, ai, out })),
                    None => (st, None),
                }
            }
            other => (other, None),
        }
    }

    fn resume_shape(p: Point, resume: SpaceResume, began: bool) -> State {
        match resume {
            SpaceResume::Convert { id, si, ai } => State::Convert { id, si, ai, start: p, began },
            SpaceResume::Handle { id, si, ai, out } => State::Handle { id, si, ai, out, start: p, began },
            SpaceResume::SpineConvert { id, ai } => State::SpineConvert { id, ai, start: p, began },
            SpaceResume::SpineHandle { id, ai, out } => State::SpineHandle { id, ai, out, start: p, began },
        }
    }

    /// How far a Shift-dragged anchor has gone: Shift still held keeps it at 45° steps.
    fn move_delta(cx: &ToolContext, start: Point, p: Point, shift: bool) -> vectorcraft_geom::Vec2 {
        if shift { constrain_angle_from(p - start, 45.0, cx.constrain_angle) } else { p - start }
    }

    fn add(cx: &ToolContext, p: Point) -> Vec<Action> {
        hit_segment(cx, p, cx.pick_tol()).map(insert_anchor).into_iter().collect()
    }

    fn delete(cx: &ToolContext, p: Point) -> Vec<Action> {
        hit_anchor(cx, p, cx.point_tol()).map(remove_anchor).into_iter().collect()
    }
}

impl Tool for AnchorTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
    /// Escape during an Anchor Point drag (an anchor pulled into a smooth point or moved, a
    /// handle, a segment, a blend spine's point, handle or segment) cancels it: what it moved goes
    /// back and the release does nothing. Escape after a press that has not moved yet drops the
    /// press, so its release converts nothing.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key != ToolKey::Escape || matches!(self.state, State::Idle) {
            return vec![];
        }
        let began = matches!(
            self.state,
            State::Convert { began: true, .. }
                | State::Move { began: true, .. }
                | State::SpineMove { began: true, .. }
                | State::Handle { began: true, .. }
                | State::SpineConvert { began: true, .. }
                | State::SpineHandle { began: true, .. }
                | State::Reshape { began: true, .. }
        );
        self.state = State::Idle;
        self.guides.clear();
        self.measure = None;
        self.space_resume = None;
        if began { vec![Action::Cancel] } else { vec![] }
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        // Anchors and handle ends are picked from a little further than segments.
        let (tol, point) = (cx.pick_tol(), cx.point_tol());
        match (self.id, ev.kind) {
            ("addAnchor", PointerKind::Down) => {
                if ev.mods.alt {
                    Self::delete(cx, p)
                } else {
                    Self::add(cx, p)
                }
            }
            ("deleteAnchor", PointerKind::Down) => {
                if ev.mods.alt {
                    Self::add(cx, p)
                } else {
                    Self::delete(cx, p)
                }
            }
            ("scissors", PointerKind::Down) => {
                if ev.mods.alt {
                    return Self::add(cx, p);
                }
                if let Some((id, si, ai)) = hit_anchor(cx, p, point) {
                    return vec![Action::Exec("path.split".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))];
                }
                match hit_segment(cx, p, tol) {
                    Some((id, si, seg, t)) => vec![Action::Exec("path.split".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))],
                    None => vec![],
                }
            }
            ("anchorPoint", PointerKind::Down) => {
                self.space_resume = None;
                self.state = Self::press_target(cx, p);
                if ev.mods.shift {
                    self.state = Self::moving(cx, self.state);
                }
                if matches!(self.state, State::Handle { .. }) {
                    self.snap = HandleSnap::default();
                }
                vec![]
            }
            ("anchorPoint", PointerKind::Drag) => {
                let mut out = vec![];
                let (began, far) = match self.state {
                    State::Idle => return out,
                    State::Convert { began, start, .. }
                    | State::Move { began, start, .. }
                    | State::SpineMove { began, start, .. }
                    | State::Handle { began, start, .. }
                    | State::SpineConvert { began, start, .. }
                    | State::SpineHandle { began, start, .. }
                    | State::Reshape { began, start, .. } => (began, p.distance(start) >= cx.tol(2.0)),
                };
                if !began {
                    if !far {
                        return out;
                    }
                    out.push(Action::Begin(
                        match self.state {
                            State::Convert { .. } | State::SpineConvert { .. } => "Convert Anchor Point",
                            State::Move { .. } | State::SpineMove { .. } => "Move",
                            _ => "Reshape",
                        }
                        .into(),
                    ));
                    if let State::Convert { began, .. }
                    | State::Move { began, .. }
                    | State::SpineMove { began, .. }
                    | State::Handle { began, .. }
                    | State::SpineConvert { began, .. }
                    | State::SpineHandle { began, .. }
                    | State::Reshape { began, .. } = &mut self.state
                    {
                        *began = true;
                    }
                }
                if ev.mods.space {
                    if self.space_resume.is_none()
                        && matches!(self.state, State::Convert { .. } | State::Handle { .. } | State::SpineConvert { .. } | State::SpineHandle { .. })
                    {
                        let was_began = match self.state {
                            State::Convert { began, .. }
                            | State::Handle { began, .. }
                            | State::SpineConvert { began, .. }
                            | State::SpineHandle { began, .. } => began,
                            _ => false,
                        };
                        let (st, resume) = Self::space_move_from(cx, self.state, p, was_began);
                        if let Some(r) = resume {
                            self.state = st;
                            self.space_resume = Some(r);
                        }
                    }
                } else if let Some(resume) = self.space_resume.take() {
                    let was_began = matches!(self.state, State::Move { began: true, .. } | State::SpineMove { began: true, .. });
                    self.state = Self::resume_shape(p, resume, was_began);
                }
                let preview = match self.state {
                    State::Idle => return out,
                    State::Convert { id, si, ai, .. } => Action::Preview(
                        "path.convertAnchor".into(),
                        json!({"id": id.0, "subpath": si, "anchor": ai, "to": "smooth", "x": p.x, "y": p.y}),
                    ),
                    State::Move { id, si, ai, from, start, .. } => {
                        let d = Self::move_delta(cx, start, p, ev.mods.shift);
                        self.measure = Some((p, d.x, d.y));
                        let q = from + d;
                        Action::Preview("path.moveAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai, "x": q.x, "y": q.y}))
                    }
                    State::SpineMove { id, ai, from, start, .. } => {
                        let d = Self::move_delta(cx, start, p, ev.mods.shift);
                        self.measure = Some((p, d.x, d.y));
                        let q = from + d;
                        Action::Preview("object.blend.spine.moveAnchor".into(), json!({"id": id.0, "anchor": ai, "x": q.x, "y": q.y}))
                    }
                    State::SpineConvert { id, ai, .. } => Action::Preview(
                        "object.blend.spine.convertAnchor".into(),
                        json!({"id": id.0, "anchor": ai, "to": "smooth", "x": p.x, "y": p.y}),
                    ),
                    State::SpineHandle { id, ai, out, .. } => {
                        // Shift keeps the handle at 45-degree steps round its anchor, as for
                        // ordinary Anchor Point handles. The spine anchor is a blend point, not
                        // a path node, so constrain it directly instead of HandleSnap::snap.
                        let q = if ev.mods.shift {
                            match crate::direct::spines(cx)
                                .into_iter()
                                .find(|(blend, _)| *blend == id)
                                .and_then(|(_, path)| path.subpaths.first()?.anchors.get(ai).copied())
                            {
                                Some(a) => a.p + constrain_angle_from(p - a.p, 45.0, cx.constrain_angle),
                                None => p,
                            }
                        } else {
                            p
                        };
                        Action::Preview(
                            "object.blend.spine.moveAnchor".into(),
                            json!({"id": id.0, "anchor": ai, "handle": if out { "out" } else { "in" }, "x": q.x, "y": q.y, "independent": true}),
                        )
                    }
                    State::Handle { id, si, ai, out, .. } => {
                        let (q, guides) = self.snap.snap(cx, (id, si, ai), p, ev.mods.shift);
                        self.guides = guides;
                        Action::Preview(
                            "path.setHandle".into(),
                            json!({"id": id.0, "subpath": si, "anchor": ai, "which": if out { "out" } else { "in" }, "x": q.x, "y": q.y, "independent": true}),
                        )
                    }
                    State::Reshape { id, seg, t, start, spine: true, .. } => Action::Preview(
                        "object.blend.spine.reshapeSegment".into(),
                        json!({"id": id.0, "segment": seg, "t": t, "dx": p.x - start.x, "dy": p.y - start.y, "perpendicular": ev.mods.shift}),
                    ),
                    State::Reshape { id, si, seg, t, start, .. } => Action::Preview(
                        "path.reshapeSegment".into(),
                        json!({"id": id.0, "subpath": si, "segment": seg, "t": t, "dx": p.x - start.x, "dy": p.y - start.y, "perpendicular": ev.mods.shift}),
                    ),
                };
                out.push(preview);
                out
            }
            ("anchorPoint", PointerKind::Up) => {
                self.guides.clear();
                self.measure = None;
                self.space_resume = None;
                let st = std::mem::replace(&mut self.state, State::Idle);
                match st {
                    State::Convert { began: true, .. }
                    | State::Move { began: true, .. }
                    | State::SpineMove { began: true, .. }
                    | State::Handle { began: true, .. }
                    | State::SpineConvert { began: true, .. }
                    | State::SpineHandle { began: true, .. }
                    | State::Reshape { began: true, .. } => {
                        vec![Action::Commit]
                    }
                    State::Convert { id, si, ai, began: false, .. } => {
                        let has_handles = cx
                            .doc
                            .node(id)
                            .and_then(|n| n.path_data())
                            .and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai))
                            .is_some_and(|a| a.has_in() || a.has_out());
                        if has_handles {
                            vec![Action::Exec("path.convertAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai, "to": "corner"}))]
                        } else {
                            vec![]
                        }
                    }
                    State::SpineConvert { id, ai, began: false, .. } => {
                        let has_handles = crate::direct::spines(cx)
                            .into_iter()
                            .find(|(blend, _)| *blend == id)
                            .and_then(|(_, path)| path.subpaths.first()?.anchors.get(ai).copied())
                            .is_some_and(|a| a.has_in() || a.has_out());
                        if has_handles {
                            vec![Action::Exec("object.blend.spine.convertAnchor".into(), json!({"id": id.0, "anchor": ai, "to": "corner"}))]
                        } else {
                            vec![]
                        }
                    }
                    State::SpineHandle { id, ai, out, began: false, .. } => {
                        let Some(a) = crate::direct::spines(cx)
                            .into_iter()
                            .find(|(blend, _)| *blend == id)
                            .and_then(|(_, path)| path.subpaths.first()?.anchors.get(ai).copied())
                        else {
                            return vec![];
                        };
                        vec![Action::Exec(
                            "object.blend.spine.moveAnchor".into(),
                            json!({"id": id.0, "anchor": ai, "handle": if out { "out" } else { "in" }, "x": a.p.x, "y": a.p.y, "independent": true}),
                        )]
                    }
                    State::Handle { id, si, ai, out, began: false, .. } => {
                        let Some(a) = cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai)) else {
                            return vec![];
                        };
                        vec![Action::Exec(
                            "path.setHandle".into(),
                            json!({"id": id.0, "subpath": si, "anchor": ai, "which": if out { "out" } else { "in" }, "x": a.p.x, "y": a.p.y, "independent": true}),
                        )]
                    }
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.measure = None;
        let st = std::mem::replace(&mut self.state, State::Idle);
        match st {
            State::Convert { began: true, .. }
            | State::Move { began: true, .. }
            | State::SpineMove { began: true, .. }
            | State::Handle { began: true, .. }
            | State::SpineConvert { began: true, .. }
            | State::SpineHandle { began: true, .. }
            | State::Reshape { began: true, .. } => vec![Action::Commit],
            _ => vec![],
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = if self.id == "anchorPoint" { Self::spine_overlays(cx) } else { vec![] };
        if matches!(self.state, State::Handle { .. }) {
            out.extend(self.guides.iter().cloned());
        }
        if cx.measurement_labels
            && let Some((p, dx, dy)) = self.measure
        {
            out.push(Overlay::Measure { p, text: cx.offset_label(dx, dy) });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        match self.id {
            "addAnchor" if !m.alt => Cursor::PenAdd,
            "addAnchor" => Cursor::PenDelete,
            "deleteAnchor" if !m.alt => Cursor::PenDelete,
            "deleteAnchor" => Cursor::PenAdd,
            "scissors" if m.alt => Cursor::PenAdd,
            "scissors" => {
                if hit_segment(cx, p, cx.pick_tol()).is_some() {
                    Cursor::Scissors
                } else {
                    Cursor::ScissorsNotAllowed
                }
            }
            _ => match self.state {
                State::Reshape { .. } => Cursor::ReshapeSegment,
                // A Shift-drag moving an anchor shows the filled arrow, as Direct Selection's does.
                State::Move { began: true, .. } | State::SpineMove { began: true, .. } => Cursor::Arrow,
                State::Move { .. } | State::SpineMove { .. } => Cursor::AnchorPoint,
                State::Idle => press_cursor(cx, p),
                State::Convert { .. } | State::Handle { .. } | State::SpineConvert { .. } | State::SpineHandle { .. } => Cursor::AnchorPoint,
            },
        }
    }
}

/// The Anchor Point tool's cursor for a press at `p`: the Reshape Segment cursor where it takes a
/// segment (a path's or a selected blend's spine's), else its caret. Handles and anchors win
/// over the segments through them, as the press has it.
pub(crate) fn press_cursor(cx: &ToolContext, p: Point) -> Cursor {
    if matches!(AnchorTool::press_target(cx, p), State::Reshape { .. }) { Cursor::ReshapeSegment } else { Cursor::AnchorPoint }
}

#[cfg(test)]
mod tests;
