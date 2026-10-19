//! Width tool (Shift+W).
//!
//! Hovering a stroked path shows a hollow width-point diamond with the stroke's width at that
//! spot. Dragging outward from the path creates (or, on an existing width point's handle end,
//! edits) a width point — symmetric by default, Alt changes only the side being dragged. Dragging
//! a width point's centre slides it along the path; Alt-dragging the centre slides a copy; dropping
//! either onto another width point makes a discontinuous point (the width steps there). Shift-click
//! adds width points to the selection (or takes them out), and dragging one of several selected
//! points moves or widens them all. Double-clicking a stroke opens Width Point Edit (the
//! `widthPoint` dialog), editing a hit point or creating one on confirmation. Delete/Backspace
//! removes the selected width points.
//!
//! Gestures preview `stroke.widthPoint.set {id, t, left, right, index?}` (side widths in points),
//! `stroke.widthPoint.copy {id, index, t}` or, for several points, `stroke.widthProfile.set {ids,
//! points}`, and commit on release; Delete runs `stroke.widthPoint.remove {id, indices}`.
//!
//! A compound path's stroke is the compound's: its members are edited through it, and the width
//! points show on every subpath of the stroke (one profile runs along each). The cursor says what a
//! drag does: add a width point on a stroke, or move or widen the one under it.

use serde_json::json;
use vectorcraft_doc::{Document, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Point, Vec2};

use super::pathutil::{eval_fraction, left_normal, nearest_fraction};
use super::{BLUE, diamond};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// Width points closer than this (in t) are at the same place (the two sides of a discontinuous
/// point).
const SAME_T: f64 = 1e-6;

/// Where the pointer is relative to a stroked path.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    /// The object whose stroke it is (a compound path for its members).
    owner: NodeId,
    /// The path under the pointer.
    id: NodeId,
    sub: usize,
    /// Fraction along the subpath.
    t: f64,
    p: Point,
    /// Left normal.
    n: Vec2,
    /// Side widths (points) at `t`.
    left: f64,
    right: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Change widths; `Some(left?)` = only one side (Alt).
    Width(Option<bool>),
    /// Slide along the path.
    Move,
    /// Slide a copy along the path (Alt-drag the centre).
    Copy,
}

/// A width point as the tool shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WPoint {
    index: usize,
    t: f64,
    /// On-path point and left normal.
    p: Point,
    n: Vec2,
    /// Side widths (points).
    left: f64,
    right: f64,
}

/// Several selected width points dragged together.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    /// The profile's points when the drag began.
    points: Vec<(f64, f64, f64)>,
    /// Which of them move.
    moving: Vec<usize>,
    /// The stroke's half weight.
    half: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct Drag {
    /// Where the dragged point started (`t`, widths); `p` and `n` follow a slide.
    spot: Spot,
    index: Option<usize>,
    mode: Mode,
    left: f64,
    right: f64,
    t: f64,
    group: Option<Group>,
    /// The other width points of the subpath when the drag began, which a slid point snaps onto.
    others: Vec<WPoint>,
    moved: bool,
}

#[derive(Default)]
pub struct WidthTool {
    hover: Option<Spot>,
    drag: Option<Drag>,
    /// Selected width points: the stroke's owner and their positions t.
    selected: Option<(NodeId, Vec<f64>)>,
}

/// The object whose stroke path `id` draws with: its compound path, if it is a member of one.
pub fn stroke_owner(doc: &Document, id: NodeId) -> NodeId {
    doc.parent_of(id).filter(|p| doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))).unwrap_or(id)
}

/// The visible weighted stroke of `owner`.
fn stroke_of(doc: &Document, owner: NodeId) -> Option<&StrokeLayer> {
    doc.node(owner)?.appearance.stroke().filter(|s| s.visible && s.width > 0.0 && !s.paint.is_none())
}

/// The paths a stroke owner draws: itself, or a compound path's members.
fn paths_of(doc: &Document, owner: NodeId) -> Vec<NodeId> {
    let mut out = vec![];
    if let Some(n) = doc.node(owner) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { guide: false, .. }) {
                out.push(c.id);
            }
        });
    }
    out
}

/// Side widths (points) of a stroke at fraction `t`.
pub fn sides_at(st: &StrokeLayer, t: f64) -> (f64, f64) {
    let (l, r) = st.profile.as_ref().map(|p| p.at(t)).unwrap_or((1.0, 1.0));
    (l * st.width / 2.0, r * st.width / 2.0)
}

impl WidthTool {
    /// The stroked path nearest to `p` within reach (stroke half-width + a few pixels).
    fn spot_at(cx: &ToolContext, p: Point) -> Option<Spot> {
        let consider = |id: NodeId| -> Option<(f64, Spot)> {
            if !cx.doc.is_editable(id) {
                return None;
            }
            let n = cx.doc.node(id)?;
            let NodeKind::Path { path, guide: false, .. } = &n.kind else { return None };
            let owner = stroke_owner(cx.doc, id);
            let st = stroke_of(cx.doc, owner)?;
            let widest = st.profile.as_ref().map_or(1.0, |pr| pr.points.iter().map(|q| q.1.max(q.2)).fold(1.0, f64::max));
            let reach = widest * st.width / 2.0 + cx.tol(4.0);
            if !path.bounds()?.inflate(reach, reach).contains(p) {
                return None;
            }
            let (sub, t, q, tan, d) = nearest_fraction(path, p)?;
            if d > reach {
                return None;
            }
            let (left, right) = sides_at(st, t);
            Some((d, Spot { owner, id, sub, t, p: q, n: left_normal(tan), left, right }))
        };
        let best_of = |ids: &mut dyn Iterator<Item = NodeId>| ids.filter_map(consider).min_by(|a, b| a.0.total_cmp(&b.0));
        // The selected paths first (a selected compound path: its members).
        if let Some(b) = best_of(&mut cx.selection.objects.iter().flat_map(|id| paths_of(cx.doc, *id))) {
            return Some(b.1);
        }
        let mut all = vec![];
        cx.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Path { .. }) {
                all.push(n.id);
            }
        });
        best_of(&mut all.into_iter().rev()).map(|b| b.1)
    }

    /// Width points of `owner`'s stroke on subpath `sub` of path `id`.
    fn points_of(cx: &ToolContext, owner: NodeId, id: NodeId, sub: usize) -> Vec<WPoint> {
        let Some(st) = stroke_of(cx.doc, owner) else { return vec![] };
        let Some(pr) = &st.profile else { return vec![] };
        let Some(NodeKind::Path { path, .. }) = cx.doc.node(id).map(|n| &n.kind) else { return vec![] };
        let Some(sp) = path.subpaths.get(sub) else { return vec![] };
        pr.points
            .iter()
            .enumerate()
            .filter_map(|(index, (t, l, r))| {
                let (p, tan) = eval_fraction(sp, *t)?;
                Some(WPoint { index, t: *t, p, n: left_normal(tan), left: l * st.width / 2.0, right: r * st.width / 2.0 })
            })
            .collect()
    }

    /// Width points of `owner`'s stroke on every subpath it draws.
    fn all_points(cx: &ToolContext, owner: NodeId) -> Vec<WPoint> {
        let subpaths = |id: NodeId| match cx.doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::Path { path, .. }) => path.subpaths.len(),
            _ => 0,
        };
        paths_of(cx.doc, owner).into_iter().flat_map(|id| (0..subpaths(id)).flat_map(move |sub| Self::points_of(cx, owner, id, sub))).collect()
    }

    /// The width point part nearest to `p` within a few pixels: a centre (`None`) or the end of a
    /// left (`Some(true)`) or right (`Some(false)`) handle.
    fn hit(cx: &ToolContext, spot: &Spot, p: Point) -> Option<(WPoint, Option<bool>)> {
        let tol = cx.tol(5.0);
        Self::points_of(cx, spot.owner, spot.id, spot.sub)
            .into_iter()
            .flat_map(|w| [(w, None, w.p), (w, Some(true), w.p + w.n * w.left), (w, Some(false), w.p - w.n * w.right)])
            .map(|(w, part, q)| (q.distance(p), w, part))
            .filter(|h| h.0 <= tol)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, w, part)| (w, part))
    }

    fn is_selected(&self, id: NodeId, t: f64) -> bool {
        self.selected.as_ref().is_some_and(|(sid, ts)| *sid == id && ts.iter().any(|s| (s - t).abs() <= SAME_T))
    }

    /// Shift-click on the width point at `t` of `id`: add it to the selection or take it out.
    /// → whether it is selected now.
    fn toggle(&mut self, id: NodeId, t: f64) -> bool {
        match &mut self.selected {
            Some((sid, ts)) if *sid == id => match ts.iter().position(|s| (s - t).abs() <= SAME_T) {
                Some(i) => {
                    ts.remove(i);
                    false
                }
                None => {
                    ts.push(t);
                    true
                }
            },
            _ => {
                self.selected = Some((id, vec![t]));
                true
            }
        }
    }

    /// The indices of the selected width points of `id` (both sides of a discontinuous point).
    fn selected_indices(&self, cx: &ToolContext, id: NodeId) -> Vec<usize> {
        let Some(pr) = stroke_of(cx.doc, id).and_then(|s| s.profile.as_ref()) else { return vec![] };
        (0..pr.points.len()).filter(|i| self.is_selected(id, pr.points[*i].0)).collect()
    }

    /// A drag of `id`'s selected points together: a move when more than one profile point is
    /// selected (a discontinuous point moves as one), a width change when several are.
    fn group(&self, cx: &ToolContext, id: NodeId, mode: Mode) -> Option<Group> {
        let st = stroke_of(cx.doc, id)?;
        let moving = self.selected_indices(cx, id);
        let several = match mode {
            Mode::Move => moving.len() > 1,
            Mode::Width(_) => self.selected.as_ref().is_some_and(|(_, ts)| ts.len() > 1),
            Mode::Copy => false,
        };
        several.then(|| Group { points: st.profile.as_ref().map(|p| p.points.clone()).unwrap_or_default(), moving, half: st.width / 2.0 })
    }

    /// The command previewing drag `d`.
    fn preview(d: &Drag) -> Action {
        let id = d.spot.owner.0;
        if let Some(g) = &d.group {
            let (dt, dl, dr) = (d.t - d.spot.t, (d.left - d.spot.left) / g.half, (d.right - d.spot.right) / g.half);
            let mut pts: Vec<[f64; 3]> = g
                .points
                .iter()
                .enumerate()
                .map(|(i, &(t, l, r))| match (g.moving.contains(&i), d.mode) {
                    (true, Mode::Move) => [(t + dt).clamp(0.0, 1.0), l, r],
                    (true, _) => [t, (l + dl).max(0.0), (r + dr).max(0.0)],
                    (false, _) => [t, l, r],
                })
                .collect();
            pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
            return Action::Preview("stroke.widthProfile.set".into(), json!({"ids": [id], "points": pts}));
        }
        match (d.mode, d.index) {
            (Mode::Copy, Some(i)) => Action::Preview("stroke.widthPoint.copy".into(), json!({"id": id, "index": i, "t": d.t})),
            (_, index) => {
                let mut v = json!({"id": id, "t": d.t, "left": d.left, "right": d.right});
                if let Some(i) = index {
                    v["index"] = json!(i);
                }
                Action::Preview("stroke.widthPoint.set".into(), v)
            }
        }
    }
}

impl Tool for WidthTool {
    fn id(&self) -> &'static str {
        "width"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = Self::spot_at(cx, p);
                vec![]
            }
            PointerKind::DoubleClick => {
                let Some(spot) = Self::spot_at(cx, p) else { return vec![] };
                let Some((w, _)) = Self::hit(cx, &spot, p) else {
                    self.selected = Some((spot.owner, vec![spot.t]));
                    return vec![Action::Dialog("widthPoint".into(), json!({"id": spot.owner.0, "t": spot.t}))];
                };
                self.selected = Some((spot.owner, vec![w.t]));
                vec![Action::Dialog("widthPoint".into(), json!({"id": spot.owner.0, "index": w.index, "adjustAdjoining": ev.mods.shift}))]
            }
            PointerKind::Down => {
                let Some(spot) = Self::spot_at(cx, p) else {
                    if !ev.mods.shift {
                        self.selected = None;
                    }
                    return vec![];
                };
                let drag = match Self::hit(cx, &spot, p) {
                    Some((w, part)) => {
                        let mode = match part {
                            None if ev.mods.alt => Mode::Copy,
                            None => Mode::Move,
                            Some(left) => Mode::Width(ev.mods.alt.then_some(left)),
                        };
                        if ev.mods.shift {
                            if !self.toggle(spot.owner, w.t) {
                                return vec![];
                            }
                        } else if mode == Mode::Copy || !self.is_selected(spot.owner, w.t) {
                            self.selected = Some((spot.owner, vec![w.t]));
                        }
                        let others = Self::points_of(cx, spot.owner, spot.id, spot.sub).into_iter().filter(|o| o.index != w.index).collect();
                        let group = self.group(cx, spot.owner, mode);
                        let spot = Spot { t: w.t, p: w.p, n: w.n, left: w.left, right: w.right, ..spot };
                        Drag { spot, index: Some(w.index), mode, left: w.left, right: w.right, t: w.t, group, others, moved: false }
                    }
                    None => {
                        let side = (p - spot.p).dot(spot.n) >= 0.0;
                        self.selected = Some((spot.owner, vec![spot.t]));
                        let mode = Mode::Width(ev.mods.alt.then_some(side));
                        Drag { spot, index: None, mode, left: spot.left, right: spot.right, t: spot.t, group: None, others: vec![], moved: false }
                    }
                };
                let label = if drag.mode == Mode::Copy { "Copy Width Point" } else { "Width Point" };
                self.drag = Some(drag);
                vec![Action::Begin(label.into())]
            }
            PointerKind::Drag => {
                let Some(d) = &mut self.drag else { return vec![] };
                d.moved = true;
                match d.mode {
                    Mode::Width(side) => {
                        let off = (p - d.spot.p).dot(d.spot.n);
                        match side {
                            None => {
                                d.left = off.abs();
                                d.right = off.abs();
                            }
                            Some(true) => d.left = off.max(0.0),
                            Some(false) => d.right = (-off).max(0.0),
                        }
                    }
                    Mode::Move | Mode::Copy => {
                        let Some(NodeKind::Path { path, .. }) = cx.doc.node(d.spot.id).map(|n| &n.kind) else { return vec![] };
                        let Some(sp) = path.subpaths.get(d.spot.sub) else { return vec![] };
                        let Some((_, t, q, tan, _)) = nearest_fraction(&vectorcraft_geom::PathData::single(sp.clone()), p) else { return vec![] };
                        (d.t, d.spot.p, d.spot.n) = (t, q, left_normal(tan));
                        // Dropped onto another width point (a single point only): it lands exactly
                        // there, making a discontinuous point.
                        let tol = cx.tol(5.0);
                        if d.group.is_none()
                            && let Some(o) = d.others.iter().find(|o| o.p.distance(q) <= tol)
                        {
                            (d.t, d.spot.p, d.spot.n) = (o.t, o.p, o.n);
                        }
                    }
                }
                vec![Self::preview(d)]
            }
            PointerKind::Up => match self.drag.take() {
                Some(d) => {
                    let dt = d.t - d.spot.t;
                    self.selected = match (d.group.is_some(), d.moved, self.selected.take()) {
                        // A click on one of several selected points selects just that one.
                        (true, false, _) if !ev.mods.shift => Some((d.spot.owner, vec![d.spot.t])),
                        // Points moved together stay selected.
                        (true, _, Some((id, ts))) if d.mode == Mode::Move => Some((id, ts.into_iter().map(|t| (t + dt).clamp(0.0, 1.0)).collect())),
                        (true, _, sel) => sel,
                        (false, _, _) => Some((d.spot.owner, vec![d.t])),
                    };
                    vec![Action::Commit]
                }
                None => vec![],
            },
        }
    }
    /// Delete/Backspace remove the selected width points; with none selected they are the
    /// shortcut's (Clear deletes the selected objects).
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Delete | ToolKey::Backspace)
            && self.drag.is_none()
            && self.selected.as_ref().is_some_and(|(id, _)| !self.selected_indices(cx, *id).is_empty())
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if !matches!(key, ToolKey::Delete | ToolKey::Backspace) {
            return vec![];
        }
        let Some(id) = self.selected.as_ref().map(|s| s.0) else { return vec![] };
        let indices = self.selected_indices(cx, id);
        if indices.is_empty() {
            return vec![];
        }
        self.selected = None;
        vec![Action::Exec("stroke.widthPoint.remove".into(), json!({"id": id.0, "indices": indices}))]
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        let r = cx.tol(4.0);
        let focus = self.drag.as_ref().map(|d| d.spot).or(self.hover);
        // Every subpath's width points of the stroke under the pointer, and of the selected ones'.
        let show = |owner: NodeId, out: &mut Vec<Overlay>| {
            for w in Self::all_points(cx, owner) {
                let sel = self.is_selected(owner, w.t);
                out.push(Overlay::Line { a: w.p + w.n * w.left, b: w.p - w.n * w.right, color: BLUE, dashed: false });
                out.push(Overlay::Path { path: diamond(w.p, r), color: BLUE, width: if sel { 2.5 } else { 1.0 }, dashed: false });
                out.push(Overlay::Handle { p: w.p + w.n * w.left, color: BLUE });
                out.push(Overlay::Handle { p: w.p - w.n * w.right, color: BLUE });
            }
        };
        if let Some(s) = focus {
            show(s.owner, &mut out);
        }
        if let Some((owner, _)) = &self.selected
            && focus.is_none_or(|s| s.owner != *owner)
        {
            show(*owner, &mut out);
        }
        if let Some(d) = &self.drag {
            let q = d.spot.p;
            out.push(Overlay::Line { a: q + d.spot.n * d.left, b: q - d.spot.n * d.right, color: BLUE, dashed: false });
            out.push(Overlay::Path { path: diamond(q, r), color: BLUE, width: 2.0, dashed: false });
            if cx.measurement_labels {
                out.push(Overlay::Measure { p: q + d.spot.n * (d.left + r * 3.0), text: format!("W: {}", cx.stroke_unit.readout(d.left + d.right)) });
            }
        } else if let Some(h) = &self.hover {
            out.push(Overlay::Line { a: h.p + h.n * h.left, b: h.p - h.n * h.right, color: BLUE, dashed: true });
            out.push(Overlay::Path { path: diamond(h.p, r), color: BLUE, width: 1.0, dashed: false });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.drag.is_some() {
            return Cursor::WidthPoint;
        }
        // The stroke under the pointer, as the last move found it (no search every frame).
        match self.hover {
            Some(spot) if Self::hit(cx, &spot, p).is_some() => Cursor::WidthPoint,
            Some(_) => Cursor::WidthAdd,
            None => Cursor::Width,
        }
    }
    fn busy(&self) -> bool {
        self.drag.is_some()
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.hover = None;
        if self.drag.take().is_some() { vec![Action::Commit] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Node, Selection, WidthProfile};
    use vectorcraft_geom::Point;
    use vectorcraft_geom::{PathData, SubPath};

    fn doc_line() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut ap = Appearance::default_art();
        ap.stroke_mut().unwrap().width = 10.0;
        let path = PathData::single(SubPath::polyline(&[Point::new(100.0, 200.0), Point::new(300.0, 200.0)], false));
        d.insert(Some(l), 0, Node::path(id, path, ap)).unwrap();
        (d, id)
    }

    /// The line with width points at t = 0.25, 0.5 and 0.75 (x = 150, 200, 250; 5 pt either side,
    /// the stroke's own width).
    fn doc_three_points() -> (Document, NodeId) {
        let (mut d, id) = doc_line();
        let points = vec![(0.0, 1.0, 1.0), (0.25, 1.0, 1.0), (0.5, 1.0, 1.0), (0.75, 1.0, 1.0), (1.0, 1.0, 1.0)];
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points });
        (d, id)
    }

    fn ev(kind: PointerKind, x: f64, y: f64, mods: Mods) -> PointerEvent {
        PointerEvent::new(kind, x, y).with_mods(mods)
    }

    fn preview(acts: &[Action]) -> (&str, &serde_json::Value) {
        match acts {
            [Action::Preview(cmd, v)] => (cmd.as_str(), v),
            _ => panic!("{acts:?}"),
        }
    }

    #[test]
    fn drag_outward_creates_symmetric_width_point() {
        let (d, id) = doc_line();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 201.0)), vec![Action::Begin("Width Point".into())]);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 220.0));
        let (cmd, v) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set");
        assert_eq!(v["id"], json!(id.0));
        assert!((v["t"].as_f64().unwrap() - 0.25).abs() < 1e-3);
        assert!((v["left"].as_f64().unwrap() - 20.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 20.0).abs() < 1e-6);
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 150.0, 220.0)), vec![Action::Commit]);
    }

    #[test]
    fn alt_drag_changes_one_side_and_move_slides() {
        let (mut d, id) = doc_line();
        let n = d.node_mut(id).unwrap();
        n.appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        // Alt-drag the left handle end (y = 200 - 10) further up.
        let alt = Mods { alt: true, ..Default::default() };
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 190.0).with_mods(alt));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 200.0, 170.0).with_mods(alt));
        let (_, v) = preview(&acts);
        assert_eq!(v["index"], json!(1));
        assert!((v["left"].as_f64().unwrap() - 30.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 10.0).abs() < 1e-6);
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 200.0, 170.0));
        let del = t.key(&c, ToolKey::Delete, Mods::default());
        assert_eq!(del, vec![Action::Exec("stroke.widthPoint.remove".into(), json!({"id": id.0, "indices": [1]}))]);
        // Drag the centre along the path.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 200.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 205.0));
        let (_, v) = preview(&acts);
        assert!((v["t"].as_f64().unwrap() - 0.75).abs() < 1e-3);
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 250.0, 205.0));
        assert!(t.selected.as_ref().is_some_and(|(_, ts)| (ts[0] - 0.75).abs() < 1e-3), "the moved point stays selected");
    }

    #[test]
    fn shift_click_selects_several_points_that_drag_together() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Default::default() });
        // Click the point at t = 0.25, Shift-click the one at 0.75.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 200.0, none));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 250.0, 200.0, shift)), vec![Action::Begin("Width Point".into())]);
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 200.0, shift));
        assert_eq!(t.selected_indices(&c, id), vec![1, 3]);
        // Dragging one of them 20 pt along moves both by a tenth of the path, as one command.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 170.0, 203.0, none));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let ts: Vec<f64> = v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 1000.0).round() / 1000.0).collect();
        assert_eq!(ts, vec![0.0, 0.35, 0.5, 0.85, 1.0]);
        t.pointer(&c, &ev(PointerKind::Up, 170.0, 203.0, none));
        let (_, sel) = t.selected.clone().unwrap();
        assert!((sel[0] - 0.35).abs() < 1e-3 && (sel[1] - 0.85).abs() < 1e-3, "the selection moves with them: {sel:?}");
        // Widening one handle widens every selected point by as much.
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.75])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 250.0, 195.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 250.0, 190.0, none));
        let pts = preview(&acts).1["points"].as_array().unwrap().clone();
        let w = |i: usize, side: usize| (pts[i][side].as_f64().unwrap() * 1e6).round() / 1e6;
        assert_eq!((w(1, 1), w(3, 2), w(2, 1)), (2.0, 2.0, 1.0));
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 190.0, none));
        // Shift-clicking a selected point takes it out again (and drags nothing).
        assert!(t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, shift)).is_empty());
        assert_eq!(t.selected_indices(&c, id), vec![3]);
        // A plain click on one of several selected points selects just that one.
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.75])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 250.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 200.0, none));
        assert_eq!(t.selected_indices(&c, id), vec![3]);
    }

    #[test]
    fn alt_drag_copies_and_dropping_onto_a_point_snaps_to_it() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, alt)), vec![Action::Begin("Copy Width Point".into())]);
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 180.0, 200.0, alt));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["id"], &v["index"]), ("stroke.widthPoint.copy", &json!(id.0), &json!(1)));
        assert!((v["t"].as_f64().unwrap() - 0.4).abs() < 1e-3);
        t.pointer(&c, &ev(PointerKind::Up, 180.0, 200.0, alt));
        // Sliding the point at 0.25 to within a few pixels of the one at 0.5 lands exactly on it.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 197.0, 200.0, Mods::default()));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, v["t"].as_f64(), v["index"].as_u64()), ("stroke.widthPoint.set", Some(0.5), Some(1)));
    }

    /// A compound path of two 200 pt lines (y = 200 and 300) with a 10 pt stroke on the compound
    /// (its members carry none), a width point at t = 0.5 → (doc, compound, second member).
    fn doc_compound() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (c, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let line = |id, y| {
            let path = PathData::single(SubPath::polyline(&[Point::new(100.0, y), Point::new(300.0, y)], false));
            std::sync::Arc::new(Node::path(id, path, Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0)))
        };
        let mut comp = Node::new(c, NodeKind::Compound { children: vec![line(a, 200.0), line(b, 300.0)], rule: Default::default() });
        comp.appearance = Appearance::default_art().into();
        let st = comp.appearance.stroke_mut().unwrap();
        st.width = 10.0;
        st.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (1.0, 1.0, 1.0)] });
        d.insert(Some(l), 0, comp).unwrap();
        (d, c, b)
    }

    #[test]
    fn compound_path_strokes_are_edited_through_the_compound() {
        let (d, c, b) = doc_compound();
        assert_eq!(stroke_owner(&d, b), c);
        let (s, p) = (Selection { objects: vec![c], ..Default::default() }, paint());
        let cx = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        // Drag out from the second member: the compound's stroke gets the width point.
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 301.0, Mods::default())), vec![Action::Begin("Width Point".into())]);
        let acts = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 320.0, Mods::default()));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["id"]), ("stroke.widthPoint.set", &json!(c.0)));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 320.0, Mods::default()));
        // The width point in the middle of the first member selects (and deletes) the compound's.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Up, 200.0, 200.0, Mods::default()));
        assert!(t.claims_key(&cx, ToolKey::Delete));
        assert_eq!(
            t.key(&cx, ToolKey::Delete, Mods::default()),
            vec![Action::Exec("stroke.widthPoint.remove".into(), json!({"id": c.0, "indices": [1]}))]
        );
    }

    #[test]
    fn width_points_show_on_every_subpath() {
        let (d, c, _) = doc_compound();
        let (s, p) = (Selection { objects: vec![c], ..Default::default() }, paint());
        let cx = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let handles = |t: &WidthTool| t.overlays(&cx).iter().filter(|o| matches!(o, Overlay::Handle { .. })).count();
        // Hovering one member shows the 3 width points on both (two handle ends each).
        t.pointer(&cx, &ev(PointerKind::Move, 150.0, 302.0, Mods::default()));
        assert_eq!(handles(&t), 3 * 2 * 2);
        // A selected point shows them on every subpath when the pointer is elsewhere.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Move, 450.0, 450.0, Mods::default()));
        assert_eq!(handles(&t), 3 * 2 * 2);
        // One path of two subpaths: both.
        let (mut d2, id) = doc_line();
        let n = d2.node_mut(id).unwrap();
        if let NodeKind::Path { path, .. } = &mut n.kind {
            path.subpaths.push(SubPath::polyline(&[Point::new(100.0, 260.0), Point::new(300.0, 260.0)], false));
        }
        n.appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)] });
        let s2 = Selection { objects: vec![id], ..Default::default() };
        let cx2 = crate::testutil::cx(&d2, &s2, &p);
        let mut t = WidthTool::default();
        t.pointer(&cx2, &ev(PointerKind::Down, 200.0, 260.0, Mods::default()));
        t.pointer(&cx2, &ev(PointerKind::Up, 200.0, 260.0, Mods::default()));
        t.pointer(&cx2, &ev(PointerKind::Move, 450.0, 450.0, Mods::default()));
        let ends: Vec<Point> = t.overlays(&cx2).iter().filter_map(|o| if let Overlay::Handle { p, .. } = o { Some(*p) } else { None }).collect();
        assert!(ends.iter().any(|q| q.y < 230.0) && ends.iter().any(|q| q.y > 230.0), "both subpaths: {ends:?}");
    }

    #[test]
    fn the_cursor_says_what_a_drag_does() {
        let (d, _) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        assert_eq!(t.cursor(&c, Point::new(400.0, 400.0), Mods::default()), Cursor::Width);
        t.pointer(&c, &ev(PointerKind::Move, 170.0, 201.0, Mods::default()));
        assert_eq!(t.cursor(&c, Point::new(170.0, 201.0), Mods::default()), Cursor::WidthAdd, "on the stroke: adds a point");
        t.pointer(&c, &ev(PointerKind::Move, 150.0, 200.0, Mods::default()));
        assert_eq!(t.cursor(&c, Point::new(150.0, 200.0), Mods::default()), Cursor::WidthPoint, "on a width point");
        assert_eq!(t.cursor(&c, Point::new(150.0, 195.0), Mods::default()), Cursor::WidthPoint, "on a handle end");
    }

    #[test]
    fn double_click_on_a_width_point_opens_width_point_edit() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        for shift in [false, true] {
            let mut t = WidthTool::default();
            let mods = Mods { shift, ..Default::default() };
            let acts = t.pointer(&c, &ev(PointerKind::DoubleClick, 200.0, 195.0, mods));
            let [Action::Dialog(kind, p)] = acts.as_slice() else { panic!("expected Width Point Edit: {acts:?}") };
            assert_eq!(kind, "widthPoint");
            assert_eq!((&p["id"], &p["index"]), (&json!(id.0), &json!(2)));
            assert_eq!(p.get("adjustAdjoining").and_then(serde_json::Value::as_bool).unwrap_or(false), shift);
            assert!(t.pointer(&c, &ev(PointerKind::DoubleClick, 120.0, 250.0, mods)).is_empty(), "not on a stroked path");
        }
    }
}
