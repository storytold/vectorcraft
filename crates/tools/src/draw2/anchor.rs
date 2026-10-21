//! Add Anchor Point (+), Delete Anchor Point (−), Anchor Point (Shift+C) and Scissors (C).
//!
//! Add: click a segment to insert an anchor without changing the shape (Alt = delete).
//! Delete: click an anchor to remove it, re-fitting the neighbouring curve (Alt = add).
//! Anchor Point: click a smooth anchor → corner; drag an anchor → pull out smooth handles; click a
//! handle end → retract that handle only (#2267); drag a handle → move it independently (Shift: at
//! 45° steps round its anchor); drag a segment → reshape the curve, a selected blend's spine's too,
//! straight or curved (#1111; Shift: its handles at right angles to the line between its anchors,
//! at one length). Shift held at the press on an anchor moves it instead, handles and all (Shift
//! still held: at 45° steps from where it was). Blend spine points and handles follow the same
//! conversions (#2544).
//! Scissors: click a path to split it there.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::{hit_anchor, hit_segment, insert_anchor, remove_anchor};
use crate::direct::hit_handle;
use crate::guides::HandleSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};
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

pub struct AnchorTool {
    id: &'static str,
    state: State,
    /// Smart guides of the handle being dragged, and what it snaps to.
    guides: Vec<Overlay>,
    snap: HandleSnap,
    /// Move offset while Shift-dragging an anchor or spine point (pointer, delta).
    measure: Option<(Point, f64, f64)>,
}

impl AnchorTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "deleteAnchor" => "deleteAnchor",
            "anchorPoint" => "anchorPoint",
            "scissors" => "scissors",
            _ => "addAnchor",
        };
        Self { id, state: State::Idle, guides: vec![], snap: HandleSnap::default(), measure: None }
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
                if let Some((id, si, ai)) = hit_anchor(cx, p, point) {
                    return vec![Action::Exec("path.split".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))];
                }
                match hit_segment(cx, p, tol) {
                    Some((id, si, seg, t)) => vec![Action::Exec("path.split".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))],
                    None => vec![],
                }
            }
            ("anchorPoint", PointerKind::Down) => {
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
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;
    use vectorcraft_geom::{Anchor, ParamCurve};

    /// The Anchor Point tool has its own caret cursor, not the Pen's start-a-path one, and over a
    /// segment the Reshape Segment cursor, kept while the segment is dragged.
    #[test]
    fn anchor_point_tool_shows_its_caret_and_reshape_over_segments() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = AnchorTool::new("anchorPoint");
        let none = Mods::default();
        for at in [Point::new(20.0, 20.0), Point::new(200.0, 100.0)] {
            assert_eq!(t.cursor(&cx, at, none), Cursor::AnchorPoint, "{at:?}");
        }
        assert_eq!(t.cursor(&cx, Point::new(150.0, 101.0), none), Cursor::ReshapeSegment);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 140.0));
        assert_eq!(t.cursor(&cx, Point::new(150.0, 140.0), none), Cursor::ReshapeSegment);
    }

    /// Selected three-anchor blend spine; the end anchors stay pinned to the
    /// centres of the two key objects.
    fn blend_spine_doc(smooth: bool) -> (vectorcraft_doc::Document, NodeId, Selection) {
        let (mut d, _) = doc_with_rect();
        let layer = d.layers[0].id;
        let (id, first, last) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let key = |id, x| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art()))
        };
        let mut path = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(160.0, 310.0), Point::new(210.0, 310.0)], false);
        if smooth {
            path.anchors[1] = Anchor::smooth(Point::new(160.0, 310.0), Point::new(190.0, 330.0));
        }
        let spec =
            vectorcraft_doc::live::BlendSpec { spine: Some(vectorcraft_geom::PathData::single(path)), key_anchors: vec![0, 2], ..Default::default() };
        let blend = vectorcraft_doc::Node::new(
            id,
            vectorcraft_doc::NodeKind::Blend { children: vec![key(first, 100.0), key(last, 200.0)], spec: Box::new(spec) },
        );
        d.insert(Some(layer), 1, blend).unwrap();
        let mut selected = Selection::default();
        selected.set([id]);
        (d, id, selected)
    }

    /// Owner priority applies to anchors, visible handles and segments alike, regardless
    /// of selection insertion order; cursor and emitted command must agree.
    #[test]
    fn overlapping_path_and_blend_follow_selection_then_stacking() {
        use vectorcraft_doc::{Appearance, Node};
        use vectorcraft_geom::{PathData, SubPath};
        for path_on_top in [false, true] {
            for path_selected in [false, true] {
                for blend_first in [false, true] {
                    let (d, blend, sel) = blend_spine_doc(true);
                    let paint = paint();
                    let spine = crate::direct::spines(&cx(&d, &sel, &paint)).pop().unwrap().1;
                    let curve = spine.subpaths[0].segment(1).eval(0.5);
                    // Point, handle and segment overlaps, including different cursor kinds.
                    for (at, path_is_segment) in [
                        (Point::new(160.0, 310.0), false),
                        (Point::new(190.0, 330.0), false),
                        (Point::new(160.0, 310.0), true),
                        (Point::new(190.0, 330.0), true),
                        (curve, false),
                        (curve, true),
                    ] {
                        let (mut d, mut sel) = (d.clone(), sel.clone());
                        let path = d.alloc_id();
                        let points = if path_is_segment {
                            vec![at - vectorcraft_geom::Vec2::new(0.0, 30.0), at + vectorcraft_geom::Vec2::new(0.0, 30.0)]
                        } else {
                            vec![at, at + vectorcraft_geom::Vec2::new(0.0, 60.0)]
                        };
                        let node = Node::path(path, PathData::single(SubPath::polyline(&points, false)), Appearance::default_art());
                        d.insert(Some(d.layers[0].id), if path_on_top { 2 } else { 1 }, node).unwrap();
                        if path_selected {
                            sel.set(if blend_first { vec![blend, path] } else { vec![path, blend] });
                        }
                        let c = cx(&d, &sel, &paint);
                        let path_wins = path_selected && path_on_top;
                        let expected_id = if path_wins { path } else { blend };
                        let spine = crate::direct::spines(&c).into_iter().find(|(id, _)| *id == blend).unwrap().1;
                        let spine_segment = AnchorTool::spine_point(&spine, at, c.point_tol()).is_none()
                            && AnchorTool::spine_handle(&c, &spine, at, c.point_tol()).is_none();
                        let reshape = if path_wins { path_is_segment } else { spine_segment };
                        let mut tool = AnchorTool::new("anchorPoint");
                        assert_eq!(tool.cursor(&c, at, Mods::default()), if reshape { Cursor::ReshapeSegment } else { Cursor::AnchorPoint });
                        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
                        let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, at.x + 10.0, at.y + 20.0));
                        let Some(Action::Preview(cmd, args)) = actions.get(1) else { panic!("missing preview: {actions:?}") };
                        assert_eq!(
                            args["id"], expected_id.0,
                            "top={path_on_top}, selected={path_selected}, blend_first={blend_first}, at={at:?}, segment={path_is_segment}: {cmd}"
                        );
                        assert_eq!(cmd.starts_with("path."), path_wins);
                        assert_eq!(cmd.ends_with("reshapeSegment"), reshape);
                    }
                }
            }
        }
    }

    #[test]
    fn anchor_point_converts_a_blend_spine_point_before_reshaping_its_segment() {
        let (d, id, sel) = blend_spine_doc(false);
        let p = paint();
        let c = cx(&d, &sel, &p);
        let mut tool = AnchorTool::new("anchorPoint");
        assert!(tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 160.0, 310.0)).is_empty());
        let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 340.0));
        assert_eq!(actions.first(), Some(&Action::Begin("Convert Anchor Point".into())));
        assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
            if cmd == "object.blend.spine.convertAnchor"
            && args["id"] == id.0 && args["anchor"] == 1
            && args["to"] == "smooth" && args["y"] == 340.0));
        assert_eq!(tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 340.0)), vec![Action::Commit]);
    }

    #[test]
    fn shift_reshapes_a_blend_spine_segment_perpendicular() {
        let (d, id, sel) = blend_spine_doc(true);
        let p = paint();
        let c = cx(&d, &sel, &p);
        let spine = crate::direct::spines(&c).pop().unwrap().1;
        let at = spine.subpaths[0].segment(1).eval(0.5);
        for shift in [false, true] {
            let mut tool = AnchorTool::new("anchorPoint");
            tool.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
            let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, at.x, at.y + 20.0).with_mods(Mods { shift, ..Mods::default() }));
            assert!(
                matches!(actions.get(1), Some(Action::Preview(cmd, args))
                if cmd == "object.blend.spine.reshapeSegment" && args["id"] == id.0
                && args["perpendicular"] == shift),
                "{actions:?}"
            );
        }
    }

    #[test]
    fn anchor_point_keeps_blend_spine_handles_visible_and_editable_after_switch() {
        let (d, id, sel) = blend_spine_doc(true);
        let p = paint();
        let c = cx(&d, &sel, &p);
        // A new tool instance models switching from Direct Selection.
        let mut tool = AnchorTool::new("anchorPoint");
        let overlays = tool.overlays(&c);
        assert!(overlays.iter().any(|o| matches!(o, Overlay::Handle { p, .. } if *p == Point::new(190.0, 330.0))));
        assert!(overlays.iter().any(|o| matches!(o, Overlay::Handle { p, .. } if *p == Point::new(130.0, 290.0))));
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
        let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 190.0, 350.0));
        assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
            if cmd == "object.blend.spine.moveAnchor"
            && args["id"] == id.0 && args["anchor"] == 1 && args["handle"] == "out"
            && args["independent"] == true));
        assert_eq!(tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 190.0, 350.0)), vec![Action::Commit]);

        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 160.0, 310.0));
        assert_eq!(
            tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 310.0)),
            vec![Action::Exec("object.blend.spine.convertAnchor".into(), json!({"id": id.0, "anchor": 1, "to": "corner"}),)],
        );
    }

    /// P1-1: a handle 2pt from its anchor must not intercept a click at the anchor centre;
    /// dragging the anchor converts it, while a click on an eligible handle still retracts it.
    #[test]
    fn anchor_point_prefers_a_blend_spine_anchor_over_its_short_handle() {
        let (mut d, _) = doc_with_rect();
        let layer = d.layers[0].id;
        let (id, first, last) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let key = |id, x| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art()))
        };
        let mut path = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(160.0, 310.0), Point::new(210.0, 310.0)], false);
        path.anchors[1] = Anchor::smooth(Point::new(160.0, 310.0), Point::new(162.0, 310.0));
        let spec =
            vectorcraft_doc::live::BlendSpec { spine: Some(vectorcraft_geom::PathData::single(path)), key_anchors: vec![0, 2], ..Default::default() };
        let blend = vectorcraft_doc::Node::new(
            id,
            vectorcraft_doc::NodeKind::Blend { children: vec![key(first, 100.0), key(last, 200.0)], spec: Box::new(spec) },
        );
        d.insert(Some(layer), 1, blend).unwrap();
        let mut selected = Selection::default();
        selected.set([id]);
        let p = paint();
        let c = cx(&d, &selected, &p);
        // Clicking the anchor centre converts the smooth point to a corner.
        let mut tool = AnchorTool::new("anchorPoint");
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 160.0, 310.0));
        assert_eq!(
            tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 310.0)),
            vec![Action::Exec("object.blend.spine.convertAnchor".into(), json!({"id": id.0, "anchor": 1, "to": "corner"}),)],
        );
        // Clicking the eligible handle end retracts only that handle.
        let mut tool = AnchorTool::new("anchorPoint");
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 162.0, 310.0));
        assert_eq!(
            tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 162.0, 310.0)),
            vec![Action::Exec(
                "object.blend.spine.moveAnchor".into(),
                json!({"id": id.0, "anchor": 1, "handle": "out", "x": 160.0, "y": 310.0, "independent": true}),
            )],
        );
    }

    /// P2-2: with Show handles when multiple anchors are selected off, spine handles hide
    /// and cannot intercept pointer input, as for ordinary paths.
    #[test]
    fn anchor_point_hides_blend_spine_handles_when_multiple_is_off() {
        let (d, _, sel) = blend_spine_doc(true);
        let p = paint();
        let single = ToolContext { handles_multiple: false, ..cx(&d, &sel, &p) };
        let tool = AnchorTool::new("anchorPoint");
        assert!(tool.overlays(&single).iter().all(|o| !matches!(o, Overlay::Handle { .. })));
        // The hidden handle endpoint is neither an editable handle nor on the curve.
        let mut tool = AnchorTool::new("anchorPoint");
        tool.pointer(&single, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
        assert!(matches!(tool.state, State::Idle));
        assert!(tool.pointer(&single, &PointerEvent::new(PointerKind::Up, 190.0, 330.0)).is_empty());
        // A drag on the curve away from anchors and handles reshapes the spine segment.
        let (_, spine) = crate::direct::spines(&single).pop().unwrap();
        let at = spine.subpaths[0].segment(1).eval(0.5);
        assert!(AnchorTool::spine_handle(&single, &spine, at, single.point_tol()).is_none());
        assert!(AnchorTool::spine_point(&spine, at, single.point_tol()).is_none());
        let mut tool = AnchorTool::new("anchorPoint");
        assert_eq!(tool.cursor(&single, at, Mods::default()), Cursor::ReshapeSegment);
        tool.pointer(&single, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        let actions = tool.pointer(&single, &PointerEvent::new(PointerKind::Drag, at.x, at.y + 20.0));
        assert!(matches!(actions.get(1), Some(Action::Preview(cmd, _)) if cmd == "object.blend.spine.reshapeSegment"));
    }

    /// Hover and click agree: the caret shows where a press converts a spine point or
    /// edits a spine handle, and ReshapeSegment only where a press reshapes a segment.
    #[test]
    fn anchor_point_cursor_matches_spine_press_precedence() {
        let (d, _, sel) = blend_spine_doc(true);
        let p = paint();
        let c = cx(&d, &sel, &p);
        let tool = AnchorTool::new("anchorPoint");
        let none = Mods::default();
        assert_eq!(tool.cursor(&c, Point::new(160.0, 310.0), none), Cursor::AnchorPoint);
        assert_eq!(tool.cursor(&c, Point::new(190.0, 330.0), none), Cursor::AnchorPoint);
        let spine = crate::direct::spines(&c).pop().unwrap().1;
        assert_eq!(tool.cursor(&c, spine.subpaths[0].segment(1).eval(0.5), none), Cursor::ReshapeSegment);
    }

    /// P1-2: Shift-dragging a blend spine handle constrains it to 45-degree steps round
    /// its anchor, as for ordinary Anchor Point handles.
    #[test]
    fn anchor_point_shift_constrains_a_blend_spine_handle() {
        let (d, id, sel) = blend_spine_doc(true);
        let p = paint();
        let c = cx(&d, &sel, &p);
        let shift = Mods { shift: true, ..Mods::default() };
        let mut tool = AnchorTool::new("anchorPoint");
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
        let drag = PointerEvent { kind: PointerKind::Drag, pos: Point::new(200.0, 340.0), mods: shift, pressure: 1.0 };
        let actions = tool.pointer(&c, &drag);
        // The pointer (40, 30) keeps its length of 50; Shift snaps it to 45 degrees.
        assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
            if cmd == "object.blend.spine.moveAnchor"
            && args["id"] == id.0 && args["anchor"] == 1 && args["handle"] == "out"
            && args["independent"] == true
            && (args["x"].as_f64().unwrap() - 160.0 - 50.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6
            && (args["y"].as_f64().unwrap() - 310.0 - 50.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6));
        assert_eq!(tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 200.0, 340.0)), vec![Action::Commit]);
    }

    #[test]
    fn add_and_delete_anchor_clicks() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = AnchorTool::new("addAnchor");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0));
        assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.insertAnchor" && v["id"] == id.0 && v["segment"] == 0));
        assert!((t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 20.0, 20.0))).is_empty());
        let mut t = AnchorTool::new("deleteAnchor");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 101.0));
        assert_eq!(a, vec![Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1}))]);
    }

    #[test]
    fn scissors_splits_at_segment() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = AnchorTool::new("scissors");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 150.0));
        assert!(
            matches!(&a[0], Action::Exec(c, v) if c == "path.split" && v["id"] == id.0 && v["segment"] == 1 && (v["t"].as_f64().unwrap() - 0.5).abs() < 1e-6)
        );
    }

    /// The Add Anchor Point, Delete Anchor Point, Anchor Point and Scissors tools pick anchors and
    /// segments only on the art a click can pick: not on a Shaper Group's original shapes unless
    /// they are isolated (Construction mode), and in isolation mode only on the isolated object.
    #[test]
    fn anchor_tools_leave_out_shaper_sources_and_art_outside_isolation() {
        let (d, _, sources, face) = doc_with_shaper_group();
        let rect = d.layers[0].children().unwrap()[0].id;
        let b = d.node(sources).and_then(|n| n.children()).unwrap()[1].id;
        let (s, p) = (Selection::default(), paint());
        // The path the first command of a press with `tool` at (x, y) acts on.
        let picked = |isolation: Option<NodeId>, tool: &str, x: f64, y: f64| {
            let cx = ToolContext { isolation, ..cx(&d, &s, &p) };
            let mut t = AnchorTool::new(tool);
            let mut a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y));
            a.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x + 20.0, y - 10.0)));
            a.iter().find_map(|a| match a {
                Action::Exec(_, v) | Action::Preview(_, v) => v["id"].as_u64().map(NodeId),
                _ => None,
            })
        };
        // The second square's top right corner and its right side, which only its original shape
        // has.
        for (tool, x, y) in [("deleteAnchor", 150.0, 300.0), ("anchorPoint", 150.0, 300.0), ("addAnchor", 150.0, 350.0), ("scissors", 150.0, 350.0)] {
            assert_eq!(picked(None, tool, x, y), None, "{tool}");
            assert_eq!(picked(Some(sources), tool, x, y), Some(b), "{tool}");
        }
        // The face's top left corner, outside the isolated rectangle.
        assert_eq!(picked(None, "deleteAnchor", 0.0, 300.0), Some(face));
        assert_eq!(picked(Some(rect), "deleteAnchor", 0.0, 300.0), None);
        assert_eq!(picked(Some(rect), "deleteAnchor", 100.0, 100.0), Some(rect));
    }

    #[test]
    fn anchor_point_drag_pulls_handles_click_converts() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = AnchorTool::new("anchorPoint");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 90.0));
        assert_eq!(a[0], Action::Begin("Convert Anchor Point".into()));
        assert!(matches!(&a[1], Action::Preview(c, v) if c == "path.convertAnchor" && v["to"] == "smooth" && v["id"] == id.0));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 90.0)), vec![Action::Commit]);
        // Clicking a corner without handles does nothing.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)).is_empty());
        // Dragging a segment reshapes it.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 200.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 230.0));
        assert!(matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["dy"] == 30.0));
    }

    /// Shift held while a segment is dragged keeps its handles at right angles to the line between
    /// its anchors (`perpendicular`).
    #[test]
    fn shift_reshapes_a_segment_perpendicular() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Mods::default() };
        for (mods, perpendicular) in [(Mods::default(), false), (shift, true)] {
            let mut t = AnchorTool::new("anchorPoint");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 200.0));
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.0, 230.0).with_mods(mods));
            assert!(
                matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["id"] == id.0 && v["perpendicular"] == perpendicular),
                "{a:?}"
            );
        }
    }

    /// #494: the Anchor Point tool picks within Selection & Anchor Display → Tolerance, and drags
    /// only the handles shown.
    #[test]
    fn anchor_point_picks_within_the_tolerance_and_drags_shown_handles() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
        sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 300.0));
        let path = vectorcraft_geom::PathData::single(sp);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let press = |c: &ToolContext, x: f64, y: f64| {
            let mut t = AnchorTool::new("anchorPoint");
            t.pointer(c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(c, &PointerEvent::new(PointerKind::Drag, x + 10.0, y + 30.0))
        };
        let dragged = |a: Vec<Action>| match a.as_slice() {
            [Action::Begin(_), Action::Preview(c, _)] => c.clone(),
            _ => String::new(),
        };
        // The segment 6 px off: picked with an 8 px tolerance, not with the default 3 px.
        assert_eq!(dragged(press(&cx(&d, &s, &p), 150.0, 306.0)), "");
        let wide = ToolContext { selection_tolerance: 8.0, ..cx(&d, &s, &p) };
        assert_eq!(dragged(press(&wide, 150.0, 306.0)), "path.reshapeSegment");
        // The middle anchor's handle drags; with Show handles when multiple anchors are selected
        // off it's hidden (three anchors selected) and the press finds the segment under it.
        assert_eq!(dragged(press(&cx(&d, &s, &p), 240.0, 301.0)), "path.setHandle");
        let single = ToolContext { handles_multiple: false, ..cx(&d, &s, &p) };
        assert_eq!(dragged(press(&single, 240.0, 301.0)), "path.reshapeSegment");
    }

    /// #2267: clicking a handle end retracts that handle only; the anchor becomes a corner and
    /// keeps the other handle, as Illustrator's Anchor Point tool does.
    #[test]
    fn anchor_point_click_on_a_handle_retracts_only_that_handle() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
        sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 320.0));
        let path = vectorcraft_geom::PathData::single(sp);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let retract = |which: &str| {
            vec![Action::Exec(
                "path.setHandle".into(),
                json!({"id": id.0, "subpath": 0, "anchor": 1, "which": which, "x": 200.0, "y": 300.0, "independent": true}),
            )]
        };
        let mut t = AnchorTool::new("anchorPoint");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 240.0, 320.0));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 240.0, 320.0)), retract("out"));
        // A hand that shakes by a pixel still clicks; the incoming handle sits opposite, at (160, 280).
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 160.0, 280.0));
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.5, 280.5)).is_empty());
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 160.5, 280.5)), retract("in"));
        // The command leaves the other handle where it was.
        let mut a = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 320.0));
        a.set_handle(true, a.p, true);
        assert!(!a.has_out());
        assert_eq!(a.h_in, Point::new(160.0, 280.0));
    }

    /// Shift held at the press on an anchor moves it, handles and all, instead of pulling out
    /// handles; Shift still held keeps the move at 45° steps, released frees it. A Shift-click
    /// that doesn't move leaves the anchor as it was.
    #[test]
    fn anchor_point_shift_drag_moves_the_anchor() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
        sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 320.0));
        let path = vectorcraft_geom::PathData::single(sp);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Mods::default() };
        let moved = |x: f64, y: f64| Action::Preview("path.moveAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "x": x, "y": y}));
        let mut t = AnchorTool::new("anchorPoint");
        // Pressed a pixel off the anchor: it keeps that offset to the pointer.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 201.0, 301.0).with_mods(shift));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 231.0, 341.0));
        assert_eq!(a, vec![Action::Begin("Move".into()), moved(230.0, 340.0)]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 241.0, 306.0).with_mods(shift));
        assert!(
            matches!(&a[..], [Action::Preview(c, v)] if c == "path.moveAnchor" && v["y"].as_f64().is_some_and(|y| (y - 300.0).abs() < 1e-9)),
            "{a:?}"
        );
        assert_eq!(t.cursor(&cx, Point::new(241.0, 306.0), shift), Cursor::Arrow);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 241.0, 306.0)), vec![Action::Commit]);
        // Without Shift the same press still pulls out handles.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 230.0, 340.0));
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.convertAnchor"), "{a:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 230.0, 340.0));
        // A Shift-click that doesn't move does nothing (no conversion to a corner).
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0).with_mods(shift));
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 200.0, 300.0).with_mods(shift)).is_empty());
    }

    #[test]
    fn anchor_point_shift_drag_honors_measurement_labels() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, vectorcraft_geom::PathData::single(sp), vectorcraft_doc::Appearance::default_art()))
            .unwrap();
        let s = Selection::default();
        let p = paint();
        let base = cx(&d, &s, &p);
        let on = ToolContext { measurement_labels: true, ..base };
        let off = ToolContext { measurement_labels: false, ..base };
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = AnchorTool::new("anchorPoint");
        t.pointer(&on, &PointerEvent::new(PointerKind::Down, 200.0, 300.0).with_mods(shift));
        t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 230.0, 340.0));
        let want = on.offset_label(30.0, 40.0);
        assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
        assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    }
}
