//! Direct Selection (A) and Group Selection tools.
//!
//! Direct Selection: click an anchor to select it (Shift toggles), click a segment to select the
//! path's anchors on that segment, drag to move selected anchors, drag a direction handle to
//! reshape (Shift keeps it at 45° steps round its anchor, Alt moves it alone), marquee to select
//! anchors, drag a live rectangle's corner widget to round its corners.
//! Group Selection: click selects the leaf; each further click on it adds the next enclosing group.
//! Both pick the key objects of a blend. Direct Selection also edits a blend's spine: drag its
//! points (a key object on a point moves with it) and, once a point is clicked, its handles; and
//! the points of selected gradient meshes and mesh envelopes and their handles ([`MeshEdit`]).
//! Dragging a corner or an edge of area type's frame reshapes the type area (`text.reshapeArea`):
//! the text reflows at its size.

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{AnchorRef, Node, NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, Rect};

use crate::bbox::move_delta;
use crate::corners::{CornerDrag, over_widget};
use crate::meshedit::MeshEdit;
use crate::select::{is_area_type, matrix_json};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Clone, Debug)]
enum State {
    Idle,
    MoveAnchors {
        start: Point,
        began: bool,
    },
    MoveObject {
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
        add: bool,
    },
    Corner(CornerDrag),
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
}

impl DirectSelectionTool {
    pub fn new(group: bool) -> Self {
        Self { group, state: State::Idle, spine: None, mesh: MeshEdit::default() }
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

/// Anchor of any visible path or area type frame under `p`, topmost first.
fn hit_anchor(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize)> {
    let owners = anchor_owners(cx, |n| matches!(n.kind, NodeKind::Path { .. }) || is_area_type(n));
    owners.into_iter().find_map(|id| {
        let pd = cx.doc.node(id).and_then(editable_path)?;
        pd.anchors().find(|(_, _, a)| a.p.distance(p) <= tol).map(|(si, ai, _)| (id, si, ai))
    })
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

/// Direction handle of a partially selected anchor under `p`: (id, si, ai, is_out).
fn hit_handle(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, bool)> {
    for (id, set) in &cx.selection.anchors {
        let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data()) else { continue };
        for &(si, ai) in set {
            let Some(a) = pd.subpaths.get(si).and_then(|s| s.anchors.get(ai)) else { continue };
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

/// Where a direction handle of anchor `ai` of subpath `si` of path `id` dragged to `p` goes:
/// Shift keeps it at a multiple of 45° around its anchor.
pub(crate) fn handle_at(cx: &ToolContext, id: NodeId, si: usize, ai: usize, p: Point, shift: bool) -> Point {
    let anchor = cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai)).map(|a| a.p);
    match anchor {
        Some(a) if shift => a + vectorcraft_geom::constrain_angle(p - a, 45.0),
        _ => p,
    }
}

fn anchors_json(v: &[AnchorRef]) -> Value {
    Value::Array(v.iter().map(|(s, a)| json!([s, a])).collect())
}

impl Tool for DirectSelectionTool {
    fn id(&self) -> &'static str {
        if self.group { "groupSelection" } else { "directSelection" }
    }
    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let tol = cx.tol(4.0);
        match (ev.kind, self.state.clone()) {
            (PointerKind::Down, _) if self.group => {
                let Some(h) = hit_test(cx.doc, p, cx.hit_options()) else {
                    self.state = State::Marquee { start: p, cur: p, add: ev.mods.shift };
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
            (PointerKind::Down, _) => {
                if let Some(c) = CornerDrag::hit(cx, p) {
                    self.state = State::Corner(c);
                    return vec![];
                }
                if let Some(sel) = self.spine.filter(|(id, _)| cx.selection.contains(*id))
                    && let Some(out) = hit_spine_handle(cx, sel, p, tol)
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
                if let Some((id, si, ai, out)) = hit_handle(cx, p, tol) {
                    self.state = State::Handle { id, si, ai, out };
                    return vec![Action::Begin("Reshape".into())];
                }
                if let Some((id, anchor, from)) = hit_spine_point(cx, p, tol) {
                    self.spine = Some((id, anchor));
                    self.state = State::SpinePoint { id, anchor, from, start: p, began: false };
                    return if cx.selection.contains(id) { vec![] } else { vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))] };
                }
                if let Some((id, si, ai)) = hit_anchor(cx, p, tol) {
                    if cx.doc.node(id).is_some_and(is_area_type) {
                        return self.press_type_area(cx, id, vec![(si, ai)], p, ev.mods.shift);
                    }
                    let already = cx.selection.partial(id).is_some_and(|s| s.contains(&(si, ai)));
                    self.state = State::MoveAnchors { start: p, began: false };
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
                if let Some(h) = hit_test(cx.doc, p, cx.hit_options()) {
                    // Clicking a segment/fill selects the whole leaf path (all anchors).
                    self.state = State::MoveObject { start: p, began: false };
                    if ev.mods.shift {
                        return vec![Action::Exec("select.toggle".into(), json!({"id": h.leaf.0}))];
                    }
                    if !cx.selection.contains(h.leaf) || cx.selection.partial(h.leaf).is_some() {
                        return vec![Action::Exec("select.set".into(), json!({"ids": [h.leaf.0]}))];
                    }
                    return vec![];
                }
                self.state = State::Marquee { start: p, cur: p, add: ev.mods.shift };
                vec![]
            }
            (PointerKind::Drag, State::MoveAnchors { start, began }) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Move".into()));
                    self.state = State::MoveAnchors { start, began: true };
                }
                let d = move_delta(start, p, ev.mods.shift);
                out.push(Action::Preview("path.moveAnchors".into(), json!({"dx": d.x, "dy": d.y})));
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
                }
                let d = move_delta(start, p, ev.mods.shift);
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
                let p = handle_at(cx, id, si, ai, p, ev.mods.shift);
                vec![Action::Preview(
                    "path.setHandle".into(),
                    json!({"id": id.0, "subpath": si, "anchor": ai, "which": if out {"out"} else {"in"}, "x": p.x, "y": p.y, "independent": ev.mods.alt}),
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
            (PointerKind::Drag, State::Marquee { start, add, .. }) => {
                self.state = State::Marquee { start, cur: p, add };
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
            (
                PointerKind::Up,
                State::MoveAnchors { began, .. } | State::MoveObject { began, .. } | State::SpinePoint { began, .. } | State::TypeArea { began, .. },
            ) => {
                self.state = State::Idle;
                if began { vec![Action::Commit] } else { vec![] }
            }
            (PointerKind::Up, State::Handle { .. } | State::SpineHandle { .. }) => {
                self.state = State::Idle;
                vec![Action::Commit]
            }
            (PointerKind::Up, State::Marquee { start, add, .. }) => {
                self.state = State::Idle;
                let r = Rect::from_points(start, p);
                if r.width() < cx.tol(3.0) && r.height() < cx.tol(3.0) {
                    return if add { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
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
                sel.retain(|(id, _)| cx.doc.is_editable(*id));
                let items: Vec<Value> = sel.iter().map(|(id, v)| json!({"id": id.0, "anchors": anchors_json(v)})).collect();
                vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "add": add}))]
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
                }
                out
            }
        }
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if !self.group && (matches!(self.state, State::Corner(_)) || over_widget(cx, p)) {
            return Cursor::CornerRadius;
        }
        Cursor::ArrowHollow
    }
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

    /// A smooth anchor at (350, 150), handles at 300 and 400 across, its anchor selected.
    fn smooth_doc() -> (vectorcraft_doc::Document, NodeId, Selection) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(250.0, 250.0), Point::new(350.0, 150.0), Point::new(450.0, 250.0)], false);
        sp.anchors[1].h_in = Point::new(300.0, 150.0);
        sp.anchors[1].h_out = Point::new(400.0, 150.0);
        sp.anchors[1].kind = vectorcraft_geom::AnchorKind::Smooth;
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, PathData::single(sp), vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        s.anchors.insert(id, [(0, 1)].into_iter().collect());
        (d, id, s)
    }

    #[test]
    fn shift_keeps_a_dragged_handle_at_45_degree_steps() {
        let (d, id, s) = smooth_doc();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let set = |x: f64, y: f64| {
            Action::Preview(
                "path.setHandle".into(),
                json!({"id": id.0, "subpath": 0, "anchor": 1, "which": "out", "x": x, "y": y, "independent": false}),
            )
        };
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = DirectSelectionTool::new(false);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 150.0)), vec![Action::Begin("Reshape".into())]);
        // Without Shift the handle follows the pointer; with it, it stays level (10° rounds to 0°)…
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 160.0)), vec![set(450.0, 160.0)]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 160.0).with_mods(shift));
        let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
        let (x, y) = (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap());
        assert!((x - (350.0 + 100.0_f64.hypot(10.0))).abs() < 1e-9 && (y - 150.0).abs() < 1e-9, "{v}");
        // …or diagonal (40° rounds to 45°).
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 234.0).with_mods(shift));
        let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
        let (dx, dy) = (v["x"].as_f64().unwrap() - 350.0, v["y"].as_f64().unwrap() - 150.0);
        assert!((dx - dy).abs() < 1e-9 && dx > 0.0, "{v}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 450.0, 234.0)), vec![Action::Commit]);
        // The Anchor Point tool's handle drags too.
        let mut t = crate::create("anchorPoint");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 150.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 160.0).with_mods(shift));
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        assert!((v["y"].as_f64().unwrap() - 150.0).abs() < 1e-9 && v["x"].as_f64().unwrap() > 400.0, "{v}");
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
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "add": false}))]);
    }
}
