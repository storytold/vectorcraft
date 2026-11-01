//! Smart Pen Tool (Smart Pen add-on): one tool that draws paths and edits them without switching
//! tools or holding a key for it.
//!
//! Drawing: a click adds the next anchor with a straight or a curved segment into it, as the
//! `next` option says (the Smart Pen panel's choice; a curve after a straight segment leaves it
//! tangent); a drag makes a smooth anchor (Alt held breaks the outgoing handle off, Shift keeps it
//! to 45° steps, Space moves the anchor); Tab switches the segment into the anchor being placed, or
//! the last one drawn, between straight and curved. A rubber band shows the next segment. A click
//! on the first anchor closes the path, on an end of another open path joins the two, and away
//! from paths, with no path being drawn, starts a new one. Enter or Esc ends the path.
//!
//! Editing the shown paths' points: a click selects a point (an end of an open path is then drawn
//! on from), Shift-click adds it to the selection or takes it out; a drag moves it (Shift: 45°);
//! Alt-drag pulls out its outgoing handle; Alt-click removes it keeping the shape, Shift+Alt-click
//! deletes it leaving the handles beside it; a double-click toggles it between smooth and corner.
//! A selected point's handle drags (Shift: 45°, Alt: keeps its angle); a double-click on it toggles
//! its point. A segment drags to reshape it (Shift: handles square to its chord, Alt: handles keep
//! their angles), Alt-click adds a point on it; a click on a path that isn't selected selects it.
//! With no path being drawn, an Alt-drag away from paths draws a marquee that selects the points
//! inside it (Shift: toggles them).
//!
//! Each gesture is one undo step (`addon.pen.*`, and `path.*` where the standard command does it).

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Anchor, BezPath, Point, Rect, SubPath, Vec2, constrain_angle_from};
use vectorcraft_pathops::addons::pathtools::SMOOTH_RATIO;
use vectorcraft_pathops::addons::pen::{leave_handle, straighten_segment};

use crate::direct::hit_handle;
use crate::draw2::{anchor_in, hit_segment, segment_in, shown_paths};
use crate::guides::{DrawSnap, Leave};
use crate::pen::{active_path, continue_from, open_end_at};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "smartPen";
/// Screen pixels the pointer must move after a press before it drags instead of clicking.
const DRAG_THRESHOLD: f64 = 3.0;

#[derive(Default)]
pub struct SmartPenTool {
    /// A click adds a curved segment (else a straight one): the `next` option.
    curved: bool,
    /// A path is being drawn...
    drawing: bool,
    /// ...this subpath of this path, once the engine made it.
    path: Option<(NodeId, usize)>,
    /// The press under way.
    press: Option<Press>,
    hover: Option<Point>,
    /// The pointer at the last press or drag (Space moves an anchor being placed by its moves).
    last: Point,
    snap: DrawSnap,
    /// An Alt-drag's marquee: where it started, where it is, and whether it toggles (Shift).
    marquee: Option<(Point, Point, bool)>,
}

/// A press: where it went down, what it is on and whether it has become a drag (and started its
/// undo step).
struct Press {
    at: Point,
    mods: Mods,
    dragged: bool,
    begun: bool,
    what: Target,
}

/// An anchor being placed at `at`, its handles `h_in` and `h_out`, on `path` (none: the first
/// anchor of a new path); `seg` is the segment into it (straight, curved, or as its handles make
/// it), `tabbed` once Tab chose it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    at: Point,
    h_in: Point,
    h_out: Point,
    path: Option<(NodeId, usize)>,
    seg: Option<bool>,
    tabbed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Place(Place),
    /// Closing the path being drawn on its first anchor, at `first`.
    Close {
        id: NodeId,
        si: usize,
        first: Point,
    },
    /// An anchor of a shown path at `p`; `n` anchors in its subpath, `closed` or not.
    Point {
        id: NodeId,
        si: usize,
        ai: usize,
        p: Point,
        n: usize,
        closed: bool,
    },
    /// A handle of a selected anchor at `anchor`, the handle at `h`.
    Handle {
        id: NodeId,
        si: usize,
        ai: usize,
        out: bool,
        anchor: Point,
        h: Point,
    },
    /// A segment, grabbed at `t`.
    Segment {
        id: NodeId,
        si: usize,
        seg: usize,
        t: f64,
    },
}

/// What lies under the pointer, in the order a press takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// The first anchor of the path being drawn: a click closes it.
    Close(NodeId, usize, Point),
    /// A handle of a selected anchor: (path, subpath, anchor, outgoing).
    Handle(NodeId, usize, usize, bool),
    /// An anchor of a shown path.
    Anchor(NodeId, usize, usize),
    /// An end of another open path, while one is drawn: a click joins the two.
    Join(NodeId, usize, bool),
    /// A segment: (path, subpath, segment, t).
    Segment(NodeId, usize, usize, f64),
    /// An end of an open path, while none is drawn: a click draws on from it.
    Continue(NodeId, usize, bool),
    Nothing,
}

fn xy(p: Point) -> Value {
    json!([p.x, p.y])
}

/// Is `id` selected as the path drawn on: itself, or the compound path it is in?
fn selected_alone(cx: &ToolContext, id: NodeId) -> bool {
    let sel = cx.selection.objects.as_slice();
    sel == [id] || cx.doc.parent_of(id).is_some_and(|p| sel == [p] && cx.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })))
}

impl SmartPenTool {
    /// The marquee's part of a pointer event: an Alt-press away from paths, with no path being
    /// drawn, starts it; drags move its corner; the release selects the points of the editable
    /// paths inside it (`select.anchorsMany`; Shift: toggled). `None`: the event isn't its.
    fn marquee_event(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        match (ev.kind, self.marquee) {
            (PointerKind::Down, None) if ev.mods.alt && self.active(cx).is_none() && self.hit(cx, ev.pos) == Hit::Nothing => {
                self.marquee = Some((ev.pos, ev.pos, ev.mods.shift));
                Some(vec![])
            }
            (PointerKind::Drag | PointerKind::Move, Some((a, _, toggle))) => {
                self.marquee = Some((a, ev.pos, toggle));
                Some(vec![])
            }
            (PointerKind::Up, Some((a, _, toggle))) => {
                self.marquee = None;
                let r = Rect::from_points(a, ev.pos);
                // A click, not a drag: nothing to select.
                if r.width() < cx.tol(DRAG_THRESHOLD) && r.height() < cx.tol(DRAG_THRESHOLD) {
                    return Some(vec![]);
                }
                let editable = cx.doc.editable();
                let mut items = vec![];
                crate::draw2::walk_pickable(cx, |n| {
                    if let NodeKind::Path { path, .. } = &n.kind
                        && editable(n.id)
                    {
                        let inside: Vec<Value> = path.anchors().filter(|(_, _, a)| r.contains(a.p)).map(|(s, i, _)| json!([s, i])).collect();
                        if !inside.is_empty() {
                            items.push(json!({"id": n.id.0, "anchors": inside}));
                        }
                    }
                });
                Some(match (items.is_empty(), toggle) {
                    (true, true) => vec![],
                    (true, false) => vec![Action::Exec("select.none".into(), json!({}))],
                    (false, _) => {
                        vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": if toggle { "toggle" } else { "set" }}))]
                    }
                })
            }
            _ => None,
        }
    }

    /// The open subpath being drawn: (path, subpath, its data), while it is selected and editable.
    fn active<'a>(&self, cx: &'a ToolContext) -> Option<(NodeId, usize, &'a SubPath)> {
        let (id, si) = self.path.filter(|_| self.drawing)?;
        if !selected_alone(cx, id) || !cx.doc.is_editable(id) {
            return None;
        }
        let sp = cx.doc.node(id)?.path_data()?.subpaths.get(si)?;
        (!sp.closed && !sp.anchors.is_empty()).then_some((id, si, sp))
    }

    /// The path a press began is the selected one by the next event.
    fn sync(&mut self, cx: &ToolContext) {
        if self.drawing
            && self.path.is_none()
            && let Some((id, si, ..)) = active_path(cx)
        {
            self.path = Some((id, si));
        }
    }

    fn stop(&mut self) {
        (self.drawing, self.path) = (false, None);
    }

    /// What a press at `p` is on.
    fn hit(&self, cx: &ToolContext, p: Point) -> Hit {
        let tol = cx.tol(5.0).max(cx.point_tol());
        let point = cx.point_tol();
        let active = self.active(cx);
        if let Some((id, si, sp)) = active
            && sp.anchors.len() > 1
            && let (Some(first), Some(last)) = (sp.anchors.first(), sp.anchors.last())
            && first.p.distance(p) <= tol
            && first.p.distance(p) <= last.p.distance(p)
        {
            return Hit::Close(id, si, first.p);
        }
        if let Some((id, si, ai, out)) = hit_handle(cx, p, point) {
            return Hit::Handle(id, si, ai, out);
        }
        let shown = shown_paths(cx);
        if let Some((id, si, ai)) = anchor_in(cx, shown.iter().copied(), p, point) {
            return Hit::Anchor(id, si, ai);
        }
        if let Some((id, si, _)) = active
            && let Some((other, osi, first)) = open_end_at(cx, p, tol, Some((id, si)))
        {
            return Hit::Join(other, osi, first);
        }
        if active.is_none()
            && let Some((id, si, first)) = open_end_at(cx, p, tol, None)
        {
            return Hit::Continue(id, si, first);
        }
        // Drawing, only the shown paths' segments take a press; else any path's.
        let seg = if active.is_some() { segment_in(cx, shown, p, cx.pick_tol()) } else { hit_segment(cx, p, cx.pick_tol()) };
        seg.map_or(Hit::Nothing, |(id, si, seg, t)| Hit::Segment(id, si, seg, t))
    }

    /// The command that previews the anchor being placed: with its handles once `dragged`.
    fn place_preview(&self, pl: &Place, dragged: bool) -> Action {
        let Place { at, h_in, h_out, path, seg, tabbed } = *pl;
        let Some((id, si)) = path else {
            let mut a = json!({"x": at.x, "y": at.y});
            if dragged {
                (a["in"], a["out"]) = (xy(h_in), xy(h_out));
            }
            return Action::Preview("path.create".into(), json!({"anchors": [a]}));
        };
        let mut p = json!({"id": id.0, "subpath": si, "x": at.x, "y": at.y});
        if dragged {
            (p["in"], p["out"]) = (xy(h_in), xy(h_out));
        }
        // A click adds the segment the option asks for; a drag shapes it with its handles.
        let seg = if tabbed || dragged { seg } else { Some(self.curved) };
        if let Some(curved) = seg {
            p["segment"] = json!(if curved { "curved" } else { "straight" });
        }
        Action::Preview("addon.pen.append".into(), p)
    }

    /// Where the drag puts handle `h` of the anchor at `anchor`: Shift turns it in 45° steps; with
    /// `keep` it keeps the direction of `was` and only its length follows.
    fn handle_to(cx: &ToolContext, anchor: Point, pos: Point, was: Point, shift: bool, keep: bool) -> Point {
        let v = pos - anchor;
        let dir = was - anchor;
        if keep && dir.hypot() > 1e-9 {
            let u = dir / dir.hypot();
            return anchor + u * v.dot(u).max(0.0);
        }
        if shift { anchor + constrain_angle_from(v, 45.0, cx.constrain_angle) } else { pos }
    }

    fn press_down(&mut self, cx: &ToolContext, ev: &PointerEvent, p: Point) -> Vec<Action> {
        let press = |what| Press { at: ev.pos, mods: ev.mods, dragged: false, begun: false, what };
        match self.hit(cx, ev.pos) {
            Hit::Close(id, si, first) => {
                self.press = Some(Press { begun: true, ..press(Target::Close { id, si, first }) });
                let seg = if self.curved { "curved" } else { "straight" };
                vec![
                    Action::Begin("Close Path".into()),
                    Action::Preview("addon.pen.close".into(), json!({"id": id.0, "subpath": si, "segment": seg})),
                ]
            }
            Hit::Handle(id, si, ai, out) => {
                let Some(a) = cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai).copied()) else {
                    return vec![];
                };
                self.press = Some(press(Target::Handle { id, si, ai, out, anchor: a.p, h: if out { a.h_out } else { a.h_in } }));
                vec![]
            }
            Hit::Anchor(id, si, ai) => {
                let Some(sp) = cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si)) else { return vec![] };
                let Some(a) = sp.anchors.get(ai) else { return vec![] };
                self.press = Some(press(Target::Point { id, si, ai, p: a.p, n: sp.anchors.len(), closed: sp.closed }));
                if ev.mods.alt {
                    // Alt: a click removes it, a drag pulls out a handle (decided on the way).
                    return vec![];
                }
                let mode = if ev.mods.shift { "toggle" } else { "set" };
                vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": mode}))]
            }
            Hit::Join(other, osi, first) => {
                let Some((id, si, _)) = self.active(cx) else { return vec![] };
                self.stop();
                let end = if first { "first" } else { "last" };
                vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", end], "subpaths": [si, osi]}))]
            }
            Hit::Segment(id, si, seg, t) => {
                self.press = Some(press(Target::Segment { id, si, seg, t }));
                vec![]
            }
            Hit::Continue(id, si, first) => {
                (self.drawing, self.path) = (true, Some((id, si)));
                continue_from(cx, id, si, first)
            }
            Hit::Nothing => {
                let path = self.active(cx).map(|(id, si, _)| (id, si));
                if path.is_none() {
                    // A new path: the engine makes it, and the next event picks it up.
                    (self.drawing, self.path) = (true, None);
                }
                let pl = Place { at: p, h_in: p, h_out: p, path, seg: self.curved.then_some(true), tabbed: false };
                self.press = Some(Press { begun: true, ..press(Target::Place(pl)) });
                vec![Action::Begin("Smart Pen".into()), self.place_preview(&pl, false)]
            }
        }
    }

    fn press_drag(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let Some(mut pr) = self.press.take() else { return vec![] };
        if !pr.dragged && ev.pos.distance(pr.at) < cx.tol(DRAG_THRESHOLD) {
            self.last = ev.pos;
            self.press = Some(pr);
            return vec![];
        }
        let first = !pr.dragged;
        pr.dragged = true;
        let (shift, alt) = (ev.mods.shift, ev.mods.alt);
        let (label, act) = match pr.what {
            Target::Place(mut pl) => {
                if ev.mods.space {
                    let d = ev.pos - self.last;
                    pl.at += d;
                    pl.h_in += d;
                }
                pl.h_out = Self::handle_to(cx, pl.at, ev.pos, pl.at, shift, false);
                // Alt keeps the incoming handle where it is: only the outgoing one follows.
                if !alt {
                    pl.h_in = pl.at - (pl.h_out - pl.at);
                }
                pr.what = Target::Place(pl);
                ("", self.place_preview(&pl, true))
            }
            Target::Close { id, si, first } => {
                let out = Self::handle_to(cx, first, ev.pos, first, shift, false);
                let h_in = first - (out - first);
                ("", Action::Preview("path.close".into(), json!({"id": id.0, "subpath": si, "in": xy(h_in), "independent": alt})))
            }
            Target::Point { id, si, ai, p, .. } if pr.mods.alt => {
                let h = Self::handle_to(cx, p, ev.pos, p, shift, false);
                let params = json!({"id": id.0, "subpath": si, "anchor": ai, "which": "out", "x": h.x, "y": h.y});
                ("Move Direction Handle", Action::Preview("path.setHandle".into(), params))
            }
            Target::Point { id, si, ai, p, .. } => {
                let to = if shift { p + constrain_angle_from(ev.pos - pr.at, 45.0, cx.constrain_angle) } else { p + (ev.pos - pr.at) };
                (
                    "Move Anchor Point",
                    Action::Preview("path.moveAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai, "x": to.x, "y": to.y})),
                )
            }
            Target::Handle { id, si, ai, out, anchor, h } => {
                let to = Self::handle_to(cx, anchor, h + (ev.pos - pr.at), h, shift, alt);
                let which = if out { "out" } else { "in" };
                let params = json!({"id": id.0, "subpath": si, "anchor": ai, "which": which, "x": to.x, "y": to.y});
                ("Move Direction Handle", Action::Preview("path.setHandle".into(), params))
            }
            Target::Segment { id, si, seg, t } => {
                let d = ev.pos - pr.at;
                let mode = if shift {
                    "symmetric"
                } else if alt {
                    "keepAngles"
                } else {
                    "free"
                };
                let params = json!({"id": id.0, "subpath": si, "segment": seg, "t": t, "dx": d.x, "dy": d.y, "mode": mode});
                ("Reshape Segment", Action::Preview("addon.pen.reshape".into(), params))
            }
        };
        self.last = ev.pos;
        let mut out = vec![];
        if first && !pr.begun {
            pr.begun = true;
            out.push(Action::Begin(label.into()));
        }
        out.push(act);
        self.press = Some(pr);
        out
    }

    fn press_up(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.snap.clear();
        let Some(pr) = self.press.take() else { return vec![] };
        if pr.begun {
            if matches!(pr.what, Target::Close { .. }) {
                self.stop();
            }
            return vec![Action::Commit];
        }
        match pr.what {
            Target::Point { id, si, ai, .. } if pr.mods.alt => {
                vec![Action::Exec("addon.pen.removePoint".into(), json!({"id": id.0, "subpath": si, "anchor": ai, "plain": pr.mods.shift}))]
            }
            // A click on an end of an open path draws on from it; on any other point it ends the
            // path being drawn.
            Target::Point { id, si, ai, n, closed, .. } => {
                if closed || (ai != 0 && ai + 1 != n) || pr.mods.shift {
                    self.stop();
                    return vec![];
                }
                (self.drawing, self.path) = (true, Some((id, si)));
                if ai + 1 == n {
                    return vec![];
                }
                let reverse = Action::Exec("path.reverse".into(), json!({"ids": [id.0], "subpath": si}));
                vec![reverse, Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, n - 1]], "mode": "set"}))]
            }
            Target::Segment { id, si, seg, t } if pr.mods.alt => {
                vec![Action::Exec("path.insertAnchor".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))]
            }
            Target::Segment { id, .. } if !cx.selection.contains(id) && self.active(cx).is_none() => {
                let top = cx.doc.parent_of(id).filter(|p| cx.doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))).unwrap_or(id);
                vec![Action::Exec("select.set".into(), json!({"ids": [top.0]}))]
            }
            _ => vec![],
        }
    }

    /// Tab: the segment into the anchor being placed, or else the last one drawn, switches
    /// between straight and curved.
    fn tab(&mut self, cx: &ToolContext) -> Vec<Action> {
        if let Some(pr) = &mut self.press
            && let Target::Place(pl) = &mut pr.what
        {
            pl.seg = Some(pl.seg == Some(false));
            pl.tabbed = true;
            let (pl, dragged) = (*pl, pr.dragged);
            return vec![self.place_preview(&pl, dragged)];
        }
        let Some((id, si, sp)) = self.active(cx) else { return vec![] };
        let Some(seg) = sp.segment_count().checked_sub(1) else { return vec![] };
        vec![Action::Exec("addon.pen.segment".into(), json!({"id": id.0, "subpath": si, "segment": seg, "curved": sp.segment_is_line(seg)}))]
    }

    /// The next segment as a click at `h` would add it.
    fn rubber_band(&self, sp: &SubPath, h: Point) -> Option<BezPath> {
        let mut sp = sp.clone();
        sp.anchors.push(Anchor::corner(h));
        let prev = sp.anchors.len().checked_sub(2)?;
        if self.curved {
            leave_handle(&mut sp, prev, true, SMOOTH_RATIO);
        } else {
            straighten_segment(&mut sp, prev);
        }
        let tail = sp.anchors.get(prev..)?;
        let mut bp = BezPath::new();
        SubPath::new(tail.to_vec(), false).to_bezpath_into(&mut bp);
        Some(bp)
    }
}

impl Tool for SmartPenTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.press.is_some() || self.marquee.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.sync(cx);
        let active = self.active(cx);
        if self.drawing && active.is_none() && ev.kind == PointerKind::Down {
            self.stop();
        }
        // Anchors placed snap to Smart Guides; Shift keeps them to 45° steps from the last one.
        let drawn = active.map(|a| a.0);
        let from = active.and_then(|(.., sp)| sp.anchors.last()).map(|a| {
            let leave = Leave::segment(cx, a.p, ev.mods.shift);
            if a.has_out() { leave.without_contact() } else { leave }
        });
        if let Some(acts) = self.marquee_event(cx, ev) {
            return acts;
        }
        match ev.kind {
            PointerKind::Move => {
                self.hover = Some(self.snap.hover(cx, ev.pos, drawn.as_slice(), from.as_ref()));
                vec![]
            }
            PointerKind::Down => {
                self.last = ev.pos;
                let p = self.snap.press(cx, ev.pos, drawn.as_slice(), from.as_ref());
                self.press_down(cx, ev, p)
            }
            PointerKind::Drag => self.press_drag(cx, ev),
            PointerKind::Up => self.press_up(cx),
            PointerKind::DoubleClick => match self.hit(cx, ev.pos) {
                Hit::Handle(id, si, ai, _) | Hit::Anchor(id, si, ai) => {
                    vec![Action::Exec("addon.pen.togglePoint".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))]
                }
                _ => vec![],
            },
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        self.sync(cx);
        match key {
            ToolKey::Tab => self.tab(cx),
            ToolKey::Enter | ToolKey::Escape => self.deactivate(cx),
            _ => vec![],
        }
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Tab && (self.press.is_some() || self.active(cx).is_some())
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.stop();
        self.marquee = None;
        self.snap.clear();
        match self.press.take() {
            Some(pr) if pr.begun => vec![Action::Commit],
            _ => vec![],
        }
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        if cx.pen_rubber_band
            && self.press.is_none()
            && let (Some((id, _, sp)), Some(h)) = (self.active(cx), self.hover)
            && let Some(path) = self.rubber_band(sp, h)
        {
            o.push(Overlay::Path { path, color: cx.doc.layer_color(id), width: 1.0, dashed: false });
            if let Some(last) = sp.anchors.last().map(|a| a.p)
                && cx.measurement_labels
                && last.distance(h) > 1e-9
            {
                o.push(Overlay::Measure { p: h + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.segment_measure_label(last, h) });
            }
        }
        o.extend_from_slice(self.snap.guides());
        if let Some((a, b, _)) = self.marquee {
            o.push(Overlay::Marquee(Rect::from_points(a, b)));
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        match self.hit(cx, p) {
            Hit::Close(..) => Cursor::PenClose,
            Hit::Handle(..) => Cursor::AnchorPoint,
            Hit::Anchor(..) if m.alt => Cursor::PenDelete,
            Hit::Anchor(..) => Cursor::DirectAnchorSelected,
            Hit::Join(..) => Cursor::PenJoin,
            Hit::Segment(..) if m.alt => Cursor::PenAdd,
            Hit::Segment(..) => Cursor::ReshapeSegment,
            Hit::Continue(..) => Cursor::PenContinue,
            Hit::Nothing if self.active(cx).is_some() => Cursor::PenDrawing,
            Hit::Nothing => Cursor::SmartPen,
        }
    }

    fn options(&self) -> Value {
        json!({ "next": if self.curved { "curved" } else { "straight" } })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        if key == "next" {
            match v.as_str() {
                Some("curved") => self.curved = true,
                Some("straight") => self.curved = false,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
