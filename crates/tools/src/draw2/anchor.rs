//! Add Anchor Point (+), Delete Anchor Point (−), Anchor Point (Shift+C) and Scissors (C).
//!
//! Add: click a segment to insert an anchor without changing the shape (Alt = delete).
//! Delete: click an anchor to remove it, re-fitting the neighbouring curve (Alt = add).
//! Anchor Point: click a smooth anchor → corner; drag an anchor → pull out smooth handles; drag a
//! handle → move it independently (Shift: at 45° steps round its anchor); drag a segment → reshape
//! the curve.
//! Scissors: click a path to split it there.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::{hit_anchor, hit_segment};
use crate::{Action, Cursor, Mods, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Clone, Copy, Debug)]
enum State {
    Idle,
    Convert { id: NodeId, si: usize, ai: usize, start: Point, began: bool },
    Handle { id: NodeId, si: usize, ai: usize, out: bool, began: bool },
    Reshape { id: NodeId, si: usize, seg: usize, t: f64, start: Point, began: bool },
}

pub struct AnchorTool {
    id: &'static str,
    state: State,
}

impl AnchorTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "deleteAnchor" => "deleteAnchor",
            "anchorPoint" => "anchorPoint",
            "scissors" => "scissors",
            _ => "addAnchor",
        };
        Self { id, state: State::Idle }
    }

    fn add(cx: &ToolContext, p: Point) -> Vec<Action> {
        match hit_segment(cx, p, cx.tol(4.0)) {
            Some((id, si, seg, t)) => vec![Action::Exec("path.insertAnchor".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))],
            None => vec![],
        }
    }

    fn delete(cx: &ToolContext, p: Point) -> Vec<Action> {
        match hit_anchor(cx, p, cx.tol(4.0)) {
            Some((id, si, ai)) => vec![Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))],
            None => vec![],
        }
    }
}

/// A direction handle of a selected path under `p`: (id, subpath, anchor, is_out).
fn hit_handle(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, bool)> {
    for id in &cx.selection.objects {
        let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data()) else { continue };
        for (si, ai, a) in pd.anchors() {
            if a.has_out() && a.h_out.distance(p) <= tol {
                return Some((*id, si, ai, true));
            }
            if a.has_in() && a.h_in.distance(p) <= tol {
                return Some((*id, si, ai, false));
            }
        }
    }
    None
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
        let tol = cx.tol(4.0);
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
                if let Some((id, si, ai)) = hit_anchor(cx, p, tol) {
                    return vec![Action::Exec("path.split".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))];
                }
                match hit_segment(cx, p, tol) {
                    Some((id, si, seg, t)) => vec![Action::Exec("path.split".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))],
                    None => vec![],
                }
            }
            ("anchorPoint", PointerKind::Down) => {
                if let Some((id, si, ai, out)) = hit_handle(cx, p, tol) {
                    self.state = State::Handle { id, si, ai, out, began: false };
                } else if let Some((id, si, ai)) = hit_anchor(cx, p, tol) {
                    self.state = State::Convert { id, si, ai, start: p, began: false };
                } else if let Some((id, si, seg, t)) = hit_segment(cx, p, tol) {
                    self.state = State::Reshape { id, si, seg, t, start: p, began: false };
                }
                vec![]
            }
            ("anchorPoint", PointerKind::Drag) => {
                let mut out = vec![];
                let (began, far) = match self.state {
                    State::Idle => return out,
                    State::Convert { began, start, .. } | State::Reshape { began, start, .. } => (began, p.distance(start) >= cx.tol(2.0)),
                    State::Handle { began, .. } => (began, true),
                };
                if !began {
                    if !far {
                        return out;
                    }
                    out.push(Action::Begin(if matches!(self.state, State::Convert { .. }) { "Convert Anchor Point" } else { "Reshape" }.into()));
                    if let State::Convert { began, .. } | State::Handle { began, .. } | State::Reshape { began, .. } = &mut self.state {
                        *began = true;
                    }
                }
                let preview = match self.state {
                    State::Idle => return out,
                    State::Convert { id, si, ai, .. } => Action::Preview(
                        "path.convertAnchor".into(),
                        json!({"id": id.0, "subpath": si, "anchor": ai, "to": "smooth", "x": p.x, "y": p.y}),
                    ),
                    State::Handle { id, si, ai, out, .. } => {
                        let h = crate::direct::handle_at(cx, id, si, ai, p, ev.mods.shift);
                        Action::Preview(
                            "path.setHandle".into(),
                            json!({"id": id.0, "subpath": si, "anchor": ai, "which": if out { "out" } else { "in" }, "x": h.x, "y": h.y, "independent": true}),
                        )
                    }
                    State::Reshape { id, si, seg, t, start, .. } => Action::Preview(
                        "path.reshapeSegment".into(),
                        json!({"id": id.0, "subpath": si, "segment": seg, "t": t, "dx": p.x - start.x, "dy": p.y - start.y}),
                    ),
                };
                out.push(preview);
                out
            }
            ("anchorPoint", PointerKind::Up) => {
                let st = std::mem::replace(&mut self.state, State::Idle);
                match st {
                    State::Convert { began: true, .. } | State::Handle { began: true, .. } | State::Reshape { began: true, .. } => {
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
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let st = std::mem::replace(&mut self.state, State::Idle);
        match st {
            State::Convert { began: true, .. } | State::Handle { began: true, .. } | State::Reshape { began: true, .. } => vec![Action::Commit],
            _ => vec![],
        }
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        match self.id {
            "addAnchor" if !m.alt => Cursor::PenAdd,
            "addAnchor" => Cursor::PenDelete,
            "deleteAnchor" if !m.alt => Cursor::PenDelete,
            "deleteAnchor" => Cursor::PenAdd,
            "scissors" => {
                if hit_segment(cx, p, cx.tol(4.0)).is_some() {
                    Cursor::Crosshair
                } else {
                    Cursor::NotAllowed
                }
            }
            _ => Cursor::Pen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

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
}
