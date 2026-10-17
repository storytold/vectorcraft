//! Add Anchor Point (+), Delete Anchor Point (−), Anchor Point (Shift+C) and Scissors (C).
//!
//! Add: click a segment to insert an anchor without changing the shape (Alt = delete).
//! Delete: click an anchor to remove it, re-fitting the neighbouring curve (Alt = add).
//! Anchor Point: click a smooth anchor → corner; drag an anchor → pull out smooth handles; click a
//! handle end → retract that handle only (#2267); drag a handle → move it independently (Shift: at
//! 45° steps round its anchor); drag a segment → reshape the curve, a selected blend's spine's too,
//! straight or curved (#1111; Shift: its handles at right angles to the line between its anchors,
//! at one length). Blend spine points and handles follow the same conversions (#2544).
//! Scissors: click a path to split it there.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Anchor;
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
}

impl AnchorTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "deleteAnchor" => "deleteAnchor",
            "anchorPoint" => "anchorPoint",
            "scissors" => "scissors",
            _ => "addAnchor",
        };
        Self { id, state: State::Idle, guides: vec![], snap: HandleSnap::default() }
    }

    /// Only selected, editable blends are eligible; their visible spines are
    /// computed in document coordinates, as for Direct Selection.
    fn spine_point(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, Anchor)> {
        let mut best: Option<((NodeId, usize, Anchor), f64)> = None;
        for (id, path) in crate::direct::spines(cx).into_iter().filter(|(id, _)| cx.selection.contains(*id)) {
            for (_, ai, a) in path.anchors().filter(|(si, _, _)| *si == 0) {
                let d = a.p.distance(p);
                if d <= tol && best.is_none_or(|b| d < b.1) {
                    best = Some(((id, ai, *a), d));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// The end of a shown spine handle nearest to p within tol, and nearer to it than
    /// the handle's anchor (a short handle leaves its anchor to be picked).
    fn spine_handle(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, bool)> {
        let mut best: Option<((NodeId, usize, bool), f64)> = None;
        for (id, path) in crate::direct::spines(cx).into_iter().filter(|(id, _)| cx.selection.contains(*id)) {
            // With Show handles when multiple anchors are selected off, more than one spine
            // anchor hides every handle, as for ordinary paths.
            let anchors: Vec<_> = path.anchors().filter(|(si, _, _)| *si == 0).collect();
            if !cx.handles_multiple && anchors.len() > 1 {
                continue;
            }
            for (_, ai, a) in anchors {
                for (out, has, h) in [(true, a.has_out(), a.h_out), (false, a.has_in(), a.h_in)] {
                    let d = h.distance(p);
                    if has && d <= tol && d < a.p.distance(p) && best.is_none_or(|b| d < b.1) {
                        best = Some(((id, ai, out), d));
                    }
                }
            }
        }
        best.map(|b| b.0)
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
                if let Some((id, si, ai, out)) = hit_handle(cx, p, point) {
                    self.state = State::Handle { id, si, ai, out, start: p, began: false };
                    self.snap = HandleSnap::default();
                } else if let Some((id, ai, out)) = Self::spine_handle(cx, p, point) {
                    self.state = State::SpineHandle { id, ai, out, start: p, began: false };
                } else if let Some((id, ai, _)) = Self::spine_point(cx, p, point) {
                    self.state = State::SpineConvert { id, ai, start: p, began: false };
                } else if let Some((id, si, ai)) = hit_anchor(cx, p, point) {
                    self.state = State::Convert { id, si, ai, start: p, began: false };
                } else if let Some((id, si, seg, t)) = hit_segment(cx, p, tol) {
                    self.state = State::Reshape { id, si, seg, t, start: p, began: false, spine: false };
                } else if let Some((id, seg, t, _)) = crate::direct::hit_spine_segment(cx, p, tol) {
                    self.state = State::Reshape { id, si: 0, seg, t, start: p, began: false, spine: true };
                }
                vec![]
            }
            ("anchorPoint", PointerKind::Drag) => {
                let mut out = vec![];
                let (began, far) = match self.state {
                    State::Idle => return out,
                    State::Convert { began, start, .. }
                    | State::Handle { began, start, .. }
                    | State::SpineConvert { began, start, .. }
                    | State::SpineHandle { began, start, .. }
                    | State::Reshape { began, start, .. } => {
                        (began, p.distance(start) >= cx.tol(2.0))
                    }
                };
                if !began {
                    if !far {
                        return out;
                    }
                    out.push(Action::Begin(if matches!(self.state, State::Convert { .. } | State::SpineConvert { .. }) { "Convert Anchor Point" } else { "Reshape" }.into()));
                    if let State::Convert { began, .. }
                    | State::Handle { began, .. }
                    | State::SpineConvert { began, .. }
                    | State::SpineHandle { began, .. }
                    | State::Reshape { began, .. } = &mut self.state {
                        *began = true;
                    }
                }
                let preview = match self.state {
                    State::Idle => return out,
                    State::Convert { id, si, ai, .. } => Action::Preview(
                        "path.convertAnchor".into(),
                        json!({"id": id.0, "subpath": si, "anchor": ai, "to": "smooth", "x": p.x, "y": p.y}),
                    ),
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
                let st = std::mem::replace(&mut self.state, State::Idle);
                match st {
                    State::Convert { began: true, .. }
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
        let st = std::mem::replace(&mut self.state, State::Idle);
        match st {
            State::Convert { began: true, .. }
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
                State::Idle => press_cursor(cx, p),
                State::Convert { .. } | State::Handle { .. } => Cursor::AnchorPoint,
            },
        }
    }
}

/// The Anchor Point tool's cursor for a press at `p`: the Reshape Segment cursor where it takes a
/// segment (a path's or a selected blend's spine's), else its caret. Handles and anchors win
/// over the segments through them, as the press has it.
pub(crate) fn press_cursor(cx: &ToolContext, p: Point) -> Cursor {
    let (point, tol) = (cx.point_tol(), cx.pick_tol());
    let segment = hit_handle(cx, p, point).is_none()
        && hit_anchor(cx, p, point).is_none()
        && (hit_segment(cx, p, tol).is_some() || crate::direct::hit_spine_segment(cx, p, tol).is_some());
    if segment { Cursor::ReshapeSegment } else { Cursor::AnchorPoint }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

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
        let mut path = vectorcraft_geom::SubPath::polyline(
            &[Point::new(110.0, 310.0), Point::new(160.0, 310.0), Point::new(210.0, 310.0)],
            false,
        );
        if smooth {
            path.anchors[1] = Anchor::smooth(Point::new(160.0, 310.0), Point::new(190.0, 330.0));
        }
        let mut spec = vectorcraft_doc::live::BlendSpec::default();
        spec.spine = Some(vectorcraft_geom::PathData::single(path));
        spec.key_anchors = vec![0, 2];
        let blend = vectorcraft_doc::Node::new(
            id,
            vectorcraft_doc::NodeKind::Blend { children: vec![key(first, 100.0), key(last, 200.0)], spec: Box::new(spec) },
        );
        d.insert(Some(layer), 1, blend).unwrap();
        let mut selected = Selection::default();
        selected.set([id]);
        (d, id, selected)
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
            vec![Action::Exec(
                "object.blend.spine.convertAnchor".into(),
                json!({"id": id.0, "anchor": 1, "to": "corner"}),
            )],
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
        let mut spec = vectorcraft_doc::live::BlendSpec::default();
        spec.spine = Some(vectorcraft_geom::PathData::single(path));
        spec.key_anchors = vec![0, 2];
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
        let mut tool = AnchorTool::new("anchorPoint");
        // The press falls through to the spine point, not the hidden handle.
        tool.pointer(&single, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
        let actions = tool.pointer(&single, &PointerEvent::new(PointerKind::Drag, 190.0, 350.0));
        assert!(matches!(actions.get(1), Some(Action::Preview(cmd, _)) if cmd == "object.blend.spine.reshapeSegment"));
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
        // The pointer is at ~38 degrees from the anchor; Shift snaps it to 45 degrees.
        assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
            if cmd == "object.blend.spine.moveAnchor"
            && args["id"] == id.0 && args["anchor"] == 1 && args["handle"] == "out"
            && args["independent"] == true
            && (args["x"].as_f64().unwrap() - 160.0 - 40.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6
            && (args["y"].as_f64().unwrap() - 310.0 - 40.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6));
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
}
