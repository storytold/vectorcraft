//! The Pen tool (P).
//!
//! Click adds a corner anchor; click-drag adds a smooth anchor with symmetric handles (Alt-drag
//! breaks the handles); Shift constrains to 45°. Clicking the first anchor closes the path.
//! Enter/Esc (or switching tools) ends the path. Clicking the end of a selected open path continues
//! it. The rubber-band preview shows the next segment. On a selected blend's spine a click adds a
//! point (on a point no key object sits on: deletes it).

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{BezPath, Point};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Default)]
pub struct PenTool {
    /// The path being drawn (set once the first anchor exists and the engine selected it).
    drawing: bool,
    drag: Option<(Point, bool)>,
    hover: Option<Point>,
    /// The smart guides of the point under the pointer (where a click would land).
    hover_guides: Vec<Overlay>,
}

/// The open path the pen is extending: the single selected open path.
fn active_path(cx: &ToolContext) -> Option<(NodeId, Point, Point, Point)> {
    if cx.selection.objects.len() != 1 {
        return None;
    }
    let id = cx.selection.objects[0];
    let n = cx.doc.node(id)?;
    let NodeKind::Path { path, .. } = &n.kind else { return None };
    let sp = path.subpaths.last()?;
    if sp.closed {
        return None;
    }
    let first = sp.anchors.first()?.p;
    let last = sp.anchors.last()?;
    Some((id, first, last.p, last.h_out))
}

impl Tool for PenTool {
    fn id(&self) -> &'static str {
        "pen"
    }
    fn busy(&self) -> bool {
        self.drag.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let exclude: Vec<vectorcraft_doc::NodeId> = if self.drawing { cx.selection.objects.clone() } else { vec![] };
        let active = if self.drawing { active_path(cx) } else { None };
        // The next anchor snaps tangent or perpendicular to other paths, seen from the last one.
        let from = active.map(|(_, _, last, _)| last);
        let (mut p, guides) = if matches!(ev.kind, PointerKind::Down | PointerKind::Move) {
            crate::guides::snap_draw_from(cx, ev.pos, &exclude, from)
        } else {
            (ev.pos, vec![])
        };
        let tol = cx.tol(5.0);
        if self.drawing && active.is_none() && ev.kind == PointerKind::Down {
            self.drawing = false;
        }
        match ev.kind {
            PointerKind::Move => {
                self.hover = Some(p);
                self.hover_guides = guides;
                vec![]
            }
            PointerKind::Down => {
                if let Some((id, first, last, _)) = active {
                    if ev.mods.shift {
                        p = last + vectorcraft_geom::constrain_angle(p - last, 45.0);
                    }
                    if p.distance(first) <= tol {
                        self.drag = Some((first, true));
                        return vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))];
                    }
                    self.drag = Some((p, false));
                    return vec![Action::Begin("Pen".into()), Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": p.x, "y": p.y}))];
                }
                if let Some(acts) = spine_click(cx, p, tol) {
                    return acts;
                }
                // Continue a selected open path when clicking on one of its ends.
                if let Some((_, first, last, _)) = active_path(cx)
                    && (p.distance(last) <= tol || p.distance(first) <= tol)
                {
                    self.drawing = true;
                    if p.distance(first) <= tol && p.distance(last) > tol {
                        return vec![Action::Exec("path.reverse".into(), json!({}))];
                    }
                    return vec![];
                }
                self.drawing = true;
                self.drag = Some((p, false));
                vec![Action::Begin("Pen".into()), Action::Preview("path.create".into(), json!({"anchors": [{"x": p.x, "y": p.y}]}))]
            }
            PointerKind::Drag => {
                let Some((a, closing)) = self.drag else { return vec![] };
                let mut out_h = ev.pos;
                if ev.mods.shift {
                    out_h = a + vectorcraft_geom::constrain_angle(ev.pos - a, 45.0);
                }
                let alt = ev.mods.alt;
                let Some((id, ..)) = active_path(cx).or(active) else {
                    // First anchor of a new path: re-issue create with handles.
                    let in_h = a - (out_h - a);
                    return vec![Action::Preview(
                        "path.create".into(),
                        json!({"anchors": [{"x": a.x, "y": a.y, "out": [out_h.x, out_h.y], "in": [in_h.x, in_h.y]}]}),
                    )];
                };
                if closing {
                    return vec![Action::Preview(
                        "path.close".into(),
                        json!({"id": id.0, "in": [2.0 * a.x - out_h.x, 2.0 * a.y - out_h.y], "independent": alt}),
                    )];
                }
                let in_h = if alt { a } else { a - (out_h - a) };
                vec![Action::Preview(
                    "path.appendAnchor".into(),
                    json!({"id": id.0, "x": a.x, "y": a.y, "in": [in_h.x, in_h.y], "out": [out_h.x, out_h.y]}),
                )]
            }
            PointerKind::Up => {
                let Some((_, closing)) = self.drag.take() else { return vec![] };
                if closing {
                    self.drawing = false;
                }
                vec![Action::Commit]
            }
            PointerKind::DoubleClick => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Enter | ToolKey::Escape => {
                self.drawing = false;
                self.drag = None;
                vec![]
            }
            _ => vec![],
        }
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drawing = false;
        vec![]
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if self.drag.is_some() {
            return vec![];
        }
        if !self.drawing {
            return self.hover_guides.clone();
        }
        let (Some((id, _, last, out)), Some(h)) = (active_path(cx), self.hover) else { return self.hover_guides.clone() };
        let mut bp = BezPath::new();
        bp.move_to(last);
        if out.distance(last) > 1e-9 {
            bp.quad_to(out, h);
        } else {
            bp.line_to(h);
        }
        let c = cx.doc.layer_color(id);
        let mut o = vec![Overlay::Path { path: bp, color: c, width: 1.0, dashed: false }];
        o.extend(self.hover_guides.iter().cloned());
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if !self.drawing {
            match spine_click(cx, p, cx.tol(5.0)).as_deref() {
                Some([Action::Exec(c, _)]) if c == "object.blend.spine.removeAnchor" => return Cursor::PenDelete,
                Some([_]) => return Cursor::PenAdd,
                _ => {}
            }
        }
        if let Some((_, first, last, _)) = active_path(cx) {
            if self.drawing && p.distance(first) <= cx.tol(5.0) {
                return Cursor::PenClose;
            }
            if !self.drawing && (p.distance(last) <= cx.tol(5.0) || p.distance(first) <= cx.tol(5.0)) {
                return Cursor::PenContinue;
            }
        }
        Cursor::Pen
    }
}

/// A click on a selected blend's spine: delete the point under `p` when no key object sits on it
/// (a key's point does nothing), else add one where the spine passes within `tol`.
fn spine_click(cx: &ToolContext, p: Point, tol: f64) -> Option<Vec<Action>> {
    for (id, path) in crate::direct::spines(cx).into_iter().filter(|(id, _)| cx.selection.contains(*id)) {
        let Some(NodeKind::Blend { children, spec }) = cx.doc.node(id).map(|n| &n.kind) else { continue };
        let keys = vectorcraft_doc::live::blend_spine(children, spec).and_then(|(_, a)| a).unwrap_or_default();
        if let Some((_, ai, _)) = path.anchors().find(|(_, _, a)| a.p.distance(p) <= tol) {
            if keys.contains(&ai) {
                return Some(vec![]);
            }
            return Some(vec![Action::Exec("object.blend.spine.removeAnchor".into(), json!({"id": id.0, "anchor": ai}))]);
        }
        if path.nearest(p).is_some_and(|n| n.4 <= tol) {
            return Some(vec![Action::Exec("object.blend.spine.addAnchor".into(), json!({"id": id.0, "x": p.x, "y": p.y}))]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn first_click_creates_path() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = PenTool::default();
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert_eq!(a[0], Action::Begin("Pen".into()));
        assert!(matches!(&a[1], Action::Preview(c, _) if c == "path.create"));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0)), vec![Action::Commit]);
    }

    #[test]
    fn clicking_a_selected_blend_spine_adds_a_point() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let key = |id: NodeId, x: f64| {
            let r = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
            std::sync::Arc::new(vectorcraft_doc::Node::path(id, r, vectorcraft_doc::Appearance::default_art()))
        };
        let (g, k1, k2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        d.insert(
            Some(l),
            1,
            vectorcraft_doc::Node::new(g, NodeKind::Blend { children: vec![key(k1, 100.0), key(k2, 200.0)], spec: Default::default() }),
        )
        .unwrap();
        let mut s = Selection::default();
        s.set([g]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = PenTool::default();
        assert_eq!(t.cursor(&cx, Point::new(160.0, 311.0), Mods::default()), Cursor::PenAdd);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 160.0, 311.0));
        assert_eq!(a, vec![Action::Exec("object.blend.spine.addAnchor".into(), json!({"id": g.0, "x": 160.0, "y": 310.0}))]);
        // A key's own point is left alone (the click snaps to the spine line).
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 310.0)), vec![]);
    }
}
