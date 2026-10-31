//! Picking and dragging mesh points and their handles on the canvas, shared by the Mesh tool and
//! Direct Selection: gradient meshes and mesh envelopes alike. A click on a point picks it alone
//! and focuses it: its handles show and can be dragged too. Shift-clicks add points to the picked
//! ones or take them away, and Direct Selection's marquee and its clicks on a gradient mesh's line
//! or patch pick points too ([`part_points`]). The picked points show filled; a drag of one of
//! them moves them all (`object.mesh.movePoint` for one, `object.mesh.movePoints` for several)
//! and the arrow keys nudge them. On a gradient mesh a solid fill colour recolours them and the
//! Transparency panel's opacity fades them (the tool's `meshPoints` option).

use std::borrow::Cow;
use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::live::{GradientMesh, envelope_grid};
use vectorcraft_doc::{EnvelopeKind, Node, NodeId, NodeKind};
use vectorcraft_geom::{Point, Vec2};

use crate::{Action, Mods, Overlay, ToolContext, ToolKey};

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

/// The points of gradient mesh `m` a Direct Selection click at `p` picks: the point within
/// `point_tol`, else the two ends of the mesh line within `line_tol`, else the four points around
/// the patch under `p`.
pub fn part_points(m: &GradientMesh, p: Point, point_tol: f64, line_tol: f64) -> Option<Vec<usize>> {
    if let Some(i) = m.point_near(p, point_tol) {
        return Some(vec![i]);
    }
    // `lines` lists the rows' edges, then the columns' (and nothing for a malformed mesh).
    let (rows, cols) = (m.rows.max(1) as usize, m.cols.max(1) as usize);
    if let Some((si, ..)) = m.lines().nearest(p).filter(|n| n.4 <= line_tol) {
        let across = (rows + 1) * cols;
        return Some(if si < across {
            let (r, c) = (si / cols, si % cols);
            vec![m.idx(r, c), m.idx(r, c + 1)]
        } else {
            let (c, r) = ((si - across) / rows, (si - across) % rows);
            vec![m.idx(r, c), m.idx(r + 1, c)]
        });
    }
    let (r, c, ..) = m.locate(p)?;
    Some(vec![m.idx(r, c), m.idx(r, c + 1), m.idx(r + 1, c), m.idx(r + 1, c + 1)])
}

/// What a press on a mesh grabs: the mesh, the point and (on the focused point) a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshGrab {
    pub id: NodeId,
    pub index: usize,
    pub handle: Option<usize>,
}

/// Mesh points, each mesh with its points.
type MeshPoints = Vec<(NodeId, Vec<usize>)>;

/// An in-progress mesh point or handle drag.
struct ActiveDrag {
    grab: MeshGrab,
    /// Where the press was.
    start: Point,
    /// Where the grabbed point or handle was when the drag began (where the press was, on a mesh
    /// line or patch).
    origin: Point,
    /// The current drag position (preview target); set on the first `drag_to`.
    at: Option<Point>,
    /// The picked points, when the drag moves several of them by its offset; empty when it moves
    /// `grab` alone to the pointer.
    many: MeshPoints,
}

/// The params of `object.mesh.movePoints` moving `items` by `d`.
fn move_params(items: &[(NodeId, Vec<usize>)], d: Vec2) -> Value {
    match items {
        [(id, points)] => json!({"id": id.0, "points": points, "dx": d.x, "dy": d.y}),
        _ => {
            let items: Vec<Value> = items.iter().map(|(id, v)| json!({"id": id.0, "points": v})).collect();
            json!({"items": items, "dx": d.x, "dy": d.y})
        }
    }
}

/// Mesh point picks and point and handle drags on meshes.
#[derive(Default)]
pub struct MeshEdit {
    /// The point last clicked: its handles show and can be dragged.
    focus: Option<(NodeId, usize)>,
    /// The picked points (the focused one among them).
    picked: BTreeSet<(NodeId, usize)>,
    drag: Option<ActiveDrag>,
    /// Pressed without Shift on points already picked among others: a click (no drag) picks
    /// these alone, as a drag moves every picked point.
    click: Option<(NodeId, Vec<usize>)>,
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

    /// The picked points that still apply: of editable meshes `shown` accepts, in range.
    fn live(&self, cx: &ToolContext, shown: &dyn Fn(NodeId) -> bool) -> MeshPoints {
        let editable = cx.doc.editable();
        let mut out: Vec<(NodeId, usize, Vec<usize>)> = vec![];
        for &(id, i) in &self.picked {
            if out.last().is_none_or(|(last, ..)| *last != id) {
                let len = if shown(id) && editable(id) { cx.doc.node(id).and_then(grid_of).map_or(0, |g| g.points.len()) } else { 0 };
                out.push((id, len, vec![]));
            }
            if let Some((_, len, v)) = out.last_mut()
                && i < *len
            {
                v.push(i);
            }
        }
        out.into_iter().filter(|(.., v)| !v.is_empty()).map(|(id, _, v)| (id, v)).collect()
    }

    /// A press at `p` on `grab`, among the meshes `ids`: a handle drag, or a point picked as
    /// [`Self::press_points`] picks it.
    pub fn press(&mut self, cx: &ToolContext, ids: &[NodeId], grab: MeshGrab, p: Point, shift: bool) -> Vec<Action> {
        let Some(h) = grab.handle else { return self.press_points(cx, ids, grab.id, &[grab.index], p, shift) };
        let Some(q) = cx.doc.node(grab.id).and_then(grid_of).and_then(|g| g.points.get(grab.index).cloned()) else { return vec![] };
        let origin = q.p + q.handles.get(h).copied().unwrap_or_default();
        self.start(ActiveDrag { grab, start: p, origin, at: None, many: vec![] })
    }

    /// A press at `p` on the points `points` of mesh `id` (one point, a mesh line's ends or a
    /// patch's corners), among the meshes `ids`. Shift takes them away from the picked points when
    /// all of them are picked (no drag), else adds them; without it they are picked alone unless
    /// all of them already are. A drag then moves every picked point.
    pub fn press_points(&mut self, cx: &ToolContext, ids: &[NodeId], id: NodeId, points: &[usize], p: Point, shift: bool) -> Vec<Action> {
        let keys: Vec<(NodeId, usize)> = points.iter().map(|i| (id, *i)).collect();
        let all = keys.iter().all(|k| self.picked.contains(k));
        self.click = None;
        if shift && all {
            for k in &keys {
                self.picked.remove(k);
            }
            self.focus = self.focus.filter(|f| !keys.contains(f));
            return vec![];
        }
        if !shift && !all {
            self.picked.clear();
        }
        self.picked.extend(keys.iter().copied());
        self.focus = keys.first().copied().filter(|_| keys.len() == 1);
        let many = self.live(cx, &|m| m == id || ids.contains(&m));
        let count: usize = many.iter().map(|(_, v)| v.len()).sum();
        if count == 0 {
            return vec![];
        }
        if !shift && all && count > keys.len() {
            self.click = Some((id, points.to_vec()));
        }
        let grab = MeshGrab { id, index: points.first().copied().unwrap_or_default(), handle: None };
        // One point goes where the pointer takes it; a line or a patch moves by the drag's offset.
        let origin = match points {
            [i] => cx.doc.node(id).and_then(grid_of).and_then(|g| g.points.get(*i).map(|q| q.p)).unwrap_or(p),
            _ => p,
        };
        let many = if count == 1 && keys.len() == 1 { vec![] } else { many };
        self.start(ActiveDrag { grab, start: p, origin, at: None, many })
    }

    fn start(&mut self, d: ActiveDrag) -> Vec<Action> {
        let label = if d.many.is_empty() { "Move Mesh Point" } else { "Move Mesh Points" };
        self.drag = Some(d);
        vec![Action::Begin(label.into())]
    }

    /// The preview of a drag to `p` (`None` when nothing is being dragged).
    pub fn drag_to(&mut self, p: Point) -> Option<Vec<Action>> {
        let d = self.drag.as_mut()?;
        d.at = Some(p);
        if !d.many.is_empty() {
            return Some(vec![Action::Preview("object.mesh.movePoints".into(), move_params(&d.many, p - d.origin))]);
        }
        let g = d.grab;
        let mut params = json!({"id": g.id.0, "index": g.index, "x": p.x, "y": p.y});
        if let Some(h) = g.handle {
            params["handle"] = json!(h);
        }
        Some(vec![Action::Preview("object.mesh.movePoint".into(), params)])
    }

    /// End a drag (`None` when nothing was being dragged). A click on one of several picked
    /// points picks it alone.
    pub fn release(&mut self, cx: &ToolContext) -> Option<Vec<Action>> {
        let d = self.drag.take()?;
        if let Some((id, points)) = self.click.take()
            && d.at.is_none_or(|a| a.distance(d.start) < cx.tol(3.0))
        {
            self.picked = points.into_iter().map(|i| (id, i)).collect();
        }
        Some(vec![Action::Commit])
    }

    /// Cancel a drag: the point or handle goes back (`None` when nothing was being dragged).
    pub fn cancel(&mut self) -> Option<Vec<Action>> {
        self.click = None;
        self.drag.take().map(|_| vec![Action::Cancel])
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Forget the focused and the picked points.
    pub fn unfocus(&mut self) {
        (self.focus, self.click) = (None, None);
        self.picked.clear();
    }

    /// A marquee's picks: the points `inside` it are picked (with `toggle`, those picked are
    /// taken away and the others added). Returns each mesh with points inside and whether any of
    /// its points are picked now.
    pub fn marquee(&mut self, inside: &[(NodeId, usize)], toggle: bool) -> Vec<(NodeId, bool)> {
        if !toggle {
            self.picked.clear();
        }
        for k in inside {
            if !(toggle && self.picked.remove(k)) {
                self.picked.insert(*k);
            }
        }
        self.focus = self.picked.first().copied().filter(|_| self.picked.len() == 1);
        let mut meshes: Vec<NodeId> = inside.iter().map(|(id, _)| *id).collect();
        meshes.dedup();
        meshes.into_iter().map(|id| (id, self.picked.range((id, 0)..=(id, usize::MAX)).next().is_some())).collect()
    }

    /// An arrow key's nudge of the picked points of the meshes `shown` accepts
    /// (`object.mesh.movePoints`); `None` during a drag, for other keys or without picked points.
    pub fn nudge(&self, cx: &ToolContext, key: ToolKey, mods: Mods, shown: &dyn Fn(NodeId) -> bool) -> Option<Vec<Action>> {
        if self.drag.is_some() {
            return None;
        }
        let d = cx.arrow_nudge(key, mods)?;
        let live = self.live(cx, shown);
        (!live.is_empty()).then(|| vec![Action::Exec("object.mesh.movePoints".into(), move_params(&live, d))])
    }

    /// `{meshPoints: [{id, index}…]}`: the picked points, which a solid fill colour recolours
    /// (`paint.setFill`) and the Transparency panel's opacity fades (`transparency.set`) on a
    /// gradient mesh.
    pub fn options(&self) -> Value {
        json!({"meshPoints": self.picked.iter().map(|(id, index)| json!({"id": id.0, "index": index})).collect::<Vec<_>>()})
    }

    /// The picked points of the meshes `shown` accepts, filled, and the focused point's handles
    /// (lines and ends).
    pub fn overlays(&self, cx: &ToolContext, shown: &dyn Fn(NodeId) -> bool) -> Vec<Overlay> {
        let mut out = vec![];
        for (id, points) in self.live(cx, shown) {
            let Some(grid) = cx.doc.node(id).and_then(grid_of) else { continue };
            out.extend(points.iter().filter_map(|i| grid.points.get(*i)).map(|q| Overlay::Anchor {
                p: q.p,
                color: FEEDBACK,
                filled: true,
                size: 5.0,
            }));
        }
        if let Some((id, index)) = self.focus.filter(|(id, _)| shown(*id))
            && let Some(grid) = cx.doc.node(id).and_then(grid_of)
            && let Some(q) = grid.points.get(index)
        {
            for h in q.handles.iter().filter(|h| h.hypot() > 1e-6) {
                out.push(Overlay::Line { a: q.p, b: q.p + *h, color: FEEDBACK, dashed: false });
                out.push(Overlay::Handle { p: q.p + *h, color: FEEDBACK });
            }
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

    /// Escape during a drag of a mesh point or handle, with the Mesh tool or Direct Selection,
    /// cancels it, and the release after it does nothing; the point stays the focused one.
    #[test]
    fn escape_cancels_a_mesh_point_drag() {
        let (d, e) = doc_with_envelope();
        let mut s = Selection::default();
        s.set([e]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let tools: [(&str, Box<dyn Tool>); 2] =
            [("mesh", crate::meshblend::create("mesh").unwrap()), ("directSelection", Box::new(crate::direct::DirectSelectionTool::new(false)))];
        for (what, mut t) in tools {
            // A point, then (once the point is focused) its handle.
            for (x, y) in [(200.0, 100.0), (200.0, 133.33333)] {
                assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y)), vec![Action::Begin("Move Mesh Point".into())], "{what}");
                t.pointer(&c, &PointerEvent::new(PointerKind::Drag, x + 10.0, y - 10.0));
                assert!(t.key(&c, crate::ToolKey::Enter, crate::Mods::default()).is_empty(), "{what}: Enter leaves the drag alone");
                assert!(t.busy(), "{what}");
                assert_eq!(t.key(&c, crate::ToolKey::Escape, crate::Mods::default()), vec![Action::Cancel], "{what}: Escape cancels the drag");
                assert!(!t.busy(), "{what}");
                assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, x + 20.0, y - 20.0)).is_empty(), "{what}: the drag is over");
                assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, x + 20.0, y - 20.0)).is_empty(), "{what}: the release does nothing");
                assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Handle { .. })), "{what}: the point stays focused");
            }
            assert!(t.key(&c, crate::ToolKey::Escape, crate::Mods::default()).is_empty(), "{what}: nothing to cancel");
        }
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

    /// A 2×2 gradient mesh over the 100..200 square, its points 50 apart.
    fn doc_with_mesh() -> (Document, NodeId) {
        let (mut d, a) = doc_with_rect();
        d.remove(a).unwrap();
        let id = d.alloc_id();
        let m = GradientMesh::from_surface(2, 2, &|u, v| Point::new(100.0 + 100.0 * u, 100.0 + 100.0 * v), &|_, _| vectorcraft_color::Color::WHITE);
        let l = d.layers[0].id;
        d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(m))).unwrap();
        (d, id)
    }

    /// The picked points of `t`, as its `meshPoints` option lists them.
    fn picked(t: &dyn Tool) -> Vec<usize> {
        t.options()["meshPoints"].as_array().unwrap().iter().map(|p| p["index"].as_u64().unwrap() as usize).collect()
    }

    fn shift(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        let mut e = PointerEvent::new(kind, x, y);
        e.mods.shift = true;
        e
    }

    /// With the Mesh tool and Direct Selection, Shift-clicks pick several mesh points (and take a
    /// picked one away); they show filled, a drag of one moves them all as one move, the arrow keys
    /// nudge them, and a click on one of them picks it alone (#4207).
    #[test]
    fn shift_clicks_pick_mesh_points_that_drag_and_nudge_together() {
        let (d, m) = doc_with_mesh();
        let mut s = Selection::default();
        s.set([m]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let at = |x: f64, y: f64| grid_of(d.node(m).unwrap()).unwrap().point_near(Point::new(x, y), 1.0).unwrap();
        let sorted = |mut v: Vec<usize>| {
            v.sort();
            v
        };
        let (a, b, e) = (at(100.0, 100.0), at(200.0, 150.0), at(150.0, 100.0));
        let tools: [(&str, Box<dyn Tool>); 2] =
            [("mesh", crate::meshblend::create("mesh").unwrap()), ("directSelection", Box::new(crate::direct::DirectSelectionTool::new(false)))];
        for (what, mut t) in tools {
            assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)), vec![Action::Begin("Move Mesh Point".into())], "{what}");
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, 100.0, 100.0));
            assert_eq!(t.pointer(&c, &shift(PointerKind::Down, 200.0, 150.0)), vec![Action::Begin("Move Mesh Points".into())], "{what}");
            t.pointer(&c, &shift(PointerKind::Up, 200.0, 150.0));
            assert_eq!(picked(&*t), sorted(vec![a, b]), "{what}");
            let filled = t.overlays(&c).iter().filter(|o| matches!(o, Overlay::Anchor { filled: true, .. })).count();
            assert_eq!(filled, 2, "{what}: the picked points show filled");
            // A drag of either moves both.
            assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)), vec![Action::Begin("Move Mesh Points".into())], "{what}");
            assert_eq!(
                t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 110.0, 95.0)),
                vec![Action::Preview("object.mesh.movePoints".into(), json!({"id": m.0, "points": sorted(vec![a, b]), "dx": 10.0, "dy": -5.0}))],
                "{what}"
            );
            assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 110.0, 95.0)), vec![Action::Commit], "{what}");
            assert_eq!(picked(&*t), sorted(vec![a, b]), "{what}: a drag keeps them picked");
            // Shift-clicking a picked point takes it away, without a drag.
            assert!(t.pointer(&c, &shift(PointerKind::Down, 200.0, 150.0)).is_empty(), "{what}");
            t.pointer(&c, &shift(PointerKind::Up, 200.0, 150.0));
            assert_eq!(picked(&*t), [a], "{what}");
            // The arrow keys nudge the picked points.
            assert!(t.claims_key(&c, ToolKey::Right), "{what}");
            assert_eq!(
                t.key(&c, ToolKey::Right, Mods::default()),
                vec![Action::Exec("object.mesh.movePoints".into(), json!({"id": m.0, "points": [a], "dx": 1.0, "dy": 0.0}))],
                "{what}"
            );
            // A click on one of several picked points picks it alone.
            t.pointer(&c, &shift(PointerKind::Down, 150.0, 100.0));
            t.pointer(&c, &shift(PointerKind::Up, 150.0, 100.0));
            assert_eq!(picked(&*t), sorted(vec![a, e]), "{what}");
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
            assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)), vec![Action::Commit], "{what}");
            assert_eq!(picked(&*t), [a], "{what}");
        }
    }

    /// Direct Selection's marquee picks the mesh points inside it and selects their mesh (Shift
    /// toggles them); a click on a mesh line picks its two ends and one inside a patch the four
    /// points around it, selecting the mesh, and a drag from there moves them (#4207).
    #[test]
    fn direct_selection_picks_mesh_points_by_marquee_line_and_patch() {
        let (d, m) = doc_with_mesh();
        let mut s = Selection::default();
        s.set([m]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let at = |x: f64, y: f64| grid_of(d.node(m).unwrap()).unwrap().point_near(Point::new(x, y), 1.0).unwrap();
        let set = |v: &[(f64, f64)]| {
            let mut v: Vec<usize> = v.iter().map(|(x, y)| at(*x, *y)).collect();
            v.sort();
            v
        };
        let mut t = crate::direct::DirectSelectionTool::new(false);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 120.0));
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 120.0));
        assert!(
            matches!(out.first(), Some(Action::Exec(cmd, q)) if cmd == "select.anchorsMany" && q["items"] == json!([{"id": m.0, "anchors": []}])),
            "{out:?}"
        );
        assert_eq!(picked(&t), set(&[(100.0, 100.0), (150.0, 100.0)]));
        // Shift: the picked points inside leave, the others join; the mesh stays selected.
        t.pointer(&c, &shift(PointerKind::Down, 90.0, 90.0));
        t.pointer(&c, &shift(PointerKind::Drag, 110.0, 160.0));
        let out = t.pointer(&c, &shift(PointerKind::Up, 110.0, 160.0));
        assert!(matches!(out.first(), Some(Action::Exec(cmd, q)) if cmd == "select.anchorsMany" && q["items"] == json!([])), "{out:?}");
        assert_eq!(picked(&t), set(&[(150.0, 100.0), (100.0, 150.0)]));
        // A mesh line: its two ends.
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 125.0, 150.0)), vec![Action::Begin("Move Mesh Points".into())]);
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 125.0, 150.0));
        assert_eq!(picked(&t), set(&[(100.0, 150.0), (150.0, 150.0)]));
        // A patch: the four points around it, which a drag moves by its offset.
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 175.0, 175.0)), vec![Action::Begin("Move Mesh Points".into())]);
        let four = set(&[(150.0, 150.0), (200.0, 150.0), (150.0, 200.0), (200.0, 200.0)]);
        assert_eq!(
            t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 185.0, 180.0)),
            vec![Action::Preview("object.mesh.movePoints".into(), json!({"id": m.0, "points": four, "dx": 10.0, "dy": 5.0}))]
        );
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 185.0, 180.0));
        // A mesh that isn't selected yet gets selected.
        let none = Selection::default();
        let c = cx(&d, &none, &p);
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 125.0, 125.0));
        assert_eq!(out, vec![Action::Exec("select.set".into(), json!({"ids": [m.0]})), Action::Begin("Move Mesh Points".into())]);
        assert_eq!(picked(&t), set(&[(100.0, 100.0), (150.0, 100.0), (100.0, 150.0), (150.0, 150.0)]));
    }
}
