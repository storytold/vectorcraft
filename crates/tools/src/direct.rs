//! Direct Selection (A) and Group Selection tools.
//!
//! Direct Selection: click an anchor to select it (Shift toggles), click a segment (an edge) to
//! select just that segment's two anchors, click the fill to select the whole path, drag to move selected anchors, drag a direction handle to
//! reshape, marquee to select the anchors inside it and the segments it cuts through, drag a live
//! rectangle's corner widget to round its corners (just the picked corners when there are some).
//! Group Selection: click selects the leaf; each further click on it adds the next enclosing group.
//! Both pick the key objects of a blend. Direct Selection also edits a blend's spine: drag its
//! points (a key object on a point moves with it) and, once a point is clicked, its handles; and
//! the points of selected gradient meshes and mesh envelopes and their handles ([`MeshEdit`]).

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{AnchorRef, NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, Rect};

use crate::bbox::move_delta;
use crate::corners::{CornerDrag, over_widget};
use crate::meshedit::MeshEdit;
use crate::select::matrix_json;
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

/// Anchor (or handle) of any selected/visible path under `p`.
/// The topmost editable path with a segment within `tol` of `p`: (path, subpath, the segment's
/// two anchors).
fn hit_segment(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, usize)> {
    segment_at(cx.doc, p, tol)
}

/// The segment the Direct Selection tool would pick at `p` (within `tol`, document units):
/// (path, subpath, the segment's two anchors). The canvas highlights it under the pointer.
pub fn segment_at(doc: &vectorcraft_doc::Document, p: Point, tol: f64) -> Option<(NodeId, usize, usize, usize)> {
    let mut ids = vec![];
    doc.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. }) && n.visible {
            ids.push(n.id)
        }
    });
    // A blend's key objects are picked whole (their anchors still select one by one).
    let in_blend = |id: NodeId| {
        doc.ancestry(id).is_some_and(|a| a.iter().any(|x| *x != id && matches!(doc.node(*x).map(|n| &n.kind), Some(NodeKind::Blend { .. }))))
    };
    ids.into_iter().rev().filter(|id| doc.is_editable(*id) && !in_blend(*id)).find_map(|id| {
        let pd = doc.node(id)?.path_data()?;
        let (si, seg, _, _, d) = pd.nearest(p)?;
        let n = pd.subpaths.get(si)?.anchors.len();
        (d <= tol && n >= 2).then_some((id, si, seg, (seg + 1) % n))
    })
}

fn hit_anchor(cx: &ToolContext, p: Point, tol: f64, selected_only: bool) -> Option<(NodeId, usize, usize)> {
    let ids: Vec<NodeId> = if selected_only {
        cx.selection.objects.clone()
    } else {
        let mut v = vec![];
        cx.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Path { .. }) {
                v.push(n.id)
            }
        });
        v.reverse();
        v
    };
    for id in ids {
        if !cx.doc.is_editable(id) {
            continue;
        }
        if let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) {
            for (si, ai, a) in pd.anchors() {
                if a.p.distance(p) <= tol {
                    return Some((id, si, ai));
                }
            }
        }
    }
    None
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

/// Does the curve pass through `r`? (Sampled: chords of 1/32 of the curve, each clipped to `r`.)
fn segment_crosses(c: &vectorcraft_geom::CubicBez, r: Rect) -> bool {
    use vectorcraft_geom::ParamCurve;
    let pts: Vec<Point> = (0..=32).map(|i| c.eval(i as f64 / 32.0)).collect();
    pts.windows(2).any(|w| match w {
        [a, b] => chord_meets_rect(*a, *b, r),
        _ => false,
    })
}

/// Liang–Barsky: does the segment a–b touch `r`?
fn chord_meets_rect(a: Point, b: Point, r: Rect) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-d.x, a.x - r.x0), (d.x, r.x1 - a.x), (-d.y, a.y - r.y0), (d.y, r.y1 - a.y)] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    t0 <= t1
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
                if let Some((id, si, ai)) = hit_anchor(cx, p, tol, false) {
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
                // Clicking an edge selects just that segment (its two anchors), as in the
                // reference app; dragging then moves only that edge.
                if let Some((id, si, a0, a1)) = hit_segment(cx, p, tol) {
                    self.state = State::MoveAnchors { start: p, began: false };
                    let anchors = json!([[si, a0], [si, a1]]);
                    let segments = json!([[si, a0]]);
                    let selected = cx.selection.partial(id).is_some_and(|s| s.contains(&(si, a0)) && s.contains(&(si, a1)))
                        && cx.selection.segments_of(cx.doc, id).contains(&(si, a0, a1));
                    if ev.mods.shift {
                        return vec![Action::Exec(
                            "select.anchors".into(),
                            json!({"id": id.0, "anchors": anchors, "segments": segments, "mode": "toggle"}),
                        )];
                    }
                    if !selected {
                        return vec![Action::Exec(
                            "select.anchors".into(),
                            json!({"id": id.0, "anchors": anchors, "segments": segments, "mode": "set"}),
                        )];
                    }
                    return vec![];
                }
                if let Some(h) = hit_test(cx.doc, p, cx.hit_options()) {
                    // Clicking the fill selects the whole leaf path (all anchors).
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
            (PointerKind::Drag, State::Handle { id, si, ai, out }) => {
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
            (PointerKind::Up, State::MoveAnchors { began, .. } | State::MoveObject { began, .. } | State::SpinePoint { began, .. }) => {
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
                // Collect the anchors inside the rect for every editable path, and the parts of
                // shapes it crosses: a segment the marquee cuts through with neither end inside
                // is picked by its two anchors, so a drag moves just that segment.
                let mut sel: Vec<(NodeId, Vec<AnchorRef>, Vec<AnchorRef>)> = vec![];
                cx.doc.walk(|n| {
                    if let NodeKind::Path { path, .. } = &n.kind {
                        let inside: Vec<AnchorRef> = path.anchors().filter(|(_, _, a)| r.contains(a.p)).map(|(s, i, _)| (s, i)).collect();
                        let mut v = inside.clone();
                        let mut segs = vec![];
                        for (si, sp) in path.subpaths.iter().enumerate() {
                            let n = sp.anchors.len();
                            for seg in 0..sp.segment_count() {
                                let (a0, a1) = (seg, (seg + 1) % n.max(1));
                                if inside.contains(&(si, a0)) || inside.contains(&(si, a1)) || !segment_crosses(&sp.segment(seg), r) {
                                    continue;
                                }
                                v.extend([(si, a0), (si, a1)]);
                                segs.push((si, a0));
                            }
                        }
                        v.sort_unstable();
                        v.dedup();
                        if !v.is_empty() {
                            sel.push((n.id, v, segs));
                        }
                    }
                });
                sel.retain(|(id, _, _)| cx.doc.is_editable(*id));
                let items: Vec<Value> = sel
                    .iter()
                    .map(|(id, v, segs)| {
                        if segs.is_empty() {
                            json!({"id": id.0, "anchors": anchors_json(v)})
                        } else {
                            json!({"id": id.0, "anchors": anchors_json(v), "segments": anchors_json(segs)})
                        }
                    })
                    .collect();
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

impl DirectSelectionTool {
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
    fn clicking_an_edge_selects_just_that_segment_and_the_fill_the_whole_path() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        // The rectangle runs 100..200: (150, 100) is on its top edge.
        let a = DirectSelectionTool::new(false).pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 100.5));
        let [Action::Exec(cmd, v)] = a.as_slice() else { panic!("{a:?}") };
        assert_eq!(cmd, "select.anchors");
        assert_eq!((v["id"].as_u64(), v["anchors"].as_array().map(Vec::len), v["mode"].as_str()), (Some(id.0), Some(2), Some("set")));
        let pts: Vec<(f64, f64)> = v["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                let (si, ai) = (a[0].as_u64().unwrap() as usize, a[1].as_u64().unwrap() as usize);
                let q = d.node(id).unwrap().path_data().unwrap().subpaths[si].anchors[ai].p;
                (q.x, q.y)
            })
            .collect();
        assert!(pts.iter().all(|(_, y)| *y == 100.0), "the two anchors of the top edge: {pts:?}");
        // Inside, on the fill: the whole path.
        let a = DirectSelectionTool::new(false).pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
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

    #[test]
    fn marquee_across_an_edge_picks_that_segment() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        // A box over the middle of the top edge (100,100)–(200,100), no anchor inside.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 140.0, 90.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.0, 110.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 160.0, 110.0));
        assert_eq!(
            a,
            vec![Action::Exec(
                "select.anchorsMany".into(),
                json!({"items": [{"id": id.0, "anchors": [[0, 0], [0, 1]], "segments": [[0, 0]]}], "add": false})
            )]
        );
        // Around one corner: just that anchor, not the far ends of its two edges.
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 110.0, 110.0));
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 0]]}], "add": false}))]);
        // Touching nothing: nothing picked.
        let mut t = DirectSelectionTool::new(false);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 320.0, 320.0));
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [], "add": false}))]);
    }

    #[test]
    fn every_kind_of_shape_picks_a_segment_by_click_and_by_marquee() {
        use vectorcraft_doc::{Appearance, Node};
        use vectorcraft_geom::shapes;
        let r = Rect::new(100.0, 100.0, 200.0, 200.0);
        let open = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(
            &[Point::new(100.0, 150.0), Point::new(200.0, 150.0), Point::new(200.0, 250.0)],
            false,
        ));
        for (name, path) in [
            ("ellipse", shapes::ellipse(r)),
            ("polygon", shapes::polygon(Point::new(150.0, 150.0), 50.0, 6, 0.0)),
            ("star", shapes::star(Point::new(150.0, 150.0), 50.0, 25.0, 5, 0.0)),
            ("rounded rectangle", shapes::rounded_rectangle(r, 20.0)),
            ("open path", open),
        ] {
            let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
            let l = d.layers[0].id;
            let id = d.alloc_id();
            d.insert(Some(l), 0, Node::path(id, path.clone(), Appearance::default_art())).unwrap();
            let s = Selection::default();
            let p = paint();
            let cx = cx(&d, &s, &p);
            // A point in the middle of the first segment.
            let sp = &path.subpaths[0];
            let mid = vectorcraft_geom::ParamCurve::eval(&sp.segment(0), 0.5);
            let pair = json!([[0, 0], [0, 1 % sp.anchors.len()]]);
            let first = json!([[0, 0]]);
            let mut t = DirectSelectionTool::new(false);
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, mid.x, mid.y));
            assert_eq!(
                a,
                vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": pair, "segments": first, "mode": "set"}))],
                "{name}: click"
            );
            // A small marquee across it, started outside the shape (a press on the fill or the
            // stroke would pick those instead).
            let seg = sp.segment(0);
            let chord = seg.p3 - seg.p0;
            let mut out = vectorcraft_geom::Vec2::new(-chord.y, chord.x).normalize();
            if out.dot(mid - Point::new(150.0, 150.0)) <= 0.0 {
                out = -out;
            }
            let side = vectorcraft_geom::Vec2::new(-out.y, out.x) * 3.0;
            let (from, to) = (mid + out * 8.0 + side, mid - out * 3.0 - side);
            let mut t = DirectSelectionTool::new(false);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, from.x, from.y));
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, to.x, to.y));
            assert_eq!(
                a,
                vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": pair, "segments": first}], "add": false}))],
                "{name}: marquee"
            );
        }
    }
}
