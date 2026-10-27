//! The Pen tool (P).
//!
//! Click adds a corner anchor; click-drag adds a smooth anchor with symmetric handles (Alt breaks
//! them: the incoming handle stays where it was when Alt went down and only the outgoing one
//! follows the pointer; Space held moves the anchor, handles and all); Shift constrains to 45°.
//! A press on the first anchor closes the path, the drag shaping the closing curve. Clicking the
//! last one retracts its outgoing handle, so the next segment leaves it as a corner; dragging from
//! it pulls a new one out on its own.
//! Alt held over a handle end, an anchor or a segment of a selected path (the one being drawn too)
//! works as the Anchor Point tool: dragging a handle moves it alone, clicking a smooth anchor makes
//! it a corner, dragging an anchor pulls out new symmetric handles and dragging a segment reshapes
//! it. Cmd held lends the selection tool used last for a drag (the engine's
//! `Session::pointer`); the path being drawn goes on afterwards while it stays selected.
//! Enter/Esc (or switching tools) ends the path. Clicking an end of an open path continues it
//! (selecting it); while drawing, clicking an end of another open path joins the two into one
//! (#776). The rubber-band preview shows the next segment (Enable Rubber Band for Pen Tool). Auto Add/Delete: between paths, a click on a
//! segment of a selected path adds an anchor there and a click on one of its anchors deletes it
//! (Shift held or General → Disable Auto Add/Delete starts a new path instead). On a selected
//! blend's spine a click adds a point (on a point no key object sits on: deletes it).
//! The pointer shows the new-path asterisk where a click starts a new path. While a path is being
//! drawn it shows no badge, except where a click closes the path, joins another open path or
//! converts an anchor.
//! Each anchor placed snaps to Smart Guides ([`DrawSnap`]): onto the anchors, centres and paths of
//! the other art and of the path being drawn, into line with them, onto the construction guides
//! through the last anchor; with Shift held it slides along its 45° step into line. Hovering shows
//! where the next one would go.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{BezPath, Point, Vec2};

use crate::direct::hit_handle;
use crate::draw2::{AnchorTool, editable_paths};
use crate::guides::{DrawSnap, Leave};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Default)]
pub struct PenTool {
    /// A path is being drawn...
    drawing: bool,
    /// ...this one, once the engine made it: another path selected meanwhile isn't drawn on.
    path: Option<NodeId>,
    /// The anchor a press placed, while the drag shapes its handles.
    drag: Option<Place>,
    hover: Option<Point>,
    /// The incoming handle of the anchor being dragged out, as last previewed.
    in_h: Point,
    /// The pointer at the last button-down or drag: Space held moves the anchor as far as it moves.
    last: Point,
    /// Where the button went down: the drag threshold is measured from it.
    pressed: Point,
    /// The pointer has moved past the drag threshold since the press: a press released inside it
    /// is a click, so the anchor stays a corner without handles.
    dragged: bool,
    /// The last anchor of the path being drawn, while a click retracts its outgoing handle or a
    /// drag pulls a new one out: (path, subpath, anchor, its position).
    handle: Option<(NodeId, usize, usize, Point)>,
    /// The Anchor Point tool, while an Alt gesture on a selected path's handle or anchor lasts.
    convert: Option<AnchorTool>,
    /// Smart Guides for the anchors placed (and the pointer hovering).
    snap: DrawSnap,
}

/// Screen pixels the pointer must move after a press before it shapes handles instead of clicking.
const DRAG_THRESHOLD: f64 = 3.0;

/// An anchor being placed, as the press decided: where it is, the path it goes on (none: the
/// first anchor of a new path) and whether it closes that path (the press was on its first
/// anchor). The drag only shapes its handles, whatever the path looks like meanwhile: a closing
/// preview leaves the path closed, so no longer one the pen draws on.
#[derive(Clone, Copy)]
struct Place {
    at: Point,
    path: Option<NodeId>,
    closing: bool,
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

/// An end of an open path (one subpath, not `except`) under `p`, within `tol`: the path the Pen
/// continues or joins to, and whether it's the path's first anchor. Only paths that can be edited,
/// not guides.
fn open_end_at(cx: &ToolContext, p: Point, tol: f64, except: Option<NodeId>) -> Option<(NodeId, bool)> {
    let id = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options())?.leaf;
    if Some(id) == except || !cx.doc.is_editable(id) {
        return None;
    }
    let NodeKind::Path { path, guide: false, .. } = &cx.doc.node(id)?.kind else { return None };
    let [sp] = path.subpaths.as_slice() else { return None };
    if sp.closed {
        return None;
    }
    let (first, last) = (sp.anchors.first()?.p, sp.anchors.last()?.p);
    let end = pick_endpoint(Some(first), last, p, tol, Endpoint::Last)?;
    Some((id, end == Endpoint::First))
}

/// The last anchor of `id`'s last subpath: (subpath, anchor).
fn last_anchor(cx: &ToolContext, id: NodeId) -> Option<(usize, usize)> {
    let pd = cx.doc.node(id)?.path_data()?;
    let si = pd.subpaths.len().checked_sub(1)?;
    let ai = pd.subpaths.get(si)?.anchors.len().checked_sub(1)?;
    Some((si, ai))
}

/// Alt held over a handle end, an anchor or a segment of a selected path (or of one inside a
/// selected group or compound path, whose anchors show too): the Anchor Point tool's gesture,
/// within its tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Endpoint {
    First,
    Last,
}
/// Pick the closest eligible endpoint at `p`. The caller supplies the historical
/// tie choice: closing prefers the first end, resuming prefers the last. A one-anchor path has
/// no eligible closing end.
fn pick_endpoint(first: Option<Point>, last: Point, p: Point, tol: f64, tie: Endpoint) -> Option<Endpoint> {
    let first_distance = first.map(|a| p.distance(a)).filter(|d| *d <= tol);
    let last_distance = p.distance(last);
    let last_distance = (last_distance <= tol).then_some(last_distance);
    match (first_distance, last_distance) {
        (Some(f), Some(l)) if f < l => Some(Endpoint::First),
        (Some(f), Some(l)) if l < f => Some(Endpoint::Last),
        (Some(_), Some(_)) => Some(tie),
        (Some(_), None) => Some(Endpoint::First),
        (None, Some(_)) => Some(Endpoint::Last),
        (None, None) => None,
    }
}
/// Alt held over a handle end, an anchor or a segment of a selected path: the Anchor Point tool's
/// gesture, within its tolerance.
fn alt_converts(cx: &ToolContext, p: Point, m: Mods) -> bool {
    let point = cx.point_tol();
    m.alt
        && (hit_handle(cx, p, point).is_some()
            || crate::draw2::anchor_in(cx, crate::draw2::shown_paths(cx), p, point).is_some()
            || crate::draw2::segment_in(cx, crate::draw2::shown_paths(cx), p, cx.pick_tol()).is_some())
}

/// The cursor where [`alt_converts`]: Alt lends the Anchor Point tool, so its caret over a handle
/// end or an anchor, the Reshape Segment cursor over a segment.
fn alt_cursor(cx: &ToolContext, p: Point) -> Cursor {
    let point = cx.point_tol();
    if hit_handle(cx, p, point).is_some() || crate::draw2::anchor_in(cx, crate::draw2::shown_paths(cx), p, point).is_some() {
        Cursor::AnchorPoint
    } else {
        Cursor::ReshapeSegment
    }
}

/// Preview the outgoing handle of anchor `ai` of subpath `si` at `h`, the incoming one left alone.
fn set_out_handle(id: NodeId, si: usize, ai: usize, h: Point) -> Action {
    Action::Preview(
        "path.setHandle".into(),
        json!({"id": id.0, "subpath": si, "anchor": ai, "which": "out", "x": h.x, "y": h.y, "independent": true}),
    )
}

impl Tool for PenTool {
    fn id(&self) -> &'static str {
        "pen"
    }
    fn busy(&self) -> bool {
        self.drag.is_some() || self.handle.is_some() || self.convert.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let tol = cx.tol(5.0).max(cx.point_tol());
        // The path a press began is the selected one by the next event.
        if self.drawing && self.path.is_none() {
            self.path = active_path(cx).map(|a| a.0);
        }
        let active = self.active(cx);
        if self.drawing && active.is_none() && ev.kind == PointerKind::Down {
            self.stop();
        }
        // The path being drawn moves its bounds as it grows: only its anchors and segments pull.
        let drawn = active.map(|a| a.0);
        let from = active.map(|(_, _, last, handle)| {
            let leave = Leave::segment(cx, last, ev.mods.shift);
            if handle == last { leave } else { leave.without_contact() }
        });
        let p = match ev.kind {
            PointerKind::Move => self.snap.hover(cx, ev.pos, drawn.as_slice(), from.as_ref()),
            PointerKind::Down => self.snap.press(cx, ev.pos, drawn.as_slice(), from.as_ref()),
            _ => ev.pos,
        };
        match ev.kind {
            PointerKind::Move => {
                self.hover = Some(p);
                vec![]
            }
            PointerKind::Down => {
                self.last = ev.pos;
                self.pressed = ev.pos;
                self.dragged = false;
                if let Some((id, first, last, _)) = active {
                    let end = last_anchor(cx, id);
                    let closing_end = end.filter(|(_, ai)| *ai > 0).map(|_| first);
                    let picked = pick_endpoint(closing_end, last, ev.pos, tol, Endpoint::First);
                    // Pick at the raw pointer before snapping or constraining a new point.
                    // A one-anchor path doesn't close on itself: its anchor is the last one too.
                    if picked == Some(Endpoint::First) {
                        self.drag = Some(Place { at: first, path: Some(id), closing: true });
                        return vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))];
                    }
                    // Alt uses Anchor Point behavior at the terminal anchor too.
                    if let Some(acts) = self.alt_convert(cx, ev) {
                        return acts;
                    }
                    if picked == Some(Endpoint::Last)
                        && let Some((si, ai)) = end
                    {
                        self.handle = Some((id, si, ai, last));
                        return vec![Action::Begin("Convert Anchor Point".into()), set_out_handle(id, si, ai, last)];
                    }
                    let raw_other = open_end_at(cx, ev.pos, tol, Some(id));
                    if raw_other.is_none() && p != ev.pos {
                        let snapped = pick_endpoint(closing_end, last, p, tol, Endpoint::First);
                        if snapped == Some(Endpoint::First) {
                            self.drag = Some(Place { at: first, path: Some(id), closing: true });
                            return vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))];
                        }
                        if snapped == Some(Endpoint::Last)
                            && let Some((si, ai)) = end
                        {
                            self.handle = Some((id, si, ai, last));
                            return vec![Action::Begin("Convert Anchor Point".into()), set_out_handle(id, si, ai, last)];
                        }
                    }
                    // An end of another open path: the two become one, and the path is finished.
                    if let Some((other, at_first)) = raw_other.or_else(|| open_end_at(cx, p, tol, Some(id))) {
                        self.stop();
                        let end = if at_first { "first" } else { "last" };
                        return vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", end]}))];
                    }
                    self.drag = Some(Place { at: p, path: Some(id), closing: false });
                    self.in_h = p;
                    return vec![Action::Begin("Pen".into()), Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": p.x, "y": p.y}))];
                }
                if let Some(acts) = self.alt_convert(cx, ev) {
                    return acts;
                }
                if let Some(acts) = spine_click(cx, p, tol) {
                    return acts;
                }
                // A raw endpoint anywhere wins before either snapped continuation route.
                for at in std::iter::once(ev.pos).chain((p != ev.pos).then_some(p)) {
                    // Continue a selected open path when clicking on one of its ends.
                    if let Some((id, first, last, _)) = active_path(cx)
                        && let Some(picked) = pick_endpoint(Some(first), last, at, tol, Endpoint::Last)
                    {
                        self.drawing = true;
                        self.path = Some(id);
                        if picked == Endpoint::First {
                            return vec![Action::Exec("path.reverse".into(), json!({}))];
                        }
                        return vec![];
                    }
                    // Continue any other open path from the end clicked, selecting it.
                    if let Some((id, at_first)) = open_end_at(cx, at, tol, None) {
                        (self.drawing, self.path) = (true, Some(id));
                        let mut out = vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))];
                        if at_first {
                            out.push(Action::Exec("path.reverse".into(), json!({"ids": [id.0]})));
                        }
                        return out;
                    }
                }
                if let Some(act) = auto_add_delete(cx, ev.pos, ev.mods, tol) {
                    return vec![act];
                }
                (self.drawing, self.path) = (true, None);
                self.drag = Some(Place { at: p, path: None, closing: false });
                vec![Action::Begin("Pen".into()), Action::Preview("path.create".into(), json!({"anchors": [{"x": p.x, "y": p.y}]}))]
            }
            PointerKind::Drag => {
                if let Some(t) = &mut self.convert {
                    return t.pointer(cx, ev);
                }
                // Illustrator treats a press as a drag only past a few screen pixels. `last` follows the
                // pointer meanwhile, so Space pressed before the threshold moves the anchor by the
                // next move only.
                if !self.dragged {
                    if ev.pos.distance(self.pressed) < cx.tol(DRAG_THRESHOLD) {
                        self.last = ev.pos;
                        return vec![];
                    }
                    self.dragged = true;
                }
                if let Some((id, si, ai, a)) = self.handle {
                    let mut h = ev.pos;
                    if ev.mods.shift {
                        h = a + vectorcraft_geom::constrain_angle_from(ev.pos - a, 45.0, cx.constrain_angle);
                    }
                    return vec![set_out_handle(id, si, ai, h)];
                }
                let Some(Place { at: mut a, path, closing }) = self.drag else { return vec![] };
                // Space held moves the anchor being placed, handles and all.
                if ev.mods.space && !closing {
                    let d = ev.pos - self.last;
                    a += d;
                    self.in_h += d;
                    self.drag = Some(Place { at: a, path, closing });
                }
                self.last = ev.pos;
                let mut out_h = ev.pos;
                if ev.mods.shift {
                    out_h = a + vectorcraft_geom::constrain_angle_from(ev.pos - a, 45.0, cx.constrain_angle);
                }
                let alt = ev.mods.alt;
                let Some(id) = path else {
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
                // Alt pressed mid-drag keeps the curve already shaped into the anchor: only the
                // outgoing handle follows. Held from the start, the incoming handle stays retracted.
                let in_h = if alt { self.in_h } else { a - (out_h - a) };
                self.in_h = in_h;
                vec![Action::Preview(
                    "path.appendAnchor".into(),
                    json!({"id": id.0, "x": a.x, "y": a.y, "in": [in_h.x, in_h.y], "out": [out_h.x, out_h.y]}),
                )]
            }
            PointerKind::Up => {
                self.snap.clear();
                if let Some(mut t) = self.convert.take() {
                    return t.pointer(cx, ev);
                }
                if self.handle.take().is_some() {
                    return vec![Action::Commit];
                }
                let Some(place) = self.drag.take() else { return vec![] };
                if place.closing {
                    self.stop();
                }
                vec![Action::Commit]
            }
            PointerKind::DoubleClick => vec![],
        }
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Enter | ToolKey::Escape => self.deactivate(cx),
            _ => vec![],
        }
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.stop();
        self.snap.clear();
        let placing = self.drag.take().is_some();
        let handling = self.handle.take().is_some();
        // Finish the preview as it stands; clearing state alone leaves its transaction open.
        let mut acts = self.convert.take().map(|mut t| t.deactivate(cx)).unwrap_or_default();
        if placing || handling {
            acts.push(Action::Commit);
        }
        acts
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if let Some(t) = &self.convert {
            return t.overlays(cx);
        }
        let mut o = vec![];
        if cx.pen_rubber_band
            && self.drag.is_none()
            && self.handle.is_none()
            && let (Some((id, _, last, out)), Some(h)) = (self.active(cx), self.hover)
        {
            let mut bp = BezPath::new();
            bp.move_to(last);
            if out.distance(last) > 1e-9 {
                bp.quad_to(out, h);
            } else {
                bp.line_to(h);
            }
            o.push(Overlay::Path { path: bp, color: cx.doc.layer_color(id), width: 1.0, dashed: false });
            if cx.measurement_labels && last.distance(h) > 1e-9 {
                o.push(Overlay::Measure { p: h + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.segment_measure_label(last, h) });
            }
        }
        o.extend_from_slice(self.snap.guides());
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        // An Alt gesture under way keeps the Anchor Point tool's cursor.
        if let Some(t) = &self.convert {
            return t.cursor(cx, p, m);
        }
        let tol = cx.tol(5.0).max(cx.point_tol());
        let active = self.active(cx);
        if let Some((id, first, last, _)) = active {
            let closing_end = last_anchor(cx, id).filter(|(_, ai)| *ai > 0).map(|_| first);
            match pick_endpoint(closing_end, last, p, tol, Endpoint::First) {
                Some(Endpoint::First) => return Cursor::PenClose,
                Some(Endpoint::Last) => return Cursor::PenConvert,
                None => {}
            }
            if !alt_converts(cx, p, m) && open_end_at(cx, p, tol, Some(id)).is_some() {
                return Cursor::PenJoin;
            }
        }
        if alt_converts(cx, p, m) {
            return alt_cursor(cx, p);
        }
        // These show whenever no path is being drawn, also after another path was selected while
        // the Pen was drawing.
        if active.is_none() {
            match spine_click(cx, p, tol).as_deref() {
                Some([Action::Exec(c, _)]) if c == "object.blend.spine.removeAnchor" => return Cursor::PenDelete,
                Some([_]) => return Cursor::PenAdd,
                _ => {}
            }
            if active_path(cx).is_some_and(|(_, first, last, _)| pick_endpoint(Some(first), last, p, tol, Endpoint::Last).is_some())
                || open_end_at(cx, p, tol, None).is_some()
            {
                return Cursor::PenContinue;
            }
            match auto_add_delete(cx, p, m, tol) {
                Some(Action::Exec(c, _)) if c == "path.removeAnchor" => return Cursor::PenDelete,
                Some(_) => return Cursor::PenAdd,
                None => {}
            }
        }
        // The new-path asterisk shows only while a click starts a new path.
        if active.is_some() { Cursor::PenDrawing } else { Cursor::Pen }
    }
}

impl PenTool {
    /// The open path being drawn: the one the pen made or went on with, while it is the single
    /// selected open path.
    fn active(&self, cx: &ToolContext) -> Option<(NodeId, Point, Point, Point)> {
        active_path(cx).filter(|(id, ..)| self.drawing && self.path.is_none_or(|p| p == *id))
    }

    /// The path is finished.
    fn stop(&mut self) {
        (self.drawing, self.path) = (false, None);
    }

    /// Hand an Alt press on a selected path's handle, anchor or segment to the Anchor Point tool.
    fn alt_convert(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        if !alt_converts(cx, ev.pos, ev.mods) {
            return None;
        }
        let mut t = AnchorTool::new("anchorPoint");
        let acts = t.pointer(cx, ev);
        if !t.busy() {
            return None;
        }
        self.convert = Some(t);
        Some(acts)
    }
}

/// Auto Add/Delete: what a click at `p` does to a selected path. On one of its anchors it deletes
/// it; else, where one of its segments passes within `tol`, it adds an anchor. None when the
/// preference turns it off, Shift is held, or no selected path is there.
fn auto_add_delete(cx: &ToolContext, p: Point, m: Mods, tol: f64) -> Option<Action> {
    if !cx.auto_add_delete || m.shift {
        return None;
    }
    let anchor = crate::draw2::anchor_in(cx, editable_paths(cx), p, tol).map(crate::draw2::remove_anchor);
    anchor.or_else(|| crate::draw2::segment_in(cx, editable_paths(cx), p, tol).map(crate::draw2::insert_anchor))
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
mod tests;
