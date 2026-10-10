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
//! Each anchor placed snaps to Smart Guides ([`DrawSnap`]): onto the anchors, centres and paths of
//! the other art and of the path being drawn, into line with them, onto the construction guides
//! through the last anchor; with Shift held it slides along its 45° step into line. Hovering shows
//! where the next one would go.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{BezPath, Point};

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
    /// The last anchor of the path being drawn, while a click retracts its outgoing handle or a
    /// drag pulls a new one out: (path, subpath, anchor, its position).
    handle: Option<(NodeId, usize, usize, Point)>,
    /// The Anchor Point tool, while an Alt gesture on a selected path's handle or anchor lasts.
    convert: Option<AnchorTool>,
    /// Smart Guides for the anchors placed (and the pointer hovering).
    snap: DrawSnap,
}

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
            || crate::draw2::anchor_in(cx, editable_paths(cx), p, point).is_some()
            || crate::draw2::segment_in(cx, editable_paths(cx), p, cx.pick_tol()).is_some())
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
        let from = active.map(|(_, _, last, _)| Leave::segment(cx, last, ev.mods.shift));
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
                if let Some((id, si, ai, a)) = self.handle {
                    let mut h = ev.pos;
                    if ev.mods.shift {
                        h = a + vectorcraft_geom::constrain_angle(ev.pos - a, 45.0);
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
                    out_h = a + vectorcraft_geom::constrain_angle(ev.pos - a, 45.0);
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
        }
        o.extend_from_slice(self.snap.guides());
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        let tol = cx.tol(5.0).max(cx.point_tol());
        if let Some((id, first, last, _)) = self.active(cx) {
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
            return Cursor::PenConvert;
        }
        if !self.drawing {
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
        Cursor::Pen
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
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn snapped_last_endpoint_converts_active_path_after_raw_miss() {
        let first = Point::new(100.0, 300.0);
        let last = Point::new(200.0, 400.0);
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
        d.grid.spacing = 100.0;
        d.grid.subdivisions = 1;
        let paint = paint();
        let cx = ToolContext {
            zoom: 1.0,
            selection_tolerance: 8.0,
            smart_guides: false,
            snap_to_point: false,
            snap_to_pixel: false,
            snap_to_grid: true,
            ..cx(&d, &s, &paint)
        };
        let sp = &d.node(id).unwrap().path_data().unwrap().subpaths[0];
        assert!(!sp.closed);
        assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), vec![first, Point::new(200.0, 300.0), last]);
        let mut pen = PenTool::default();
        assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
        assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
        assert!(!pen.busy());

        let raw = Point::new(237.0, 403.0);
        let tol = cx.tol(5.0).max(cx.point_tol());
        assert!(raw.distance(first) > tol && raw.distance(last) > tol);
        let from = Leave::segment(&cx, last, false);
        assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), last);
        assert!(open_end_at(&cx, raw, tol, Some(id)).is_none());
        assert!(open_end_at(&cx, last, tol, Some(id)).is_none());

        assert_eq!(
            pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 2, last.x, last.y)]
        );
        assert_eq!(pen.handle, Some((id, 0, 2, last)));
        assert!(pen.drag.is_none() && pen.busy());
        assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)), vec![Action::Commit]);
        assert!(pen.handle.is_none() && !pen.busy());
        assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
    }

    #[test]
    fn raw_other_endpoint_wins_before_snapped_active_last_endpoint() {
        let first = Point::new(100.0, 300.0);
        let last = Point::new(200.0, 400.0);
        let raw = Point::new(237.0, 403.0);
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
        let other = d.alloc_id();
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[raw, Point::new(337.0, 403.0)], false));
        d.insert(Some(d.layers[0].id), 2, vectorcraft_doc::Node::path(other, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        d.grid.spacing = 100.0;
        d.grid.subdivisions = 1;
        let paint = paint();
        let cx = ToolContext {
            zoom: 1.0,
            selection_tolerance: 8.0,
            smart_guides: false,
            snap_to_point: false,
            snap_to_pixel: false,
            snap_to_grid: true,
            ..cx(&d, &s, &paint)
        };
        let mut pen = PenTool::default();
        assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
        assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
        assert!(!pen.busy());
        let tol = cx.tol(5.0).max(cx.point_tol());
        assert!(raw.distance(first) > tol && raw.distance(last) > tol);
        assert_eq!(open_end_at(&cx, raw, tol, Some(id)), Some((other, true)));
        let from = Leave::segment(&cx, last, false);
        assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), last);

        assert_eq!(
            pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
            vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", "first"]}))]
        );
        assert!(!pen.busy() && pen.active(&cx).is_none());
        assert!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)).is_empty());
    }

    #[test]
    fn snapped_first_endpoint_closes_active_path_after_raw_miss() {
        let first = Point::new(100.0, 300.0);
        let last = Point::new(200.0, 400.0);
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
        d.grid.spacing = 100.0;
        d.grid.subdivisions = 1;
        let paint = paint();
        let cx = ToolContext {
            zoom: 1.0,
            selection_tolerance: 8.0,
            smart_guides: false,
            snap_to_point: false,
            snap_to_pixel: false,
            snap_to_grid: true,
            ..cx(&d, &s, &paint)
        };
        let sp = &d.node(id).unwrap().path_data().unwrap().subpaths[0];
        assert!(!sp.closed);
        assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), vec![first, Point::new(200.0, 300.0), last]);
        let mut pen = PenTool::default();
        assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
        assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
        assert!(!pen.busy());

        let raw = Point::new(63.0, 303.0);
        let tol = cx.tol(5.0).max(cx.point_tol());
        assert!(raw.distance(first) > tol && raw.distance(last) > tol);
        let from = Leave::segment(&cx, last, false);
        assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), first);
        assert!(open_end_at(&cx, raw, tol, Some(id)).is_none());
        assert!(open_end_at(&cx, first, tol, Some(id)).is_none());

        assert_eq!(
            pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
            vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
        );
        let place = pen.drag.unwrap();
        assert!(place.closing);
        assert_eq!((place.path, place.at), (Some(id), first));
        assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)), vec![Action::Commit]);
        assert!(!pen.busy() && pen.active(&cx).is_none());
    }

    #[test]
    fn raw_other_endpoint_wins_before_snapped_active_first_endpoint() {
        let first = Point::new(100.0, 300.0);
        let last = Point::new(200.0, 400.0);
        let raw = Point::new(63.0, 303.0);
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
        let other = d.alloc_id();
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[raw, Point::new(63.0, 403.0)], false));
        d.insert(Some(d.layers[0].id), 2, vectorcraft_doc::Node::path(other, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        d.grid.spacing = 100.0;
        d.grid.subdivisions = 1;
        let paint = paint();
        let cx = ToolContext {
            zoom: 1.0,
            selection_tolerance: 8.0,
            smart_guides: false,
            snap_to_point: false,
            snap_to_pixel: false,
            snap_to_grid: true,
            ..cx(&d, &s, &paint)
        };
        let mut pen = PenTool::default();
        assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
        assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
        assert!(!pen.busy());
        let tol = cx.tol(5.0).max(cx.point_tol());
        assert!(raw.distance(first) > tol && raw.distance(last) > tol);
        assert_eq!(open_end_at(&cx, raw, tol, Some(id)), Some((other, true)));
        let from = Leave::segment(&cx, last, false);
        assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), first);

        assert_eq!(
            pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
            vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", "first"]}))]
        );
        assert!(!pen.busy() && pen.active(&cx).is_none());
        assert!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)).is_empty());
    }

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
    fn alt_pressed_mid_drag_keeps_the_incoming_handle() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::shapes::line(Point::new(10.0, 300.0), Point::new(60.0, 300.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        let alt = Mods { alt: true, ..Mods::default() };
        let handles = |acts: Vec<Action>| match acts.as_slice() {
            [Action::Preview(c, v)] if c == "path.appendAnchor" => (v["in"].clone(), v["out"].clone()),
            other => panic!("not an anchor preview: {other:?}"),
        };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 420.0));
        let drag = PointerEvent::new(PointerKind::Drag, 450.0, 420.0);
        assert_eq!(handles(t.pointer(&cx, &drag)), (json!([350.0, 420.0]), json!([450.0, 420.0])));
        // Alt goes down: the curve shaped so far stays, and only the outgoing handle moves on.
        let drag = PointerEvent::new(PointerKind::Drag, 450.0, 470.0).with_mods(alt);
        assert_eq!(handles(t.pointer(&cx, &drag)), (json!([350.0, 420.0]), json!([450.0, 470.0])));
        let drag = PointerEvent::new(PointerKind::Drag, 400.0, 480.0).with_mods(alt);
        assert_eq!(handles(t.pointer(&cx, &drag)), (json!([350.0, 420.0]), json!([400.0, 480.0])));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 400.0, 480.0).with_mods(alt)), vec![Action::Commit]);
        // Alt held from the start: the new anchor gets an outgoing handle only.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 440.0).with_mods(alt));
        let drag = PointerEvent::new(PointerKind::Drag, 340.0, 440.0).with_mods(alt);
        assert_eq!(handles(t.pointer(&cx, &drag)), (json!([300.0, 440.0]), json!([340.0, 440.0])));
    }

    #[test]
    fn space_moves_the_anchor_being_placed() {
        let (mut d, _) = doc_with_rect();
        let space = Mods { space: true, ..Mods::default() };
        // The first anchor of a new path.
        let (s, p) = (Selection::default(), paint());
        let cx0 = cx(&d, &s, &p);
        let mut t = PenTool::default();
        t.pointer(&cx0, &PointerEvent::new(PointerKind::Down, 400.0, 420.0));
        t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 450.0, 420.0));
        let created = |acts: Vec<Action>| match acts.as_slice() {
            [Action::Preview(c, v)] if c == "path.create" => v["anchors"][0].clone(),
            other => panic!("not a create preview: {other:?}"),
        };
        let a = created(t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 450.0, 380.0).with_mods(space)));
        assert_eq!(a, json!({"x": 400.0, "y": 380.0, "out": [450.0, 380.0], "in": [350.0, 380.0]}), "moved, handles and all");
        // Space released: the drag shapes the handles round the anchor's new place.
        let a = created(t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 460.0, 380.0)));
        assert_eq!(a, json!({"x": 400.0, "y": 380.0, "out": [460.0, 380.0], "in": [340.0, 380.0]}));
        // An anchor added to an open path, its handles broken by Alt: the kept incoming handle
        // moves with it.
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let line = vectorcraft_geom::shapes::line(Point::new(10.0, 300.0), Point::new(60.0, 300.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let cx = cx(&d, &s, &p);
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        let alt = Mods { alt: true, ..Mods::default() };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 420.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 420.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 470.0).with_mods(alt));
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 450.0).with_mods(Mods { space: true, ..alt }));
        assert_eq!(
            acts,
            vec![Action::Preview(
                "path.appendAnchor".into(),
                json!({"id": id.0, "x": 400.0, "y": 400.0, "in": [350.0, 400.0], "out": [450.0, 450.0]})
            )]
        );
    }

    /// A document with an open path of `anchors`, selected, being drawn by a Pen.
    fn drawing(anchors: vectorcraft_geom::SubPath) -> (vectorcraft_doc::Document, NodeId, Selection) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let path = vectorcraft_geom::PathData::single(anchors);
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        (d, id, s)
    }

    fn set_out(id: NodeId, ai: usize, x: f64, y: f64) -> Action {
        Action::Preview("path.setHandle".into(), json!({"id": id.0, "subpath": 0, "anchor": ai, "which": "out", "x": x, "y": y, "independent": true}))
    }

    #[test]
    fn clicking_the_last_anchor_retracts_its_handle_and_dragging_pulls_a_new_one() {
        // The last anchor is smooth: handles at 40 and 80 either side of (60, 300).
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false);
        sp.anchors[1].h_in = Point::new(40.0, 300.0);
        sp.anchors[1].h_out = Point::new(80.0, 300.0);
        let (d, id, s) = drawing(sp);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(t.cursor(&cx, Point::new(61.0, 301.0), Mods::default()), Cursor::PenConvert);
        // A click retracts the outgoing handle and adds no anchor.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 61.0, 301.0));
        assert_eq!(a, vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 61.0, 301.0)), vec![Action::Commit]);
        // A drag pulls a new outgoing handle out.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 60.0, 300.0));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 70.0, 280.0)), vec![set_out(id, 1, 70.0, 280.0)]);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 70.0, 280.0)), vec![Action::Commit]);
        // Elsewhere a click still adds an anchor, and the first anchor still closes the path.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 400.0));
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 200.0, 400.0));
        assert_eq!(t.cursor(&cx, Point::new(10.0, 300.0), Mods::default()), Cursor::PenClose);
    }

    /// #544: picking the terminal anchor uses the pointer, even when new points snap elsewhere.
    #[test]
    fn terminal_anchor_conversion_precedes_grid_snapping() {
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(13.0, 303.0), Point::new(63.0, 303.0)], false));
        d.grid.spacing = 100.0;
        d.grid.subdivisions = 1;
        let p = paint();
        let cx = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(t.cursor(&cx, Point::new(63.0, 303.0), Mods::default()), Cursor::PenConvert);
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 63.0, 303.0)),
            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 63.0, 303.0)]
        );
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 63.0, 303.0)), vec![Action::Commit]);
        // The same raw picking applies when resuming the path and when closing it.
        let mut t = PenTool::default();
        assert_eq!(t.cursor(&cx, Point::new(63.0, 303.0), Mods::default()), Cursor::PenContinue);
        assert!(click(&mut t, &cx, 63.0, 303.0, Mods::default()).is_empty());
        assert_eq!(t.active(&cx).map(|a| a.0), Some(id));
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 13.0, 303.0)),
            vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
        );
    }

    #[test]
    fn smart_guides_cannot_steal_a_terminal_anchor_press() {
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
        let other = d.alloc_id();
        let rect = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(66.0, 300.0, 76.0, 310.0));
        d.insert(Some(d.layers[0].id), 2, vectorcraft_doc::Node::path(other, rect, vectorcraft_doc::Appearance::default_art())).unwrap();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let pos = Point::new(63.0, 300.0);
        assert_eq!(crate::guides::snap_draw(&cx, pos, &[id]).0, Point::new(66.0, 300.0));
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(t.cursor(&cx, pos, Mods::default()), Cursor::PenConvert);
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pos.x, pos.y)),
            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
        );
    }

    /// Nearby endpoints share a pick radius: use the closest one for cursor and press.
    #[test]
    fn overlapping_endpoint_tolerances_pick_the_nearest_end() {
        let paint = paint();
        let mut failures = vec![];
        for zoom in [0.5, 1.0, 2.0] {
            let first = Point::new(100.0, 300.0);
            let last = Point::new(100.0 + 6.0 / zoom, 300.0);
            let (d, id, selection) = drawing(vectorcraft_geom::SubPath::polyline(&[first, last], false));
            for grid in [false, true] {
                let cx = ToolContext {
                    zoom,
                    selection_tolerance: 8.0,
                    smart_guides: false,
                    snap_to_point: false,
                    snap_to_grid: grid,
                    ..cx(&d, &selection, &paint)
                };
                for shift in [false, true] {
                    let mods = Mods { shift, ..Mods::default() };
                    // Include off-anchor clicks in both halves of the overlap and the midpoint.
                    for offset in [6.0, 0.0, 1.0, 3.0, 5.0] {
                        let at = Point::new(first.x + offset / zoom, first.y);
                        let first_end = offset <= 3.0;
                        let mut pen = PenTool { drawing: true, ..PenTool::default() };
                        let cursor = if first_end { Cursor::PenClose } else { Cursor::PenConvert };
                        let actual_cursor = pen.cursor(&cx, at, mods);
                        let expected = if first_end {
                            vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
                        } else {
                            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, last.x, last.y)]
                        };
                        let actual = pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y).with_mods(mods));
                        if actual_cursor != cursor || actual != expected {
                            failures.push((zoom, grid, shift, offset, actual_cursor, actual));
                        }
                        assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, at.x, at.y)), vec![Action::Commit]);
                    }
                }
            }
        }
        assert!(failures.is_empty(), "cursor and press must pick the closest eligible endpoint: {failures:?}");
    }

    #[test]
    fn overlapping_endpoint_tolerances_continue_from_the_nearest_end() {
        let paint = paint();
        let mut failures = vec![];
        for zoom in [0.5, 1.0, 2.0] {
            let first = Point::new(100.0, 300.0);
            let last = Point::new(100.0 + 6.0 / zoom, 300.0);
            let (d, id, selection) = drawing(vectorcraft_geom::SubPath::polyline(&[first, last], false));
            for grid in [false, true] {
                let cx = ToolContext {
                    zoom,
                    selection_tolerance: 8.0,
                    smart_guides: false,
                    snap_to_point: false,
                    snap_to_grid: grid,
                    ..cx(&d, &selection, &paint)
                };
                for offset in [6.0, 0.0, 1.0, 3.0, 5.0] {
                    let at = Point::new(first.x + offset / zoom, first.y);
                    let mut pen = PenTool::default();
                    assert_eq!(pen.cursor(&cx, at, Mods::default()), Cursor::PenContinue);
                    let expected = if offset < 3.0 { vec![Action::Exec("path.reverse".into(), json!({}))] } else { vec![] };
                    let actual = click(&mut pen, &cx, at.x, at.y, Mods::default());
                    if actual != expected {
                        failures.push((zoom, grid, offset, actual));
                    }
                    assert_eq!(pen.active(&cx).map(|a| a.0), Some(id));
                }
            }
        }
        assert!(failures.is_empty(), "resuming must reverse only for the closest first endpoint: {failures:?}");
    }

    #[test]
    fn coincident_endpoints_keep_the_prior_tie_choice() {
        let at = Point::new(100.0, 300.0);
        let (d, id, selection) = drawing(vectorcraft_geom::SubPath::polyline(&[at, at], false));
        let paint = paint();
        for zoom in [0.5, 1.0, 2.0] {
            let cx = ToolContext { zoom, smart_guides: false, snap_to_point: false, ..cx(&d, &selection, &paint) };
            let mut drawing = PenTool { drawing: true, ..PenTool::default() };
            assert_eq!(drawing.cursor(&cx, at, Mods::default()), Cursor::PenClose);
            assert_eq!(
                click(&mut drawing, &cx, at.x, at.y, Mods::default()),
                vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
            );
            let mut resuming = PenTool::default();
            assert_eq!(resuming.cursor(&cx, at, Mods::default()), Cursor::PenContinue);
            assert!(click(&mut resuming, &cx, at.x, at.y, Mods::default()).is_empty());
        }
    }

    #[test]
    fn terminal_anchor_uses_the_selection_tolerance_at_each_zoom() {
        let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
        let p = paint();
        for zoom in [0.5, 2.0] {
            let cx = ToolContext { zoom, selection_tolerance: 8.0, smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
            let pos = Point::new(60.0, 300.0 + 6.0 / zoom);
            let mut t = PenTool { drawing: true, ..PenTool::default() };
            assert_eq!(t.cursor(&cx, pos, Mods::default()), Cursor::PenConvert);
            assert_eq!(
                t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pos.x, pos.y)),
                vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
            );
            let narrow = ToolContext { selection_tolerance: 1.0, ..cx };
            // The upstream point picker retains a minimum screen radius at low tolerance.
            let near = Point::new(60.0, 300.0 + 4.0 / zoom);
            let mut narrow_pen = PenTool { drawing: true, ..PenTool::default() };
            assert_eq!(narrow_pen.cursor(&narrow, near, Mods::default()), Cursor::PenConvert);
            assert_eq!(
                click(&mut narrow_pen, &narrow, near.x, near.y, Mods::default()),
                vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
            );
            assert_eq!(narrow_pen.cursor(&narrow, Point::new(60.0, 300.0 + 6.0 / zoom), Mods::default()), Cursor::Pen);
        }
    }

    /// Upstream's point radius grows with the drawn anchor; cursor and press keep agreeing.
    #[test]
    fn enlarged_anchor_radius_is_used_for_endpoints_and_alt_conversion() {
        let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
        let p = paint();
        for zoom in [0.5, 1.0, 2.0] {
            let cx = ToolContext { zoom, selection_tolerance: 1.0, anchor_size: 7, smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
            let at = Point::new(60.0 + 6.0 / zoom, 300.0);
            let mut terminal = PenTool { drawing: true, ..PenTool::default() };
            assert_eq!(terminal.cursor(&cx, at, Mods::default()), Cursor::PenConvert);
            assert_eq!(
                click(&mut terminal, &cx, at.x, at.y, Mods::default()),
                vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
            );
            let mut continuing = PenTool::default();
            assert_eq!(continuing.cursor(&cx, at, Mods::default()), Cursor::PenContinue);
            assert!(click(&mut continuing, &cx, at.x, at.y, Mods::default()).is_empty());
            assert_eq!(continuing.active(&cx).map(|a| a.0), Some(id));
            let mut closing = PenTool { drawing: true, ..PenTool::default() };
            let first = Point::new(10.0 - 6.0 / zoom, 300.0);
            assert_eq!(closing.cursor(&cx, first, Mods::default()), Cursor::PenClose);
            assert_eq!(
                click(&mut closing, &cx, first.x, first.y, Mods::default()),
                vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
            );
            let mut alt_pen = PenTool { drawing: true, ..PenTool::default() };
            let alt = Mods { alt: true, ..Mods::default() };
            assert!(alt_pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y).with_mods(alt)).is_empty());
            assert!(alt_pen.busy(), "Alt must reach the enlarged terminal-anchor target");
            assert!(alt_pen.convert.is_some());
            let away_from_segment = Point::new(35.0, 300.0 + 4.0 / zoom);
            assert!(!alt_converts(&cx, away_from_segment, alt), "segment picking retains the smaller selection radius");
        }
    }

    /// #516: Alt on the terminal anchor borrows Anchor Point, just like the other anchors.
    #[test]
    fn alt_on_the_terminal_anchor_pulls_symmetric_handles() {
        let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0).with_mods(alt)).is_empty());
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 230.0, 330.0).with_mods(alt)),
            vec![
                Action::Begin("Convert Anchor Point".into()),
                Action::Preview("path.convertAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "to": "smooth", "x": 230.0, "y": 330.0}))
            ]
        );
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 230.0, 330.0)), vec![Action::Commit]);
        assert!(
            matches!(click(&mut t, &cx, 300.0, 400.0, Mods::default()).as_slice(), [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor")
        );
    }

    #[test]
    fn ending_during_a_pen_gesture_commits_and_clears_it() {
        let (d, _, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false));
        let p = paint();
        let cx = cx(&d, &s, &p);
        for terminal in [false, true] {
            for key in [Some(ToolKey::Escape), Some(ToolKey::Enter), None] {
                let mut t = PenTool { drawing: true, ..PenTool::default() };
                let x = if terminal { 200.0 } else { 300.0 };
                t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, 300.0));
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x + 20.0, 330.0));
                let acts = match key {
                    Some(k) => t.key(&cx, k, Mods::default()),
                    None => t.deactivate(&cx),
                };
                assert_eq!(acts, vec![Action::Commit]);
                assert!(!t.busy());
                assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x + 20.0, 330.0)).is_empty());
                assert_eq!(t.cursor(&cx, Point::new(400.0, 400.0), Mods::default()), Cursor::Pen);
            }
        }
    }

    /// #501: a press on the first anchor closes the path, however far the drag then goes: the
    /// closing preview leaves the path closed (so not one the pen extends), and each move still
    /// previews the close with symmetric handles (Alt breaks them), released as a close.
    #[test]
    fn a_drag_from_the_first_anchor_always_closes() {
        let pts = [Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(200.0, 400.0)];
        let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&pts, false));
        let p = paint();
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        let a = t.pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 101.0, 301.0));
        assert_eq!(a, vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]);
        // What the tool sees once the preview applied: the path closed.
        d.node_mut(id).unwrap().path_data_mut().unwrap().subpaths[0].closed = true;
        let c = cx(&d, &s, &p);
        let close = |x: f64, y: f64, alt: bool| {
            vec![Action::Preview("path.close".into(), json!({"id": id.0, "in": [200.0 - x, 600.0 - y], "independent": alt}))]
        };
        for (x, y) in [(105.0, 290.0), (130.0, 260.0), (160.0, 240.0)] {
            assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, x, y)), close(x, y, false), "at ({x}, {y})");
        }
        let alt = Mods { alt: true, ..Mods::default() };
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 170.0, 250.0).with_mods(alt)), close(170.0, 250.0, true));
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 170.0, 250.0)), vec![Action::Commit]);
        assert!(!t.busy() && t.active(&c).is_none(), "the path is done");
    }

    #[test]
    fn a_one_anchor_path_does_not_close_on_itself() {
        let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0)], false));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(t.cursor(&cx, Point::new(10.0, 300.0), Mods::default()), Cursor::PenConvert);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 300.0));
        assert_eq!(a, vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 0, 10.0, 300.0)]);
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

    fn click(t: &mut PenTool, cx: &ToolContext, x: f64, y: f64, m: Mods) -> Vec<Action> {
        let acts = t.pointer(cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(m));
        t.pointer(cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(m));
        acts
    }

    #[test]
    fn clicking_a_selected_path_adds_or_deletes_an_anchor() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let pts = [Point::new(300.0, 400.0), Point::new(350.0, 300.0), Point::new(400.0, 400.0)];
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let off = ToolContext { auto_add_delete: false, ..cx(&d, &s, &p) };
        let cx = cx(&d, &s, &p);
        let none = Mods::default();
        let mut t = PenTool::default();
        // On a segment: add an anchor there.
        assert_eq!(t.cursor(&cx, Point::new(325.0, 351.0), none), Cursor::PenAdd);
        let a = click(&mut t, &cx, 325.0, 351.0, none);
        assert!(matches!(&a[..], [Action::Exec(c, v)] if c == "path.insertAnchor" && v["id"] == id.0 && v["segment"] == 0), "{a:?}");
        // On an anchor: delete it.
        assert_eq!(t.cursor(&cx, Point::new(350.0, 302.0), none), Cursor::PenDelete);
        let a = click(&mut t, &cx, 350.0, 302.0, none);
        assert_eq!(a, vec![Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1}))]);
        // An end still continues the path.
        assert_eq!(t.cursor(&cx, Point::new(400.0, 400.0), none), Cursor::PenContinue);
        // Shift, or the preference turned off, starts a new path instead.
        let shift = Mods { shift: true, ..Mods::default() };
        assert_eq!(t.cursor(&cx, Point::new(325.0, 351.0), shift), Cursor::Pen);
        assert_eq!(t.cursor(&off, Point::new(350.0, 302.0), none), Cursor::Pen);
        let a = click(&mut t, &off, 350.0, 302.0, none);
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.create"), "{a:?}");
        // While a path is being drawn, a click goes on drawing it.
        let a = click(&mut t, &cx, 325.0, 351.0, none);
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
        let mut t = PenTool::default();
        let a = click(&mut t, &cx, 325.0, 351.0, shift);
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.create"), "{a:?}");
    }

    /// Each anchor snaps to Smart Guides (#506): hovering shows where it goes (the rubber band ends
    /// there), in line with another object's centre; Shift slides it along its 45° step into line.
    #[test]
    fn anchors_snap_to_smart_guides() {
        let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 152.0, 318.0));
        let o = t.overlays(&cx);
        assert!(o.iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "align" && *p == Point::new(150.0, 318.0))), "{o:?}");
        assert!(o.iter().any(
            |o| matches!(o, Overlay::Path { path, .. } if path.elements().last().and_then(|e| e.end_point()) == Some(Point::new(150.0, 318.0)))
        ));
        let a = click(&mut t, &cx, 152.0, 318.0, Mods::default());
        assert_eq!(a[1], Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": 150.0, "y": 318.0})));
        let shift = Mods { shift: true, ..Mods::default() };
        let a = click(&mut t, &cx, 152.0, 310.0, shift);
        assert_eq!(a[1], Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": 150.0, "y": 300.0})));
        // The first anchor of a new path lands on the rect's corner.
        let mut t = PenTool::default();
        let a = click(&mut t, &cx, 102.0, 99.0, Mods::default());
        assert_eq!(a[1], Action::Preview("path.create".into(), json!({"anchors": [{"x": 100.0, "y": 100.0}]})));
    }

    #[test]
    fn alt_over_a_shown_handle_or_an_anchor_converts() {
        // The middle anchor is smooth: handles at (150, 100) and (250, 100) round (200, 100).
        let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 200.0), Point::new(200.0, 100.0), Point::new(300.0, 200.0)], false);
        sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 100.0), Point::new(250.0, 100.0));
        let (d, id, mut s) = drawing(sp);
        let p = paint();
        let alt = Mods { alt: true, ..Mods::default() };
        let t = PenTool::default();
        let cx0 = cx(&d, &s, &p);
        assert_eq!(t.cursor(&cx0, Point::new(250.0, 101.0), alt), Cursor::PenConvert);
        assert_eq!(t.cursor(&cx0, Point::new(200.0, 101.0), alt), Cursor::PenConvert);
        assert_eq!(t.cursor(&cx0, Point::new(250.0, 101.0), Mods::default()), Cursor::Pen);
        assert_eq!(t.cursor(&cx0, Point::new(150.0, 300.0), alt), Cursor::Pen);
        // A drag of the handle moves it alone, as one gesture.
        let mut t = PenTool::default();
        assert_eq!(t.pointer(&cx0, &PointerEvent::new(PointerKind::Down, 250.0, 101.0).with_mods(alt)), vec![]);
        assert!(t.busy());
        let a = t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 260.0, 140.0).with_mods(alt));
        assert_eq!(
            a,
            vec![
                Action::Begin("Reshape".into()),
                Action::Preview(
                    "path.setHandle".into(),
                    json!({"id": id.0, "subpath": 0, "anchor": 1, "which": "out", "x": 260.0, "y": 140.0, "independent": true})
                )
            ]
        );
        // Alt let go mid-drag: the gesture goes on to its end.
        assert_eq!(t.pointer(&cx0, &PointerEvent::new(PointerKind::Up, 260.0, 140.0)), vec![Action::Commit]);
        assert!(!t.busy());
        // With other anchors direct-selected the handles of this one are hidden, and not grabbed.
        s.anchors.insert(id, std::collections::BTreeSet::from([(0, 0)]));
        let cx1 = cx(&d, &s, &p);
        assert_eq!(t.cursor(&cx1, Point::new(250.0, 101.0), alt), Cursor::Pen);
    }

    /// #494: Alt held over a segment of a selected path reshapes it as the Anchor Point tool does,
    /// while drawing too; Alt away from paths still places an anchor.
    #[test]
    fn alt_over_a_segment_reshapes_it() {
        let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false));
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(t.cursor(&cx, Point::new(150.0, 302.0), alt), Cursor::PenConvert);
        assert_eq!(t.cursor(&cx, Point::new(150.0, 302.0), Mods::default()), Cursor::Pen);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 302.0).with_mods(alt)), vec![]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 262.0).with_mods(alt));
        assert_eq!(a[0], Action::Begin("Reshape".into()));
        assert!(
            matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["id"] == id.0 && v["segment"] == 0 && v["dy"] == -40.0),
            "{a:?}"
        );
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 262.0)), vec![Action::Commit]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 250.0, 400.0).with_mods(alt));
        assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
    }

    /// #494: the Pen goes on drawing the path it drew while that path stays selected (after a Cmd
    /// drag with Direct Selection), but never another open path selected meanwhile.
    #[test]
    fn the_pen_draws_on_the_path_it_began_only() {
        let (mut d, a, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
        let l = d.layers[0].id;
        let b = d.alloc_id();
        let line = vectorcraft_geom::shapes::line(Point::new(10.0, 400.0), Point::new(60.0, 400.0));
        d.insert(Some(l), 2, vectorcraft_doc::Node::path(b, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        let p = paint();
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        let cx_a = cx(&d, &s, &p);
        let appended = |acts: &[Action]| match acts {
            [Action::Begin(_), Action::Preview(c, v)] if c == "path.appendAnchor" => v["id"].as_u64(),
            _ => None,
        };
        assert_eq!(appended(&click(&mut t, &cx_a, 100.0, 320.0, Mods::default())), Some(a.0));
        // Path B selected instead: the next click starts a new path.
        let mut sb = Selection::default();
        sb.set([b]);
        let cx_b = cx(&d, &sb, &p);
        assert_eq!(t.cursor(&cx_b, Point::new(150.0, 350.0), Mods::default()), Cursor::Pen);
        let acts = click(&mut t, &cx_b, 150.0, 350.0, Mods::default());
        assert!(matches!(&acts[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.create"), "{acts:?}");
    }
}
