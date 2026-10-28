//! Dragging mesh points and their handles on the canvas, shared by the Mesh tool and Direct
//! Selection: gradient meshes and mesh envelopes alike (`object.mesh.movePoint`). Clicking a
//! point focuses it: its handles show and can be dragged too, and on a gradient mesh a solid fill
//! colour recolours it (the tool's `meshPoint` option).

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_doc::live::{GradientMesh, envelope_grid};
use vectorcraft_doc::{EnvelopeKind, Node, NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::{Action, Overlay, ToolContext};

const FEEDBACK: [u8; 3] = [0x4f, 0x9d, 0xff];

/// The editable grid of `n`: a gradient mesh, or a mesh envelope's grid (not while its contents
/// are being edited).
pub fn grid_of(n: &Node) -> Option<Cow<'_, GradientMesh>> {
    match &n.kind {
        NodeKind::Mesh(m) => Some(Cow::Borrowed(m)),
        NodeKind::Envelope { kind, editing: false, .. } => match &**kind {
            EnvelopeKind::Mesh { rows, cols, points, handles } => envelope_grid(*rows, *cols, points, handles).map(Cow::Owned),
            _ => None,
        },
        _ => None,
    }
}

/// What a press on a mesh grabs: the mesh, the point and (on the focused point) a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshGrab {
    pub id: NodeId,
    pub index: usize,
    pub handle: Option<usize>,
}

/// An in-progress mesh point or handle drag.
struct ActiveDrag {
    grab: MeshGrab,
    /// Where the grabbed point or handle was when the drag began.
    origin: Point,
    /// The current drag position (preview target); set on the first `drag_to`.
    at: Option<Point>,
}

/// Point and handle drags on meshes.
#[derive(Default)]
pub struct MeshEdit {
    /// The point last clicked: its handles show and can be dragged.
    focus: Option<(NodeId, usize)>,
    /// The selected points (Shift-click, Direct Selection's marquee): dragging one moves them all.
    selected: Vec<(NodeId, usize)>,
    drag: Option<ActiveDrag>,
}

impl MeshEdit {
    /// The handle of the focused point, else the mesh point, under `p` among the meshes `ids`
    /// (editable ones, in order).
    pub fn hit(&self, cx: &ToolContext, ids: &[NodeId], p: Point) -> Option<MeshGrab> {
        let tol = cx.tol(5.0);
        if let Some((id, index)) = self.focus
            && ids.contains(&id)
            && let Some(grid) = cx.doc.node(id).and_then(grid_of)
            && let Some(q) = grid.points.get(index)
            && let Some(h) = (0..4).find(|h| q.handles[*h].hypot() > 1e-6 && (q.p + q.handles[*h]).distance(p) <= tol)
        {
            return Some(MeshGrab { id, index, handle: Some(h) });
        }
        let editable = cx.doc.editable();
        ids.iter().filter(|id| editable(**id)).find_map(|id| {
            let index = grid_of(cx.doc.node(*id)?)?.point_near(p, tol)?;
            Some(MeshGrab { id: *id, index, handle: None })
        })
    }

    /// Start dragging `grab` (its point takes the focus). A point joins the selected points (or,
    /// with `toggle`, leaves them and is not dragged); without `toggle` a point outside them
    /// becomes the only one.
    pub fn press(&mut self, cx: &ToolContext, grab: MeshGrab, toggle: bool) -> Vec<Action> {
        let key = (grab.id, grab.index);
        // Points the document lost since they were selected (deleted, undone…) drop out.
        self.selected.retain(|(id, i)| cx.doc.node(*id).and_then(grid_of).is_some_and(|g| *i < g.points.len()));
        if grab.handle.is_none() {
            let had = self.selected.contains(&key);
            if toggle && had {
                self.selected.retain(|k| *k != key);
                self.focus = None;
                return vec![];
            }
            if !toggle && !had {
                self.selected.clear();
            }
            if !had {
                self.selected.push(key);
            }
        }
        self.focus = Some(key);
        let mut origin = Point::ZERO;
        if let Some(grid) = cx.doc.node(grab.id).and_then(grid_of)
            && let Some(q) = grid.points.get(grab.index)
        {
            origin = if let Some(h) = grab.handle { q.p + q.handles[h] } else { q.p };
        }
        self.drag = Some(ActiveDrag { grab, origin, at: None });
        vec![Action::Begin("Move Mesh Point".into())]
    }

    /// The preview of a drag to `p` (`None` when nothing is being dragged).
    pub fn drag_to(&mut self, p: Point) -> Option<Vec<Action>> {
        let d = self.drag.as_mut()?;
        d.at = Some(p);
        let g = d.grab;
        let mut params = json!({"id": g.id.0, "index": g.index, "x": p.x, "y": p.y});
        if let Some(h) = g.handle {
            params["handle"] = json!(h);
        } else {
            let also: Vec<usize> = self.selected.iter().filter(|(id, i)| *id == g.id && *i != g.index).map(|(_, i)| *i).collect();
            if !also.is_empty() {
                params["also"] = json!(also);
            }
        }
        Some(vec![Action::Preview("object.mesh.movePoint".into(), params)])
    }

    /// End a drag (`None` when nothing was being dragged).
    pub fn release(&mut self) -> Option<Vec<Action>> {
        self.drag.take().map(|_| vec![Action::Commit])
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Forget the focused and the selected points.
    pub fn unfocus(&mut self) {
        self.focus = None;
        self.selected.clear();
    }

    /// Select the points of the meshes `ids` (editable ones) inside `r`, added to the selected
    /// points with `toggle` (toggled), else in their place; whether any point was inside.
    pub fn select_in(&mut self, cx: &ToolContext, ids: &[NodeId], r: Rect, toggle: bool) -> bool {
        let editable = cx.doc.editable();
        let mut inside = vec![];
        for id in ids.iter().filter(|id| editable(**id)) {
            let Some(grid) = cx.doc.node(*id).and_then(grid_of) else { continue };
            inside.extend(grid.points.iter().enumerate().filter(|(_, q)| r.contains(q.p)).map(|(i, _)| (*id, i)));
        }
        if !toggle {
            self.selected.clear();
        }
        for k in &inside {
            if let Some(at) = self.selected.iter().position(|s| s == k) {
                self.selected.remove(at);
            } else {
                self.selected.push(*k);
            }
        }
        self.focus = None;
        !inside.is_empty()
    }

    /// `{meshPoint: {id, index}|null}`: the focused point, which a solid fill colour recolours
    /// (`paint.setFill` on a gradient mesh).
    pub fn options(&self) -> Value {
        json!({"meshPoint": self.focus.map(|(id, index)| json!({"id": id.0, "index": index}))})
    }

    /// The selected points, and the focused point's handles (lines and ends).
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        for (id, index) in self.selected.iter().filter(|k| Some(**k) != self.focus) {
            if let Some(grid) = cx.doc.node(*id).and_then(grid_of)
                && let Some(q) = grid.points.get(*index)
            {
                out.push(Overlay::Anchor { p: q.p, color: FEEDBACK, filled: true, size: 5.0 });
            }
        }
        let Some((id, index)) = self.focus else { return out };
        let Some(grid) = cx.doc.node(id).and_then(grid_of) else { return out };
        let Some(q) = grid.points.get(index) else { return out };
        out.push(Overlay::Anchor { p: q.p, color: FEEDBACK, filled: true, size: 5.0 });
        for h in q.handles.iter().filter(|h| h.hypot() > 1e-6) {
            out.push(Overlay::Line { a: q.p, b: q.p + *h, color: FEEDBACK, dashed: false });
            out.push(Overlay::Handle { p: q.p + *h, color: FEEDBACK });
        }
        if cx.measurement_labels
            && let Some(d) = &self.drag
            && let Some(at) = d.at
        {
            let delta = at - d.origin;
            out.push(Overlay::Measure { p: at + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.offset_label(delta.x, delta.y) });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::testutil::*;
    use crate::{PointerEvent, PointerKind, Tool};
    use vectorcraft_doc::live::{EnvelopeOptions, grid_points};
    use vectorcraft_doc::{Document, Selection};
    use vectorcraft_geom::{Affine, Rect};

    /// The rectangle of [`doc_with_rect`] (100..200 square) in a 1×1 mesh envelope.
    fn doc_with_envelope() -> (Document, NodeId) {
        let (mut d, a) = doc_with_rect();
        let rect = d.node(a).cloned().unwrap();
        d.remove(a).unwrap();
        let id = d.alloc_id();
        let kind = EnvelopeKind::Mesh { rows: 1, cols: 1, points: grid_points(Rect::new(100.0, 100.0, 200.0, 200.0), 1, 1), handles: vec![] };
        let env = NodeKind::Envelope {
            content: vec![Arc::new(rect)],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: EnvelopeOptions::NEW,
            frame: Box::new(Affine::IDENTITY),
        };
        let l = d.layers[0].id;
        d.insert(Some(l), 0, Node::new(id, env)).unwrap();
        (d, id)
    }

    #[test]
    fn the_mesh_tool_drags_envelope_points_and_adds_lines() {
        let (d, e) = doc_with_envelope();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = crate::meshblend::create("mesh").unwrap();
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 100.0)), vec![Action::Begin("Move Mesh Point".into())]);
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 210.0, 90.0)),
            vec![Action::Preview("object.mesh.movePoint".into(), json!({"id": e.0, "index": 1, "x": 210.0, "y": 90.0}))]
        );
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 210.0, 90.0)), vec![Action::Commit]);
        // The clicked point shows its handles (the smooth mesh's) and they can be dragged.
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Handle { .. })));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 133.33333)), vec![Action::Begin("Move Mesh Point".into())]);
        let drag = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 220.0, 140.0));
        assert!(matches!(&drag[..], [Action::Preview(id, p)] if id == "object.mesh.movePoint" && p["handle"] == json!(2)), "{drag:?}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 220.0, 140.0));
        // Inside the envelope: a new row and column.
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 160.0)),
            vec![Action::Exec("object.mesh.addLine".into(), json!({"id": e.0, "x": 150.0, "y": 160.0}))]
        );
    }

    #[test]
    fn direct_selection_drags_a_selected_envelopes_points() {
        let (d, e) = doc_with_envelope();
        let mut s = Selection::default();
        let p = paint();
        let mut t = crate::direct::DirectSelectionTool::new(false);
        // Not selected: its points aren't handles to grab.
        let c = cx(&d, &s, &p);
        assert_ne!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 200.0)), vec![Action::Begin("Move Mesh Point".into())]);
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 100.0, 200.0));
        s.set([e]);
        let c = cx(&d, &s, &p);
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 200.0)), vec![Action::Begin("Move Mesh Point".into())]);
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 90.0, 210.0)),
            vec![Action::Preview("object.mesh.movePoint".into(), json!({"id": e.0, "index": 2, "x": 90.0, "y": 210.0}))]
        );
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 90.0, 210.0)), vec![Action::Commit]);
    }

    /// Dragging one of the mesh points a Direct Selection marquee picked moves them all, not the
    /// whole object (#4207).
    #[test]
    fn direct_selection_drags_marquee_selected_mesh_points_together() {
        let (mut d, a) = doc_with_rect();
        let grid = vectorcraft_doc::live::envelope_grid(1, 1, &grid_points(Rect::new(100.0, 100.0, 200.0, 200.0), 1, 1), &[]).unwrap();
        d.node_mut(a).unwrap().kind = NodeKind::Mesh(grid);
        let mut s = Selection::default();
        s.set([a]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = crate::direct::DirectSelectionTool::new(false);
        // A marquee around the top two points (0 and 1).
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 210.0, 110.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 210.0, 110.0));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)), vec![Action::Begin("Move Mesh Point".into())]);
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 120.0, 80.0)),
            vec![Action::Preview("object.mesh.movePoint".into(), json!({"id": a.0, "index": 0, "x": 120.0, "y": 80.0, "also": [1]}))]
        );
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 120.0, 80.0)), vec![Action::Commit]);
        // Shift-clicking a point adds it: point 3 joins, dragging it moves 0 and 1 too.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 200.0).with_mods(crate::Mods { shift: true, ..Default::default() }));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 200.0, 200.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 200.0));
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 210.0, 210.0)),
            vec![Action::Preview("object.mesh.movePoint".into(), json!({"id": a.0, "index": 3, "x": 210.0, "y": 210.0, "also": [0, 1]}))]
        );
    }

    /// Selected mesh points the mesh no longer has (it shrank after the marquee) are not sent
    /// with a drag, which the engine would reject as out of range (#4207).
    #[test]
    fn direct_selection_drag_drops_mesh_points_the_mesh_lost() {
        let (mut d, a) = doc_with_rect();
        let r = Rect::new(100.0, 100.0, 200.0, 200.0);
        let grid = vectorcraft_doc::live::envelope_grid(1, 2, &grid_points(r, 1, 2), &[]).unwrap();
        d.node_mut(a).unwrap().kind = NodeKind::Mesh(grid);
        let mut s = Selection::default();
        s.set([a]);
        let p = paint();
        let mut t = crate::direct::DirectSelectionTool::new(false);
        {
            let c = cx(&d, &s, &p);
            // A marquee around the bottom row (points 3, 4 and 5).
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, 90.0, 190.0));
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 210.0, 210.0));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, 210.0, 210.0));
        }
        // The mesh shrinks to 4 points (an undo, a deleted line…).
        let grid = vectorcraft_doc::live::envelope_grid(1, 1, &grid_points(r, 1, 1), &[]).unwrap();
        d.node_mut(a).unwrap().kind = NodeKind::Mesh(grid);
        let c = cx(&d, &s, &p);
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 200.0)), vec![Action::Begin("Move Mesh Point".into())]);
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 210.0, 190.0)),
            vec![Action::Preview("object.mesh.movePoint".into(), json!({"id": a.0, "index": 3, "x": 210.0, "y": 190.0}))]
        );
    }

    /// Group Selection's marquees select objects whole: they leave no mesh points selected, and
    /// clear the ones an earlier marquee picked (#4207).
    #[test]
    fn group_selection_marquees_leave_no_mesh_points_selected() {
        let (mut d, a) = doc_with_rect();
        let grid = vectorcraft_doc::live::envelope_grid(1, 1, &grid_points(Rect::new(100.0, 100.0, 200.0, 200.0), 1, 1), &[]).unwrap();
        d.node_mut(a).unwrap().kind = NodeKind::Mesh(grid);
        let mut s = Selection::default();
        s.set([a]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let mesh_point_shown = |o: &[Overlay]| o.iter().any(|o| matches!(o, Overlay::Anchor { p, .. } if *p == Point::new(100.0, 100.0)));
        let mut t = crate::direct::DirectSelectionTool::new(true);
        // A marquee around the top two points (0 and 1).
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 210.0, 110.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 210.0, 110.0));
        assert!(!mesh_point_shown(&t.overlays(&c)));
        // An enclosed marquee (E) around the whole mesh.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 250.0));
        t.key(&c, crate::ToolKey::Letter('e'), crate::Mods::default());
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 250.0, 250.0));
        assert!(!mesh_point_shown(&t.overlays(&c)));
    }

    #[test]
    fn mesh_drag_honors_measurement_labels() {
        let (d, _e) = doc_with_envelope();
        let (s, p) = (Selection::default(), paint());
        let base = cx(&d, &s, &p);
        let on = ToolContext { measurement_labels: true, ..base };
        let off = ToolContext { measurement_labels: false, ..base };
        let mut t = crate::meshblend::create("mesh").unwrap();
        t.pointer(&on, &PointerEvent::new(PointerKind::Down, 200.0, 100.0));
        assert!(!t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { .. })));
        t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 210.0, 90.0));
        let want = on.offset_label(10.0, -10.0);
        assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
        assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    }
}
