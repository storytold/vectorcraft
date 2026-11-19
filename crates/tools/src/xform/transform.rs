//! Rotate (R), Reflect (O), Scale (S) and Shear tools.
//!
//! The reference point defaults to the centre of the selection. A click sets it (shown as a cyan
//! target); a drag transforms about it with a live preview (Shift constrains to 45° / uniform, Alt
//! at release makes a copy: Alt-drag). Alt-click (the release of a click made with Alt) sets the
//! point and opens the tool's dialog; double-click or Return opens the dialog too. A clicked or
//! Alt-clicked point snaps to the nearest anchor or centre (Snap to Point / Smart Guides), labelled
//! while hovering, so transforms pivot exactly on a corner. A drag starts from the point it grabbed
//! snapped the same way, and the pointer snaps as a drawn point does (Smart Guides: the other
//! objects' anchors, centres and edges; or the grid, pixels, Snap to Point), so a corner dragged
//! onto another object's anchor scales exactly onto it. Scale stretches along the bounding box's
//! own width and height, turned with rotated objects until Reset Bounding Box; Reflect's default
//! axis is the box's vertical one, and Shift constrains a drawn axis to 45° steps from the box.
//! With the Reflect tool, a click also sets the first point of an axis, and a second click
//! reflects the selection across the line through both points (Alt on it reflects a copy); a
//! drag from there draws the axis from that first point.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_doc::{AnchorRef, NodeId, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::target_overlays;
use crate::bbox::rotate_for_drag;
use crate::guides::{PointSnap, Targets, snap_pick};
use crate::select::{matrix_json, selection_bounds, selection_box};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Readout, Tool, ToolContext, ToolKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformKind {
    Rotate,
    Reflect,
    Scale,
    Shear,
}

impl TransformKind {
    pub fn id(self) -> &'static str {
        match self {
            TransformKind::Rotate => "rotate",
            TransformKind::Reflect => "reflect",
            TransformKind::Scale => "scale",
            TransformKind::Shear => "shear",
        }
    }
    fn label(self) -> &'static str {
        match self {
            TransformKind::Rotate => "Rotate",
            TransformKind::Reflect => "Reflect",
            TransformKind::Scale => "Scale",
            TransformKind::Shear => "Shear",
        }
    }
    /// Dialog fields (kinds/keys match the UI dialogs).
    fn dialog_fields(self, origin: Point) -> Value {
        let o = json!([origin.x, origin.y]);
        match self {
            TransformKind::Rotate => json!({ "angle": 0, "origin": o }),
            TransformKind::Reflect => json!({ "axis": "vertical", "origin": o }),
            TransformKind::Scale => json!({ "sx": 100, "sy": 100, "origin": o }),
            TransformKind::Shear => json!({ "angle": 0, "axis": "horizontal", "origin": o }),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    start: Point,
    /// The pressed point snapped to an anchor or centre: the reference point on a click, where a
    /// drag starts from.
    pick: Point,
    origin: Point,
    began: bool,
    /// Alt was down at the press: a click opens the dialog, a drag transforms a copy.
    alt: bool,
    bounds: Rect,
    /// The angle of the selection's bounding box: Scale and Reflect act along its axes
    /// ([`TransformTool::matrix_in`]).
    angle: f64,
    /// Anchors are selected with Direct Selection: they transform alone along the page's axes,
    /// and a copy or a pattern-only drag transforms the whole objects along the box
    /// ([`Drag::angle_for`]).
    anchors: bool,
    /// The snapped pointer and Shift at the last drag event, to work out the transform again
    /// when the Alt state at the release changes what it transforms.
    pointer: Point,
    shift: bool,
    last: Affine,
    /// The document revision at the press: a press whose document or selection changes before it
    /// becomes a drag is dropped, so its reference point is never applied to other objects.
    revision: (u64, u64),
}

impl Drag {
    /// The angle Scale and Reflect act along, for a drag that copies (`copy`) or not, and that
    /// transforms only pattern tiles (`patterns_only`, the backquote key) or not.
    fn angle_for(&self, copy: bool, patterns_only: bool) -> f64 {
        if self.anchors && !copy && !patterns_only { 0.0 } else { self.angle }
    }
}

/// The first point of a Reflect axis set by a click, waiting for the second point, with the
/// selection and the document revision it was set for.
#[derive(Clone, Debug)]
struct Axis {
    first: Point,
    objects: Vec<NodeId>,
    anchors: BTreeMap<NodeId, BTreeSet<AnchorRef>>,
    revision: (u64, u64),
}

pub struct TransformTool {
    kind: TransformKind,
    /// Custom reference point and the selection it was set for.
    origin: Option<(Point, Vec<NodeId>)>,
    /// Reflect: the axis whose second point the next click sets.
    axis: Option<Axis>,
    /// The snapped pointer while hovering with an axis waiting, where its dashed line ends.
    axis_to: Option<Point>,
    drag: Option<Drag>,
    measure: Option<(Point, String)>,
    /// What the Info panel shows for the drag in progress.
    readout: Option<Readout>,
    /// Snap feedback while hovering ("anchor" / "center" label), and for the dragged pointer.
    hover: Vec<Overlay>,
    /// What the dragged pointer snaps to, gathered when the drag begins.
    snap: Option<PointSnap>,
}

fn about(o: Point, a: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * a * Affine::translate(-o.to_vec2())
}

/// The transform `m` of the selection (of a copy with `copy`): of the anchors selected with Direct
/// Selection alone, when there are some.
fn preview(m: Affine, copy: bool, backquote: bool) -> Action {
    let mut params = json!({ "matrix": matrix_json(m), "copy": copy, "anchors": true });
    if backquote {
        params["objects"] = json!(false);
        params["patterns"] = json!(true);
    }
    Action::Preview("object.transform".into(), params)
}

/// Reflection across the line through the origin at angle `theta` (radians, screen space).
pub fn reflect_matrix(o: Point, theta: f64) -> Affine {
    about(o, Affine::rotate(theta) * Affine::scale_non_uniform(1.0, -1.0) * Affine::rotate(-theta))
}

impl TransformTool {
    pub fn new(kind: TransformKind) -> Self {
        Self { kind, origin: None, axis: None, axis_to: None, drag: None, measure: None, readout: None, hover: vec![], snap: None }
    }

    /// The first point of the Reflect axis waiting for its second point, when it was set for the
    /// current selection (objects and anchors) and nothing in the document or the selection has
    /// changed since, so a selection changed and then changed back does not bring it back.
    fn axis_first(&self, cx: &ToolContext) -> Option<Point> {
        self.axis
            .as_ref()
            .filter(|a| !a.objects.is_empty() && a.revision == cx.revision && a.objects == cx.selection.objects && a.anchors == cx.selection.anchors)
            .map(|a| a.first)
    }

    /// Drops the Reflect axis waiting for its second point.
    fn drop_axis(&mut self) {
        self.axis = None;
        self.axis_to = None;
    }

    /// The effective reference point: the custom one if it belongs to the current selection,
    /// otherwise the centre of its bounding box (rotated with rotated objects).
    pub fn reference_point(&self, cx: &ToolContext) -> Option<Point> {
        if let Some((p, ids)) = &self.origin
            && *ids == cx.selection.objects
            && !ids.is_empty()
        {
            return Some(*p);
        }
        // Anchors selected with Direct Selection transform alone, about their own centre.
        vectorcraft_effects::preview::anchor_bounds_with(cx.selection, cx.doc, cx.bounds_measure())
            .map(|b| b.center())
            .or_else(|| selection_box(cx).map(|b| b.center()))
    }

    /// Transform for dragging from `start` to `p` about `o`; returns the matrix and a readout.
    pub fn matrix_for(&self, o: Point, start: Point, p: Point, shift: bool, bounds: Rect) -> (Affine, String) {
        self.matrix_in(0.0, o, start, p, shift, bounds)
    }

    /// [`TransformTool::matrix_for`] with the bounding box turned by `angle` (counter-clockwise
    /// degrees): Scale stretches along the box's own width and height, and Reflect's default axis
    /// and Shift's 45° steps start from the box's axes. A Reflect axis drawn by the drag is the
    /// line the pointer draws on the page.
    pub fn matrix_in(&self, angle: f64, o: Point, start: Point, p: Point, shift: bool, bounds: Rect) -> (Affine, String) {
        let (m, a, b) = self.solve(angle, o, start, p, shift, bounds);
        (m, self.label(a, b))
    }

    /// The canvas label for the two values [`Self::solve`] gives.
    fn label(&self, a: f64, b: f64) -> String {
        match self.kind {
            TransformKind::Rotate => format!("{:.1}°", -a),
            TransformKind::Reflect => format!("{a:.1}°"),
            TransformKind::Scale => format!("W: {:.1}%\nH: {:.1}%", a * 100.0, b * 100.0),
            TransformKind::Shear => format!("{:.1}°", if a == 0.0 { b } else { -b }),
        }
    }

    /// What the Info panel shows for the transform `m` of a selection with `bounds`, with the two
    /// values [`Self::solve`] gives.
    fn readout_of(&self, m: Affine, a: f64, b: f64, bounds: Rect) -> Readout {
        let center = m * bounds.center();
        match self.kind {
            TransformKind::Rotate => Readout::Rotate { center, angle: a },
            TransformKind::Reflect => Readout::Reflect { center, angle: a },
            TransformKind::Scale => Readout::Scale { sx: a, sy: b },
            TransformKind::Shear => Readout::Shear { center, axis: a, angle: b },
        }
    }

    /// [`TransformTool::matrix_in`]'s transform, with the two values of its readout: the angle
    /// turned, in degrees counter-clockwise as `object.rotate` takes it (Rotate), the angle of the
    /// axis in degrees counter-clockwise (Reflect), the scale along the box's width and height
    /// (Scale), or the angle of the shear axis (0 horizontal, 90 vertical) and the shear angle as
    /// `object.shear` takes it (Shear).
    fn solve(&self, angle: f64, o: Point, start: Point, p: Point, shift: bool, bounds: Rect) -> (Affine, f64, f64) {
        let frame = OrientedBox { angle, rect: Rect::ZERO };
        let local = |v: Vec2| frame.to_doc().inverse() * v.to_point();
        match self.kind {
            TransformKind::Rotate => {
                let (a, deg) = rotate_for_drag(o, start, p, shift);
                (a, deg, 0.0)
            }
            TransformKind::Reflect => {
                let mut v = p - o;
                if v.hypot() < 1e-9 {
                    v = (frame.to_doc() * Point::new(0.0, 1.0)).to_vec2();
                } else if shift {
                    v = (frame.to_doc() * vectorcraft_geom::constrain_angle(local(v).to_vec2(), 45.0).to_point()).to_vec2();
                }
                let theta = v.y.atan2(v.x);
                (reflect_matrix(o, theta), vectorcraft_geom::normalize_deg(-theta.to_degrees()), 0.0)
            }
            TransformKind::Scale => {
                let d0 = local(start - o).to_vec2();
                let d1 = local(p - o).to_vec2();
                let ratio = |a: f64, b: f64| if a.abs() > 1e-6 { b / a } else { 1.0 };
                let (mut sx, mut sy) = (ratio(d0.x, d1.x), ratio(d0.y, d1.y));
                if shift {
                    let l2 = d0.hypot2();
                    let k = if l2 > 1e-12 { d1.dot(d0) / l2 } else { 1.0 };
                    sx = k;
                    sy = k;
                }
                let clamp = |s: f64| if s.abs() < 1e-4 { 1e-4f64.copysign(s) } else { s };
                let (sx, sy) = (clamp(sx), clamp(sy));
                (about(o, frame.conjugate(Affine::scale_non_uniform(sx, sy))), sx, sy)
            }
            TransformKind::Shear => {
                // The shear axis follows the dominant drag direction. Shift keeps the drag on that
                // axis and locks the object's width (vertical shear) or height (horizontal shear).
                let mut d = p - start;
                if shift {
                    if d.x.abs() >= d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                let horizontal = d.x.abs() >= d.y.abs();
                let lever = |a: f64, fallback: f64| if a.abs() > 1e-3 { a } else { fallback.max(1.0) };
                let shear_k = |horizontal: bool| {
                    if horizontal { d.x / lever(start.y - o.y, bounds.height() / 2.0) } else { d.y / lever(start.x - o.x, bounds.width() / 2.0) }
                };
                let mut k = shear_k(horizontal);
                let mut m = if horizontal {
                    about(o, Affine::new([1.0, 0.0, k, 1.0, 0.0, 0.0]))
                } else {
                    about(o, Affine::new([1.0, k, 0.0, 1.0, 0.0, 0.0]))
                };
                if shift {
                    let (size0, size1) = if horizontal {
                        (bounds.height(), m.transform_rect_bbox(bounds).height())
                    } else {
                        (bounds.width(), m.transform_rect_bbox(bounds).width())
                    };
                    if size1 > 1e-9 && (size1 - size0).abs() > 1e-6 {
                        k *= size0 / size1;
                        m = if horizontal {
                            about(o, Affine::new([1.0, 0.0, k, 1.0, 0.0, 0.0]))
                        } else {
                            about(o, Affine::new([1.0, k, 0.0, 1.0, 0.0, 0.0]))
                        };
                    }
                }
                if horizontal { (m, 0.0, (-k.atan()).to_degrees()) } else { (m, 90.0, k.atan().to_degrees()) }
            }
        }
    }

    /// Ends the drag, its snapping and readouts with it.
    fn end_drag(&mut self) -> Option<Drag> {
        self.measure = None;
        self.readout = None;
        self.snap = None;
        self.hover.clear();
        self.drag.take()
    }

    fn dialog(&self, o: Point) -> Action {
        Action::Dialog(self.kind.id().into(), self.kind.dialog_fields(o))
    }
}

impl Tool for TransformTool {
    fn id(&self) -> &'static str {
        self.kind.id()
    }

    fn busy(&self) -> bool {
        self.drag.is_some_and(|d| d.began)
    }

    fn readout(&self, _cx: &ToolContext) -> Option<Readout> {
        self.readout.clone()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                let Some(bounds) = selection_bounds(cx) else { return vec![] };
                self.hover.clear();
                let (pick, _) = snap_pick(cx, p);
                // Alt: decided on release. A click sets the point and opens the dialog; a drag
                // transforms a copy about the reference point as it was. With a Reflect axis waiting
                // for its second point, a drag draws the axis from its first point.
                self.axis_to = None;
                let origin = self.axis_first(cx).or_else(|| self.reference_point(cx)).unwrap_or(bounds.center());
                let anchors = vectorcraft_effects::preview::anchor_bounds_with(cx.selection, cx.doc, cx.bounds_measure()).is_some();
                let angle = selection_box(cx).map_or(0.0, |b| b.angle);
                self.drag = Some(Drag {
                    start: p,
                    pick,
                    origin,
                    began: false,
                    alt: ev.mods.alt,
                    bounds,
                    angle,
                    anchors,
                    pointer: p,
                    shift: false,
                    last: Affine::IDENTITY,
                    revision: cx.revision,
                });
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut d) = self.drag else { return vec![] };
                let mut out = vec![];
                if !d.began {
                    if p.distance(d.start) < cx.tol(3.0) {
                        return out;
                    }
                    // The document or the selection changed since the press: its reference point
                    // (or Reflect axis) belongs to the selection as it was, so the press is dropped.
                    if d.revision != cx.revision {
                        self.end_drag();
                        return out;
                    }
                    d.began = true;
                    // The selection moves, so only the other objects are targets. The Scale tool's
                    // drag scales; the others move the art about the reference point.
                    let targets = || Targets::collect(cx.doc, &cx.selection.objects, None);
                    self.snap =
                        Some(if self.kind == TransformKind::Scale { PointSnap::scaling(cx, targets) } else { PointSnap::moving(cx, targets) });
                    out.push(Action::Begin(self.kind.label().into()));
                }
                let (p, guides) = self.snap.as_ref().map_or((p, vec![]), |s| s.snap(cx, p));
                self.hover = guides;
                let (m, a, b) = self.solve(d.angle_for(ev.mods.alt, ev.backquote), d.origin, d.pick, p, ev.mods.shift, d.bounds);
                (d.last, d.pointer, d.shift) = (m, p, ev.mods.shift);
                self.drag = Some(d);
                self.measure = Some((p, self.label(a, b)));
                self.readout = Some(self.readout_of(m, a, b, d.bounds));
                out.push(preview(m, ev.mods.alt, ev.backquote));
                out
            }
            PointerKind::Up => {
                let Some(d) = self.end_drag() else { return vec![] };
                if d.began {
                    // The Alt and backquote states at release decide whether the result is a copy
                    // or moves only pattern tiles, and with anchors selected, whether it acts
                    // along the box or the page.
                    let m = if d.anchors {
                        self.matrix_in(d.angle_for(ev.mods.alt, ev.backquote), d.origin, d.pick, d.pointer, d.shift, d.bounds).0
                    } else {
                        d.last
                    };
                    // A drag draws a Reflect axis to its end, so it uses up an axis waiting for
                    // its second point.
                    self.drop_axis();
                    vec![preview(m, ev.mods.alt, ev.backquote), Action::Commit]
                } else if d.revision != cx.revision {
                    // The document or the selection changed since the press, as for a drag: the
                    // release sets no reference point or Reflect axis.
                    vec![]
                } else if let (TransformKind::Reflect, Some(first)) = (self.kind, self.axis_first(cx)) {
                    // The second point of a Reflect axis: the selection (with Alt at the release, as
                    // for a drag, a copy) reflects across the line through both points, as one undo
                    // step. A click on the first point does not define a line, so the axis keeps
                    // waiting.
                    if d.pick.distance(first) < cx.tol(3.0) {
                        return vec![];
                    }
                    self.drop_axis();
                    let copy = ev.mods.alt;
                    let (m, _, _) = self.solve(d.angle_for(copy, ev.backquote), first, first, d.pick, false, d.bounds);
                    vec![Action::Begin(self.kind.label().into()), preview(m, copy, ev.backquote), Action::Commit]
                } else {
                    // A click sets the reference point; with Alt, it opens the dialog too. With the
                    // Reflect tool, a plain click also sets the first point of an axis.
                    self.origin = Some((d.pick, cx.selection.objects.clone()));
                    self.drop_axis();
                    if d.alt {
                        vec![self.dialog(d.pick)]
                    } else {
                        if self.kind == TransformKind::Reflect {
                            self.axis = Some(Axis {
                                first: d.pick,
                                objects: cx.selection.objects.clone(),
                                anchors: cx.selection.anchors.clone(),
                                revision: cx.revision,
                            });
                        }
                        vec![]
                    }
                }
            }
            PointerKind::DoubleClick => {
                self.end_drag();
                self.drop_axis();
                match self.reference_point(cx) {
                    Some(o) => vec![self.dialog(o)],
                    None => vec![],
                }
            }
            PointerKind::TripleClick => vec![],
            PointerKind::Move => {
                if self.drag.is_none() {
                    let (pick, hover) = if cx.selection.is_empty() { (p, vec![]) } else { snap_pick(cx, p) };
                    self.hover = hover;
                    self.axis_to = self.axis_first(cx).map(|_| pick);
                }
                vec![]
            }
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            // Escape cancels a drag, and drops the Reflect axis the drag was drawing from too.
            ToolKey::Escape if self.busy() => {
                self.end_drag();
                self.drop_axis();
                vec![Action::Cancel]
            }
            // Escape drops a Reflect axis waiting for its second point, and a press not yet
            // released with it, so the release sets no point; the reference point stays.
            ToolKey::Escape if self.axis.is_some() => {
                self.end_drag();
                self.drop_axis();
                vec![]
            }
            // Enter opens the dialog. A press not yet a drag ends with it, so its release sets no
            // reference point or Reflect axis behind the dialog.
            ToolKey::Enter => {
                if !self.busy() {
                    self.end_drag();
                }
                self.drop_axis();
                self.reference_point(cx).map(|o| vec![self.dialog(o)]).unwrap_or_default()
            }
            _ => vec![],
        }
    }

    /// A dropped press ends whether or not it became a drag, so the release the engine sends after
    /// it sets no reference point or Reflect axis; a Reflect axis waiting for its second point is
    /// dropped too, as with Escape.
    fn drop_press(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drop_axis();
        if self.end_drag().is_some_and(|d| d.began) { vec![Action::Cancel] } else { vec![] }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        let reference = match self.drag {
            Some(d) => Some(d.origin),
            None => self.reference_point(cx),
        };
        if let Some(r) = reference {
            o.extend(target_overlays(r, cx.tol(8.0)));
        }
        o.extend(self.hover.iter().cloned());
        if let (Some(d), TransformKind::Reflect | TransformKind::Rotate) = (self.drag, self.kind)
            && d.began
            && let Some((p, _)) = &self.measure
        {
            o.push(Overlay::Line { a: d.origin, b: *p, color: super::CYAN, dashed: true });
        }
        // A Reflect axis waiting for its second point: a dashed line from its first point to the
        // pointer.
        if self.drag.is_none()
            && let (Some(a), Some(b)) = (self.axis_first(cx), self.axis_to)
        {
            o.push(Overlay::Line { a, b, color: super::CYAN, dashed: true });
        }
        if let (Some((p, t)), true) = (&self.measure, cx.transform_tools_guides) {
            o.push(Overlay::Measure { p: *p + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: t.clone() });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        if cx.selection.is_empty() {
            return Cursor::NotAllowed;
        }
        match self.kind {
            TransformKind::Rotate if self.busy() => Cursor::Rotate,
            _ => Cursor::Crosshair,
        }
    }

    fn options(&self) -> Value {
        match &self.origin {
            Some((p, _)) => json!({ "origin": [p.x, p.y] }),
            None => json!({ "origin": null }),
        }
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        if key == "origin" {
            self.origin = value.as_array().and_then(|a| Some((Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?), vec![])));
            self.drop_axis();
        }
    }

    fn ends_on_escape(&self, cx: &ToolContext) -> bool {
        self.axis_first(cx).is_some()
    }

    /// An edit, Undo or Redo made outside the tool, or another document, drops a Reflect axis
    /// waiting for its second point.
    fn history_moved(&mut self) {
        self.drop_axis();
    }

    /// `origin` is `[x, y]`, or null to clear it.
    fn check_option(&self, key: &str, value: &Value) -> Result<(), String> {
        if key != "origin" {
            return Err(crate::option_check::unknown(key));
        }
        let point = value.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(|n| n.as_f64().is_some_and(f64::is_finite)));
        if point || value.is_null() { Ok(()) } else { Err("`origin` must be [x, y] with finite numbers, or null".into()) }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drop_axis();
        if self.end_drag().is_some_and(|d| d.began) { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }
    #[test]
    fn backquote_drag_previews_and_commits_pattern_only_for_each_transform_tool() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        for kind in [TransformKind::Rotate, TransformKind::Reflect, TransformKind::Scale, TransformKind::Shear] {
            let mut t = TransformTool::new(kind);
            t.pointer(&cx, &ev(PointerKind::Down, 250.0, 170.0));
            let drag = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, 290.0, 210.0) });
            assert!(
                matches!(drag.as_slice(), [Action::Begin(_), Action::Preview(c, v)] if c == "object.transform" && v["objects"] == false && v["patterns"] == true),
                "{kind:?}: {drag:?}"
            );
            assert_ne!(matrix(&drag[1]), Affine::IDENTITY, "{kind:?}");
            let up = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Up, 290.0, 210.0) });
            assert!(
                matches!(up.as_slice(), [Action::Preview(_, v), Action::Commit] if v["objects"] == false && v["patterns"] == true),
                "{kind:?}: {up:?}"
            );
            let mut t = TransformTool::new(kind);
            t.pointer(&cx, &ev(PointerKind::Down, 250.0, 170.0));
            t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, 290.0, 210.0) });
            let up = t.pointer(&cx, &ev(PointerKind::Up, 290.0, 210.0));
            assert!(
                matches!(up.as_slice(), [Action::Preview(_, v), Action::Commit] if v.get("objects").is_none() && v.get("patterns").is_none()),
                "{kind:?}: {up:?}"
            );
            let mut t = TransformTool::new(kind);
            t.pointer(&cx, &ev(PointerKind::Down, 250.0, 170.0));
            t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, 290.0, 210.0) });
            assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
            assert!(t.pointer(&cx, &ev(PointerKind::Up, 290.0, 210.0)).is_empty());
        }
        let mut t = TransformTool::new(TransformKind::Scale);
        t.pointer(&cx, &ev(PointerKind::Down, 250.0, 170.0));
        let mods = Mods { alt: true, shift: true, ..Default::default() };
        let drag = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, 290.0, 210.0).with_mods(mods) });
        assert!(matches!(&drag[1], Action::Preview(_, v) if v["copy"] == true && v["objects"] == false && v["patterns"] == true));
        let c = matrix(&drag[1]).as_coeffs();
        assert!((c[0] - c[3]).abs() < 1e-9);
        let up = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Up, 290.0, 210.0).with_mods(mods) });
        assert!(matches!(&up[0], Action::Preview(_, v) if v["copy"] == true && v["objects"] == false));
    }
    fn alt() -> Mods {
        Mods { alt: true, ..Default::default() }
    }

    fn matrix(a: &Action) -> Affine {
        let Action::Preview(c, v) = a else { panic!("not a preview: {a:?}") };
        assert_eq!(c, "object.transform");
        let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]])
    }

    #[test]
    fn rotate_drag_about_center_single_undo() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TransformTool::new(TransformKind::Rotate);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 250.0, 150.0)).is_empty());
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 250.0));
        assert_eq!(a[0], Action::Begin("Rotate".into()));
        // Centre (150,150) is fixed; (250,150) → (150,250).
        let m = matrix(&a[1]);
        assert!((m * Point::new(150.0, 150.0)).distance(Point::new(150.0, 150.0)) < 1e-9);
        assert!((m * Point::new(250.0, 150.0)).distance(Point::new(150.0, 250.0)) < 1e-9);
        let up = t.pointer(&cx, &ev(PointerKind::Up, 150.0, 250.0));
        assert_eq!(up.last(), Some(&Action::Commit));
    }

    #[test]
    fn click_sets_reference_point_then_drag_uses_it() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TransformTool::new(TransformKind::Scale);
        t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 100.0, 100.0)).is_empty());
        assert_eq!(t.reference_point(&cx), Some(Point::new(100.0, 100.0)));
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Line { a, .. } if a.y == 100.0)));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 250.0));
        let m = matrix(&a[1]);
        assert!((m * Point::new(100.0, 100.0)).distance(Point::new(100.0, 100.0)) < 1e-9);
        assert!((m * Point::new(200.0, 200.0)).distance(Point::new(300.0, 250.0)) < 1e-9);
    }

    #[test]
    fn scale_shift_is_uniform() {
        let t = TransformTool::new(TransformKind::Scale);
        let (m, _) = t.matrix_for(Point::new(150.0, 150.0), Point::new(200.0, 200.0), Point::new(250.0, 240.0), true, Rect::ZERO);
        let c = m.as_coeffs();
        assert!((c[0] - c[3]).abs() < 1e-9 && c[0] > 1.0);
    }

    #[test]
    fn alt_click_opens_dialog_and_alt_release_copies() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TransformTool::new(TransformKind::Reflect);
        // The release decides: a click with Alt opens the dialog…
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 120.0, 130.0).with_mods(alt())), vec![]);
        let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 130.0).with_mods(alt()));
        assert_eq!(a, vec![Action::Dialog("reflect".into(), json!({"axis": "vertical", "origin": [120.0, 130.0]}))]);
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 200.0));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 200.0).with_mods(alt()));
        assert!(matches!(&up[0], Action::Preview(_, v) if v["copy"] == true));
        assert_eq!(up[1], Action::Commit);
        let mut t = TransformTool::new(TransformKind::Shear);
        assert!(
            matches!(&t.pointer(&cx, &ev(PointerKind::DoubleClick, 0.0, 0.0))[0], Action::Dialog(k, v) if k == "shear" && v["axis"] == "horizontal")
        );
        let mut t = TransformTool::new(TransformKind::Rotate);
        assert!(matches!(&t.key(&cx, ToolKey::Enter, Mods::default())[0], Action::Dialog(k, v) if k == "rotate" && v["angle"] == 0));
    }

    #[test]
    fn alt_drag_transforms_a_copy_and_leaves_the_dialog_shut() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        for kind in [TransformKind::Rotate, TransformKind::Reflect, TransformKind::Scale, TransformKind::Shear] {
            let mut t = TransformTool::new(kind);
            // Alt down at the press, then a drag: no dialog, and the preview and the release copy.
            assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 260.0, 150.0).with_mods(alt())), vec![], "{kind:?}");
            let a = t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 180.0).with_mods(alt()));
            assert_eq!(a[0], Action::Begin(kind.label().into()), "{kind:?}");
            assert!(matches!(&a[1], Action::Preview(c, v) if c == "object.transform" && v["copy"] == true), "{kind:?}: {a:?}");
            let up = t.pointer(&cx, &ev(PointerKind::Up, 300.0, 180.0).with_mods(alt()));
            assert!(matches!(&up[0], Action::Preview(_, v) if v["copy"] == true), "{kind:?}: {up:?}");
            assert_eq!(up[1], Action::Commit);
            // The reference point is the one it was (the centre), not the press point.
            assert_eq!(t.reference_point(&cx), Some(Point::new(150.0, 150.0)), "{kind:?}");
        }
    }

    #[test]
    fn alt_click_and_click_snap_the_reference_point_to_anchors_and_centres() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        // Alt-click 3 px from the top-left corner: the reflect pivots exactly on the corner.
        let mut t = TransformTool::new(TransformKind::Reflect);
        t.pointer(&c, &ev(PointerKind::Down, 102.0, 98.0).with_mods(alt()));
        let a = t.pointer(&c, &ev(PointerKind::Up, 102.0, 98.0).with_mods(alt()));
        assert_eq!(a, vec![Action::Dialog("reflect".into(), json!({"axis": "vertical", "origin": [100.0, 100.0]}))]);
        // A plain click near the bottom-right corner, then near the centre.
        let mut t = TransformTool::new(TransformKind::Rotate);
        t.pointer(&c, &ev(PointerKind::Down, 198.0, 203.0));
        t.pointer(&c, &ev(PointerKind::Up, 198.0, 203.0));
        assert_eq!(t.reference_point(&c), Some(Point::new(200.0, 200.0)));
        t.pointer(&c, &ev(PointerKind::Down, 151.0, 148.0));
        t.pointer(&c, &ev(PointerKind::Up, 151.0, 148.0));
        assert_eq!(t.reference_point(&c), Some(Point::new(150.0, 150.0)));
        // Hovering a corner labels it before the click.
        t.pointer(&c, &ev(PointerKind::Move, 199.0, 101.0));
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "anchor" && *p == Point::new(200.0, 100.0))));
        t.pointer(&c, &ev(PointerKind::Move, 175.0, 120.0));
        assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { .. })));
        // Snapping off: the point stays where it was clicked.
        c.snap_to_point = false;
        c.smart_guides = false;
        t.pointer(&c, &ev(PointerKind::Down, 102.0, 98.0).with_mods(alt()));
        let a = t.pointer(&c, &ev(PointerKind::Up, 102.0, 98.0).with_mods(alt()));
        assert!(matches!(&a[0], Action::Dialog(_, v) if v["origin"] == json!([102.0, 98.0])));
    }

    /// Smart Guides while a transform tool drags (#482): the drag starts from the selection's
    /// anchor it grabbed and the pointer lands on another object's anchor, so the scale is exact.
    #[test]
    fn scale_drag_snaps_from_an_anchor_onto_another_objects_anchor() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let other = d.alloc_id();
        d.insert(
            Some(l),
            1,
            Node::path(other, vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 260.0, 340.0, 300.0)), Appearance::default_art()),
        )
        .unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        let mut t = TransformTool::new(TransformKind::Scale);
        // The reference point on the top-left corner, then a drag from near the bottom-right one
        // to near the other rect's top-left corner.
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 100.0));
        t.pointer(&c, &ev(PointerKind::Up, 100.0, 100.0));
        t.pointer(&c, &ev(PointerKind::Down, 199.0, 198.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 302.0, 262.0));
        let m = matrix(&a[1]);
        assert!((m * Point::new(200.0, 200.0)).distance(Point::new(300.0, 260.0)) < 1e-9, "{m:?}");
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "anchor" && *p == Point::new(300.0, 260.0))));
        t.pointer(&c, &ev(PointerKind::Up, 302.0, 262.0));
        assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { .. })));
        // Smart Guides and Snap to Point off: the pointer is where it is.
        (c.smart_guides, c.snap_to_point) = (false, false);
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 302.0, 262.0));
        assert!((matrix(&a[1]) * Point::new(200.0, 200.0)).distance(Point::new(302.0, 262.0)) < 1e-9);
    }

    /// Sends a click (a press and a release, with `mods`) at `(x, y)` and returns the release's actions.
    fn click(t: &mut TransformTool, cx: &ToolContext, x: f64, y: f64, mods: Mods) -> Vec<Action> {
        assert!(t.pointer(cx, &ev(PointerKind::Down, x, y).with_mods(mods)).is_empty());
        t.pointer(cx, &ev(PointerKind::Up, x, y).with_mods(mods))
    }

    /// Returns the matrix of a two-click reflection: a Begin, one `object.transform` preview with
    /// `copy`, and a Commit.
    fn reflected(a: &[Action], copy: bool) -> Affine {
        assert!(
            matches!(a, [Action::Begin(l), Action::Preview(c, v), Action::Commit] if l == "Reflect" && c == "object.transform" && v["copy"] == copy),
            "{a:?}"
        );
        matrix(&a[1])
    }

    /// The Reflect tool: a click sets the first point of an axis, and a second click reflects the
    /// selection across the line through both points, as one interaction. The next click starts a
    /// new axis.
    #[test]
    fn a_second_reflect_click_reflects_across_the_line_through_both_points() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let none = Mods::default();
        let mut t = TransformTool::new(TransformKind::Reflect);
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        assert_eq!(t.reference_point(&c), Some(Point::new(100.0, 100.0)));
        assert!(t.ends_on_escape(&c));
        // Hovering draws the axis from the first point to the pointer.
        t.pointer(&c, &ev(PointerKind::Move, 200.0, 260.0));
        let line = Overlay::Line { a: Point::new(100.0, 100.0), b: Point::new(200.0, 260.0), color: super::super::CYAN, dashed: true };
        assert!(t.overlays(&c).contains(&line), "{:?}", t.overlays(&c));
        let m = reflected(&click(&mut t, &c, 200.0, 200.0, none), false);
        for fixed in [Point::new(100.0, 100.0), Point::new(200.0, 200.0), Point::new(300.0, 300.0)] {
            assert!((m * fixed).distance(fixed) < 1e-9, "{m:?}");
        }
        assert!((m * Point::new(200.0, 100.0)).distance(Point::new(100.0, 200.0)) < 1e-9, "{m:?}");
        // The axis is used up; the first point stays the reference point.
        assert!(!t.ends_on_escape(&c));
        assert!(!t.overlays(&c).contains(&line));
        assert_eq!(t.reference_point(&c), Some(Point::new(100.0, 100.0)));
        // The next click starts a new axis, and the one after it reflects across a vertical line.
        assert!(click(&mut t, &c, 120.0, 50.0, none).is_empty());
        let m = reflected(&click(&mut t, &c, 120.0, 250.0, none), false);
        assert!((m * Point::new(150.0, 130.0)).distance(Point::new(90.0, 130.0)) < 1e-9, "{m:?}");
        // Shift on a plain second click does not constrain the line.
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        let shift = Mods { shift: true, ..Default::default() };
        let m = reflected(&click(&mut t, &c, 210.0, 190.0, shift), false);
        assert!((m * Point::new(210.0, 190.0)).distance(Point::new(210.0, 190.0)) < 1e-9, "{m:?}");
    }

    /// Alt on the second click reflects a copy and opens no dialog; Alt on a first click still opens
    /// the dialog and starts no axis.
    #[test]
    fn alt_on_the_second_reflect_click_reflects_a_copy() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let mut t = TransformTool::new(TransformKind::Reflect);
        assert!(click(&mut t, &c, 250.0, 100.0, Mods::default()).is_empty());
        let m = reflected(&click(&mut t, &c, 250.0, 200.0, alt()), true);
        assert!((m * Point::new(150.0, 150.0)).distance(Point::new(350.0, 150.0)) < 1e-9, "{m:?}");
        // As for a drag, Alt at the release decides: held only at the press, the original reflects;
        // pressed during the click, a copy does.
        for (press, release) in [(alt(), Mods::default()), (Mods::default(), alt())] {
            assert!(click(&mut t, &c, 250.0, 100.0, Mods::default()).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Down, 250.0, 200.0).with_mods(press)).is_empty());
            reflected(&t.pointer(&c, &ev(PointerKind::Up, 250.0, 200.0).with_mods(release)), release.alt);
        }
        // Alt on a first click: the dialog, and the click after it is a first click again.
        assert_eq!(
            click(&mut t, &c, 120.0, 130.0, alt()),
            vec![Action::Dialog("reflect".into(), json!({"axis": "vertical", "origin": [120.0, 130.0]}))]
        );
        assert!(!t.ends_on_escape(&c));
        assert!(click(&mut t, &c, 130.0, 160.0, Mods::default()).is_empty());
        assert!(t.ends_on_escape(&c));
    }

    /// A second click on the first point does not define a line: nothing happens and the axis keeps
    /// waiting. Escape, Enter, a double-click, a changed selection, an edit made outside the tool,
    /// a new reference point and switching tools drop the axis, so the next click starts a new one.
    #[test]
    fn a_reflect_axis_waits_through_a_click_on_its_first_point_and_drops_on_escape_or_a_new_selection() {
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let other = d.alloc_id();
        d.insert(
            Some(l),
            1,
            vectorcraft_doc::Node::path(
                other,
                vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 340.0, 340.0)),
                vectorcraft_doc::Appearance::default_art(),
            ),
        )
        .unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let none = Mods::default();
        let mut t = TransformTool::new(TransformKind::Reflect);
        click(&mut t, &c, 100.0, 100.0, none);
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        assert!(click(&mut t, &c, 101.0, 102.0, none).is_empty());
        assert!(t.ends_on_escape(&c));
        reflected(&click(&mut t, &c, 100.0, 200.0, none), false);

        // Each way of dropping the axis, then a click that only starts a new one.
        type DropAxis = fn(&mut TransformTool, &ToolContext);
        let drops: [(&str, DropAxis); 6] = [
            ("escape", |t, c| assert!(t.key(c, ToolKey::Escape, Mods::default()).is_empty())),
            ("enter", |t, c| assert!(matches!(t.key(c, ToolKey::Enter, Mods::default()).as_slice(), [Action::Dialog(..)]))),
            ("double-click", |t, c| assert!(matches!(t.pointer(c, &ev(PointerKind::DoubleClick, 100.0, 100.0)).as_slice(), [Action::Dialog(..)]))),
            ("history", |t, _| t.history_moved()),
            ("origin", |t, _| t.set_option("origin", &json!([10.0, 10.0]))),
            ("deactivate", |t, c| assert!(t.deactivate(c).is_empty())),
        ];
        for (name, drop) in drops {
            click(&mut t, &c, 100.0, 100.0, none);
            assert!(t.ends_on_escape(&c), "{name}");
            drop(&mut t, &c);
            assert!(!t.ends_on_escape(&c), "{name}");
            assert!(click(&mut t, &c, 100.0, 200.0, none).is_empty(), "{name}");
            assert!(t.ends_on_escape(&c), "{name}");
            t.key(&c, ToolKey::Escape, Mods::default());
        }

        // A double-click elsewhere arrives as two clicks and then the double-click: its first click
        // sets the second point and reflects, its second click starts a new axis, and the
        // double-click drops that axis and opens the dialog there.
        click(&mut t, &c, 100.0, 100.0, none);
        reflected(&click(&mut t, &c, 100.0, 200.0, none), false);
        assert!(click(&mut t, &c, 100.0, 200.0, none).is_empty());
        let a = t.pointer(&c, &ev(PointerKind::DoubleClick, 100.0, 200.0));
        assert_eq!(a, vec![Action::Dialog("reflect".into(), json!({"axis": "vertical", "origin": [100.0, 200.0]}))]);
        assert!(!t.ends_on_escape(&c));

        // The selection changed and changed back (or the document edited) moves the revision: the
        // axis set before it does not come back.
        click(&mut t, &c, 100.0, 100.0, none);
        let mut moved = cx(&d, &s, &p);
        (moved.smart_guides, moved.snap_to_point) = (false, false);
        moved.revision = (c.revision.0, c.revision.1 + 1);
        assert!(!t.ends_on_escape(&moved));
        assert!(click(&mut t, &moved, 100.0, 200.0, none).is_empty());
        reflected(&click(&mut t, &moved, 200.0, 200.0, none), false);

        // Another object selected, then nothing selected: the axis set for the first selection
        // does not reflect them.
        click(&mut t, &c, 100.0, 100.0, none);
        let mut s2 = Selection::default();
        s2.add(other);
        let mut c2 = cx(&d, &s2, &p);
        (c2.smart_guides, c2.snap_to_point) = (false, false);
        assert!(!t.ends_on_escape(&c2));
        assert!(click(&mut t, &c2, 100.0, 200.0, none).is_empty());
        let empty = Selection::default();
        let c3 = cx(&d, &empty, &p);
        assert!(t.pointer(&c3, &ev(PointerKind::Down, 100.0, 300.0)).is_empty());
        assert!(t.pointer(&c3, &ev(PointerKind::Up, 100.0, 300.0)).is_empty());
        // Anchors selected with Direct Selection make a different selection too.
        let mut s4 = s.clone();
        s4.anchors.insert(id, [(0, 0), (0, 1)].into());
        let mut c4 = cx(&d, &s4, &p);
        (c4.smart_guides, c4.snap_to_point) = (false, false);
        click(&mut t, &c, 100.0, 100.0, none);
        assert!(!t.ends_on_escape(&c4));
        assert!(click(&mut t, &c4, 100.0, 200.0, none).is_empty());
        // With those anchors selected, the second click reflects them, across the line y = 200.
        let m = reflected(&click(&mut t, &c4, 200.0, 200.0, none), false);
        assert!((m * Point::new(150.0, 150.0)).distance(Point::new(150.0, 250.0)) < 1e-9, "{m:?}");

        // The other transform tools only move the reference point on a second click.
        for kind in [TransformKind::Rotate, TransformKind::Scale, TransformKind::Shear] {
            let mut t = TransformTool::new(kind);
            assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty(), "{kind:?}");
            assert!(click(&mut t, &c, 100.0, 200.0, none).is_empty(), "{kind:?}");
            assert!(!t.ends_on_escape(&c), "{kind:?}");
            assert_eq!(t.reference_point(&c), Some(Point::new(100.0, 200.0)), "{kind:?}");
        }
    }

    /// Escape pressed between the press and the release of the second click drops the axis and the
    /// press with it: the release starts no new axis and moves no reference point, so the click
    /// after it starts an axis and reflects nothing. Escape during a drag from the first point
    /// cancels the drag and drops the axis too.
    #[test]
    fn escape_during_the_second_reflect_click_cancels_that_click() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let none = Mods::default();
        let mut t = TransformTool::new(TransformKind::Reflect);
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        assert!(t.key(&c, ToolKey::Escape, none).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 200.0, 200.0)).is_empty());
        assert!(!t.ends_on_escape(&c));
        assert_eq!(t.reference_point(&c), Some(Point::new(100.0, 100.0)));
        assert!(click(&mut t, &c, 150.0, 300.0, none).is_empty());
        assert!(t.ends_on_escape(&c));
        // A drag from the first point, then Escape.
        assert!(t.pointer(&c, &ev(PointerKind::Down, 300.0, 300.0)).is_empty());
        assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, 100.0, 250.0))[0], Action::Begin("Reflect".into()));
        assert_eq!(t.key(&c, ToolKey::Escape, none), vec![Action::Cancel]);
        assert!(!t.ends_on_escape(&c));
        assert!(t.pointer(&c, &ev(PointerKind::Up, 100.0, 250.0)).is_empty());
        assert!(click(&mut t, &c, 200.0, 200.0, none).is_empty());
    }

    /// Enter between the press and the release of the second click opens the dialog at the first
    /// point and ends the press: the release sets no point and starts no axis. A press the engine
    /// drops ends the same way, with or without an axis waiting, and a dropped drag is canceled. A
    /// press whose document or selection changes before it becomes a drag is dropped.
    #[test]
    fn enter_or_a_dropped_press_during_a_reflect_click_ends_that_press() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let none = Mods::default();
        let first = Point::new(100.0, 100.0);
        let mut t = TransformTool::new(TransformKind::Reflect);
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        assert_eq!(t.key(&c, ToolKey::Enter, none), vec![Action::Dialog("reflect".into(), json!({"axis": "vertical", "origin": [100.0, 100.0]}))]);
        assert!(t.pointer(&c, &ev(PointerKind::Up, 200.0, 200.0)).is_empty());
        assert!(!t.ends_on_escape(&c));
        assert_eq!(t.reference_point(&c), Some(first));

        // A dropped first press: its release sets no point and starts no axis.
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        assert!(t.drop_press(&c).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 200.0, 200.0)).is_empty());
        assert!(!t.ends_on_escape(&c));
        assert_eq!(t.reference_point(&c), Some(first));
        // A dropped second press drops the axis too, so the next click is a first click.
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        assert!(t.drop_press(&c).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 200.0, 200.0)).is_empty());
        assert!(!t.ends_on_escape(&c));
        assert!(click(&mut t, &c, 150.0, 300.0, none).is_empty());
        // A dropped drag is canceled, and its release commits nothing.
        assert!(t.pointer(&c, &ev(PointerKind::Down, 300.0, 300.0)).is_empty());
        assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, 100.0, 250.0))[0], Action::Begin("Reflect".into()));
        assert_eq!(t.drop_press(&c), vec![Action::Cancel]);
        assert!(t.pointer(&c, &ev(PointerKind::Up, 100.0, 250.0)).is_empty());
        assert!(!t.ends_on_escape(&c));

        // The selection changes between the press and the first drag event: the press is dropped,
        // and neither the drag nor its release transforms anything.
        assert!(click(&mut t, &c, 100.0, 100.0, none).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        let mut moved = cx(&d, &s, &p);
        (moved.smart_guides, moved.snap_to_point) = (false, false);
        moved.revision = (c.revision.0, c.revision.1 + 1);
        assert!(t.pointer(&moved, &ev(PointerKind::Drag, 100.0, 250.0)).is_empty());
        assert!(!t.busy());
        assert!(t.pointer(&moved, &ev(PointerKind::Up, 100.0, 250.0)).is_empty());
    }

    /// A press whose document or selection changes before a release that is not a drag sets no
    /// reference point and starts no Reflect axis, so the next click is a first click.
    #[test]
    fn a_reflect_press_released_after_the_document_changed_sets_nothing() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let none = Mods::default();
        let mut moved = cx(&d, &s, &p);
        (moved.smart_guides, moved.snap_to_point) = (false, false);
        moved.revision = (c.revision.0, c.revision.1 + 1);
        let mut t = TransformTool::new(TransformKind::Reflect);
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        assert!(t.pointer(&moved, &ev(PointerKind::Up, 200.0, 200.0)).is_empty());
        assert!(!t.ends_on_escape(&moved));
        assert_ne!(t.reference_point(&moved), Some(Point::new(200.0, 200.0)));
        assert!(click(&mut t, &moved, 150.0, 300.0, none).is_empty());
        // The other transform tools set no reference point either.
        let mut t = TransformTool::new(TransformKind::Rotate);
        assert!(t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0)).is_empty());
        assert!(t.pointer(&moved, &ev(PointerKind::Up, 200.0, 200.0)).is_empty());
        assert_ne!(t.reference_point(&moved), Some(Point::new(200.0, 200.0)));
    }

    /// After a click sets the first point, a drag draws the axis from it (Shift in 45° steps from a
    /// turned box), and its release uses the axis up. With the selection turned, two clicks still
    /// reflect across the line through the clicked points on the page.
    #[test]
    fn a_drag_after_a_reflect_click_draws_the_axis_from_the_first_point() {
        let (mut d, id) = doc_with_rect();
        let c0 = Point::new(150.0, 150.0);
        d.node_mut(id).unwrap().transform(about(c0, Affine::rotate(-30f64.to_radians())), false);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        (c.smart_guides, c.snap_to_point) = (false, false);
        let none = Mods::default();
        let mut t = TransformTool::new(TransformKind::Reflect);
        click(&mut t, &c, 100.0, 100.0, none);
        t.pointer(&c, &ev(PointerKind::Down, 300.0, 300.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 100.0, 250.0));
        assert_eq!(a[0], Action::Begin("Reflect".into()));
        let m = matrix(&a[1]);
        assert!((m * Point::new(100.0, 180.0)).distance(Point::new(100.0, 180.0)) < 1e-9, "{m:?}");
        assert!((m * Point::new(150.0, 150.0)).distance(Point::new(50.0, 150.0)) < 1e-9, "{m:?}");
        // Shift: a pointer a few degrees off the box's vertical axis steps onto it.
        let frame = OrientedBox { angle: 30.0, rect: Rect::ZERO };
        let along = |x: f64, y: f64| Point::new(100.0, 100.0) + (frame.to_doc() * Point::new(x, y)).to_vec2();
        let to = along(3.0, 50.0);
        let a = t.pointer(&c, &ev(PointerKind::Drag, to.x, to.y).with_mods(Mods { shift: true, ..Default::default() }));
        assert!((matrix(&a[0]) * along(0.0, 50.0)).distance(along(0.0, 50.0)) < 1e-9, "{a:?}");
        let up = t.pointer(&c, &ev(PointerKind::Up, to.x, to.y));
        assert_eq!(up.last(), Some(&Action::Commit));
        assert!(!t.ends_on_escape(&c));
        assert!(click(&mut t, &c, 100.0, 300.0, none).is_empty());
        // Two clicks with the box turned: the line through the clicked points on the page.
        let m = reflected(&click(&mut t, &c, 300.0, 300.0, none), false);
        assert!((m * Point::new(200.0, 250.0)).distance(Point::new(200.0, 350.0)) < 1e-9, "{m:?}");
        // A drag with no axis waiting still draws it from the reference point.
        let mut t = TransformTool::new(TransformKind::Reflect);
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 50.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 20.0));
        assert!((matrix(&a[1]) * Point::new(100.0, 120.0)).distance(Point::new(200.0, 120.0)) < 1e-9);
    }

    #[test]
    fn reflect_vertical_axis_mirrors_x() {
        let t = TransformTool::new(TransformKind::Reflect);
        let o = Point::new(150.0, 150.0);
        let (m, _) = t.matrix_for(o, Point::new(150.0, 100.0), Point::new(150.0, 50.0), false, Rect::ZERO);
        assert!((m * Point::new(100.0, 120.0)).distance(Point::new(200.0, 120.0)) < 1e-9);
    }

    /// With the selection turned by 30°, a Scale drag stretches along the box's own sides, and
    /// Reflect's default axis and Shift steps start from the box's axes while a drawn axis stays
    /// the line drawn on the page.
    #[test]
    fn scale_and_reflect_drags_follow_a_turned_bounding_box() {
        let (mut d, id) = doc_with_rect();
        let c = Point::new(150.0, 150.0);
        d.node_mut(id).unwrap().transform(about(c, Affine::rotate(-30f64.to_radians())), false);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        (cx.smart_guides, cx.snap_to_point) = (false, false);
        let frame = OrientedBox { angle: 30.0, rect: Rect::ZERO };
        let along = |x: f64, y: f64| c + (frame.to_doc() * Point::new(x, y)).to_vec2();
        // The right side's middle dragged out to twice its distance: the width doubles along the
        // box, and the height stays.
        let mut t = TransformTool::new(TransformKind::Scale);
        let (from, to) = (along(50.0, 0.0), along(100.0, 0.0));
        t.pointer(&cx, &ev(PointerKind::Down, from.x, from.y));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, to.x, to.y));
        let m = matrix(&a[1]);
        assert!((m * along(50.0, 50.0)).distance(along(100.0, 50.0)) < 1e-9, "{m:?}");
        assert!((m * along(0.0, 50.0)).distance(along(0.0, 50.0)) < 1e-9, "{m:?}");
        assert!(a.iter().any(|x| matches!(x, Action::Preview(..))));

        let t = TransformTool::new(TransformKind::Reflect);
        // No axis drawn: the box's vertical axis.
        let (m, _) = t.matrix_in(30.0, c, c, c, false, Rect::ZERO);
        assert!((m * along(40.0, 10.0)).distance(along(-40.0, 10.0)) < 1e-9, "{m:?}");
        // Shift: 39° from the box's horizontal axis steps to 45° from it.
        let (m, text) = t.matrix_in(30.0, c, c, along(10.0, 8.0), true, Rect::ZERO);
        assert!((m * along(10.0, 0.0)).distance(along(0.0, 10.0)) < 1e-9, "{m:?}");
        assert_eq!(text, "-15.0°");
        // A drawn axis is the line on the page.
        let (m, _) = t.matrix_in(30.0, c, c, c + Vec2::new(0.0, -50.0), false, Rect::ZERO);
        assert!((m * Point::new(100.0, 120.0)).distance(Point::new(200.0, 120.0)) < 1e-9, "{m:?}");

        // Anchors selected with Direct Selection scale alone along the page's axes; Alt at the
        // release copies the whole object, which scales along its box.
        let mut s = s.clone();
        s.anchors.insert(id, [(0, 0), (0, 1)].into());
        let mut cx = crate::testutil::cx(&d, &s, &p);
        (cx.smart_guides, cx.snap_to_point) = (false, false);
        let page_axes = |m: Affine| m.as_coeffs()[1].abs() < 1e-9 && m.as_coeffs()[2].abs() < 1e-9;
        let (from, to) = (along(50.0, 0.0), along(100.0, 30.0));
        for copy in [false, true] {
            let mut t = TransformTool::new(TransformKind::Scale);
            t.pointer(&cx, &ev(PointerKind::Down, from.x, from.y));
            let a = t.pointer(&cx, &ev(PointerKind::Drag, to.x, to.y));
            assert!(page_axes(matrix(&a[1])), "{a:?}");
            let up = t.pointer(&cx, &ev(PointerKind::Up, to.x, to.y).with_mods(if copy { alt() } else { Mods::default() }));
            assert_eq!(page_axes(matrix(&up[0])), !copy, "copy {copy}: {up:?}");
        }
        // With the backquote key the drag moves only the whole objects' pattern tiles, which
        // scale along the box, both while dragging and at the release.
        let mut t = TransformTool::new(TransformKind::Scale);
        t.pointer(&cx, &ev(PointerKind::Down, from.x, from.y));
        let a = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, to.x, to.y) });
        assert!(!page_axes(matrix(&a[1])), "{a:?}");
        let up = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Up, to.x, to.y) });
        assert!(!page_axes(matrix(&up[0])), "{up:?}");
    }

    #[test]
    fn shear_horizontal_keeps_origin_row() {
        let t = TransformTool::new(TransformKind::Shear);
        let o = Point::new(150.0, 200.0);
        let (m, _) = t.matrix_for(o, Point::new(150.0, 100.0), Point::new(200.0, 100.0), false, Rect::new(100.0, 100.0, 200.0, 200.0));
        assert!((m * Point::new(100.0, 200.0)).distance(Point::new(100.0, 200.0)) < 1e-9);
        assert!((m * Point::new(150.0, 100.0)).distance(Point::new(200.0, 100.0)) < 1e-9);
    }

    #[test]
    fn shear_shift_vertical_preserves_width() {
        let b = Rect::new(100.0, 100.0, 200.0, 200.0);
        let o = Point::new(150.0, 200.0);
        let t = TransformTool::new(TransformKind::Shear);
        let start = Point::new(100.0, 100.0);
        let p = Point::new(100.0, 190.0);
        let (m, _) = t.matrix_for(o, start, p, true, b);
        assert!((m.transform_rect_bbox(b).width() - b.width()).abs() < 1e-6);
    }

    #[test]
    fn shear_shift_horizontal_preserves_height() {
        let b = Rect::new(100.0, 100.0, 200.0, 200.0);
        let o = Point::new(150.0, 200.0);
        let t = TransformTool::new(TransformKind::Shear);
        let start = Point::new(100.0, 100.0);
        let p = Point::new(190.0, 130.0);
        let (m, _) = t.matrix_for(o, start, p, true, b);
        assert!((m.transform_rect_bbox(b).height() - b.height()).abs() < 1e-6);
    }

    #[test]
    fn nothing_selected_does_nothing() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TransformTool::new(TransformKind::Rotate);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0)).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 150.0)).is_empty());
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::NotAllowed);
    }
}
