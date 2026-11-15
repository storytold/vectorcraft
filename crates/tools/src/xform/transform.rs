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

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::target_overlays;
use crate::bbox::rotate_for_drag;
use crate::guides::{PickSnap, PointSnap, Targets};
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
}

impl Drag {
    /// The angle Scale and Reflect act along, for a drag that copies (`copy`) or not, and that
    /// transforms only pattern tiles (`patterns_only`, the backquote key) or not.
    fn angle_for(&self, copy: bool, patterns_only: bool) -> f64 {
        if self.anchors && !copy && !patterns_only { 0.0 } else { self.angle }
    }
}

pub struct TransformTool {
    kind: TransformKind,
    /// Custom reference point and the selection it was set for.
    origin: Option<(Point, Vec<NodeId>)>,
    drag: Option<Drag>,
    measure: Option<(Point, String)>,
    /// What the Info panel shows for the drag in progress.
    readout: Option<Readout>,
    /// Snap feedback while hovering ("anchor" / "center" label), and for the dragged pointer.
    hover: Vec<Overlay>,
    /// What the dragged pointer snaps to, gathered when the drag begins.
    snap: Option<PointSnap>,
    pick_snap: PickSnap,
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
        Self { kind, origin: None, drag: None, measure: None, readout: None, hover: vec![], snap: None, pick_snap: PickSnap::default() }
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

    /// Keep new Scale contacts on the path its solver can actually place the grabbed point on.
    fn snap_pointer(&self, cx: &ToolContext, d: Drag, ev: &PointerEvent) -> (Point, Vec<Overlay>) {
        let Some(snap) = &self.snap else { return (ev.pos, vec![]) };
        if self.kind != TransformKind::Scale {
            return snap.snap(cx, ev.pos);
        }
        let angle = d.angle_for(ev.mods.alt, ev.backquote);
        snap.snap_scale_pointer(
            cx,
            ev.pos,
            || {
                let direction = d.pick - d.origin;
                if ev.mods.shift {
                    return Some(direction);
                }
                let frame = OrientedBox { angle, rect: Rect::ZERO }.to_doc();
                let local = frame.inverse() * direction.to_point();
                match (local.x.abs() <= 1e-6, local.y.abs() <= 1e-6) {
                    (true, true) => Some(Vec2::ZERO),
                    (true, false) => Some((frame * Point::new(0.0, 1.0)).to_vec2()),
                    (false, true) => Some((frame * Point::new(1.0, 0.0)).to_vec2()),
                    (false, false) => None,
                }
            },
            |p| self.solve(angle, d.origin, d.pick, p, ev.mods.shift, d.bounds).0 * d.pick,
        )
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
                let (pick, _) = self.pick_snap.snap(cx, p);
                // Alt: decided on release. A click sets the point and opens the dialog; a drag
                // transforms a copy about the reference point as it was.
                let origin = self.reference_point(cx).unwrap_or(bounds.center());
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
                    d.began = true;
                    // The selection moves, so only the other objects are targets. The Scale tool's
                    // drag scales; the others move the art about the reference point.
                    let targets = || Targets::collect(cx.doc, &cx.selection.objects, None).with_stroke_edges(cx, &cx.selection.objects, None);
                    self.snap =
                        Some(if self.kind == TransformKind::Scale { PointSnap::scaling(cx, targets) } else { PointSnap::moving(cx, targets) });
                    out.push(Action::Begin(self.kind.label().into()));
                }
                let (p, guides) = self.snap_pointer(cx, d, ev);
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
                    vec![preview(m, ev.mods.alt, ev.backquote), Action::Commit]
                } else {
                    // A click sets the reference point; with Alt, it opens the dialog too.
                    self.origin = Some((d.pick, cx.selection.objects.clone()));
                    if d.alt { vec![self.dialog(d.pick)] } else { vec![] }
                }
            }
            PointerKind::DoubleClick => {
                self.end_drag();
                match self.reference_point(cx) {
                    Some(o) => vec![self.dialog(o)],
                    None => vec![],
                }
            }
            PointerKind::TripleClick => vec![],
            PointerKind::Move => {
                if self.drag.is_none() {
                    self.hover = if cx.selection.is_empty() { vec![] } else { self.pick_snap.snap(cx, p).1 };
                }
                vec![]
            }
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.busy() => {
                self.end_drag();
                vec![Action::Cancel]
            }
            ToolKey::Enter => self.reference_point(cx).map(|o| vec![self.dialog(o)]).unwrap_or_default(),
            _ => vec![],
        }
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
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
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
    /// A free stroke snap must not turn into false feedback after uniform-scale projection.
    #[test]
    fn stroke_review_scale_constraint_contact() {
        use vectorcraft_doc::{Appearance, Node};

        let (mut d, selected) = doc_with_rect();
        let layer = d.layers[0].id;
        let target = d.alloc_id();
        let mut appearance = Appearance::default_art();
        appearance.set_fill(vectorcraft_color::Paint::None);
        appearance.stroke_mut().unwrap().width = 10.0;
        // Endpoints and center stay outside the 4 pt reach of every probe, so legacy
        // point/alignment targets cannot stand in for the visible vertical edge at x=205.
        d.insert(Some(layer), 1, Node::path(target, vectorcraft_geom::shapes::line(Point::new(210.0, 80.0), Point::new(210.0, 340.0)), appearance))
            .unwrap();
        assert_eq!(d.node(target).unwrap().appearance.stroke_width(), 10.0);
        assert!(!d.node(target).unwrap().appearance.stroke_paint().is_none());
        let mut selection = Selection::default();
        selection.add(selected);
        let paint = paint();
        let document_before = d.clone();
        let selection_before = selection.clone();
        let origin = Point::new(0.0, 0.0);
        let grab = Point::new(100.0, 100.0);
        // Legitimate controls first, then the two ON+Shift cases. Collect all real
        // gestures before assertions, so a failing first ON case cannot hide the second.
        let cases = [
            (false, true, Point::new(202.0, 198.0), Point::new(200.0, 200.0), None),
            (true, false, Point::new(202.0, 198.0), Point::new(205.0, 198.0), Some(Point::new(205.0, 198.0))),
            // Uniform pointer is (200,200); contour contact (205,205) is >4 pt away.
            (true, true, Point::new(202.0, 198.0), Point::new(200.0, 200.0), None),
            // Uniform pointer is (203.5,203.5); (205,205) is within 4 pt along its path.
            (true, true, Point::new(205.0, 202.0), Point::new(205.0, 205.0), Some(Point::new(205.0, 205.0))),
        ];
        let mut observed = Vec::new();
        for (on, shift, raw, _, _) in cases {
            let mut c = cx(&d, &selection, &paint);
            c.snap_to_stroke_edges = on;
            c.snapping_tolerance = 4.0;
            c.smart_guides = true;
            c.snap_to_point = false;
            c.snap_to_grid = false;
            c.snap_to_pixel = false;
            c.snap_to_glyph = false;
            c.snap_tangent = false;
            c.snap_perpendicular = false;
            c.guides = false;
            c.alignment_guides = false;
            c.construction_angles = &[];
            c.anchor_path_labels = true;
            c.measurement_labels = false;
            c.transform_tools_guides = false;
            let mut tool = TransformTool::new(TransformKind::Scale);
            assert!(tool.pointer(&c, &ev(PointerKind::Down, origin.x, origin.y)).is_empty());
            assert!(tool.pointer(&c, &ev(PointerKind::Up, origin.x, origin.y)).is_empty());
            assert_eq!(tool.reference_point(&c), Some(origin), "set the real reference point by clicking");
            assert!(tool.pointer(&c, &ev(PointerKind::Down, grab.x, grab.y)).is_empty());
            assert_eq!(tool.drag.unwrap().pick, grab, "the actual pressed selection corner is the scale grab");
            let mods = Mods { shift, ..Mods::default() };
            let drag = tool.pointer(&c, &ev(PointerKind::Drag, raw.x, raw.y).with_mods(mods));
            assert!(
                matches!(drag.as_slice(), [Action::Begin(label), Action::Preview(command, _)] if label == "Scale" && command == "object.transform"),
                "{drag:?}"
            );
            let m = matrix(&drag[1]);
            let labels: Vec<Point> = tool
                .overlays(&c)
                .iter()
                .filter_map(|overlay| match overlay {
                    Overlay::Label { p, text, .. } if text == "stroke edge" => Some(*p),
                    _ => None,
                })
                .collect();
            let up = tool.pointer(&c, &ev(PointerKind::Up, raw.x, raw.y).with_mods(mods));
            assert!(matches!(up.as_slice(), [Action::Preview(_, _), Action::Commit]), "{up:?}");
            assert_eq!(matrix(&up[0]), m, "release retains the actual constrained preview");
            assert!(!tool.busy());
            assert!(!tool.overlays(&c).iter().any(|overlay| matches!(overlay, Overlay::Label { text, .. } if text == "stroke edge")));
            observed.push((m, m * grab, labels));
        }
        assert_eq!(d, document_before, "the tool's preview must not edit the source document");
        assert_eq!(selection, selection_before);
        for ((on, shift, raw, expected, label), (m, actual, labels)) in cases.into_iter().zip(observed) {
            assert!(
                actual.distance(expected) < 1e-9,
                "on={on}, shift={shift}, raw={raw:?}: actual grabbed corner {actual:?}, expected {expected:?}; stroke feedback {labels:?}"
            );
            assert!((m * origin).distance(origin) < 1e-9);
            let coefficients = m.as_coeffs();
            if shift {
                assert!((coefficients[0] - coefficients[3]).abs() < 1e-9, "the real preview must remain uniform: {m:?}");
            }
            match label {
                Some(contact) => {
                    assert_eq!(labels.len(), 1, "on={on}, shift={shift}: {labels:?}");
                    assert!(
                        labels[0].distance(contact) < 1e-9 && labels[0].distance(actual) < 1e-9,
                        "feedback must be at the final solver's actual contact: actual={actual:?}, labels={labels:?}"
                    );
                }
                None => assert!(labels.is_empty(), "no false stroke-edge contact after constraint: {labels:?}"),
            }
        }
    }

    /// Locked Scale axes and the minimum-scale clamp must describe the final grab, not a free snap.
    #[test]
    fn stroke_review_scale_axis_and_clamp_contact() {
        use vectorcraft_doc::{Appearance, Node};

        let cases = [
            (
                "locked axis misses",
                Point::new(100.0, 0.0),
                Point::new(115.0, 80.0),
                Point::new(115.0, 340.0),
                Point::new(107.0, 198.0),
                false,
                Point::new(100.0, 198.0),
                None,
            ),
            (
                "locked axis contacts",
                Point::new(100.0, 0.0),
                Point::new(20.0, 210.0),
                Point::new(400.0, 210.0),
                Point::new(107.0, 202.0),
                false,
                Point::new(100.0, 205.0),
                Some(Point::new(100.0, 205.0)),
            ),
            (
                "clamp rejects unreachable edge",
                Point::ORIGIN,
                Point::new(5.005, -100.0),
                Point::new(5.005, 400.0),
                Point::new(0.001, 0.001),
                true,
                Point::new(0.01, 0.01),
                None,
            ),
            (
                "clamp keeps reachable edge",
                Point::ORIGIN,
                Point::new(5.01, -100.0),
                Point::new(5.01, 400.0),
                Point::new(0.001, 0.001),
                true,
                Point::new(0.01, 0.01),
                Some(Point::new(0.01, 0.01)),
            ),
        ];
        let grab = Point::new(100.0, 100.0);
        let mut observed = Vec::new();
        for (_, origin, from, to, raw, shift, _, _) in cases {
            let (mut d, selected) = doc_with_rect();
            // Keep the ordinary artboard's corners, edges and center away from clamp probes.
            d.artboards[0].rect = Rect::new(-500.0, -300.0, 700.0, 600.0);
            let layer = d.layers[0].id;
            let target = d.alloc_id();
            let mut appearance = Appearance::default_art();
            appearance.set_fill(vectorcraft_color::Paint::None);
            appearance.stroke_mut().unwrap().width = 10.0;
            d.insert(Some(layer), 1, Node::path(target, vectorcraft_geom::shapes::line(from, to), appearance)).unwrap();
            assert_eq!(d.node(target).unwrap().appearance.stroke_width(), 10.0);
            let mut selection = Selection::default();
            selection.add(selected);
            let paint = paint();
            let before = d.clone();
            let mut c = cx(&d, &selection, &paint);
            c.snap_to_stroke_edges = true;
            c.snapping_tolerance = 4.0;
            c.smart_guides = false;
            c.snap_to_point = false;
            c.snap_to_grid = false;
            c.snap_to_pixel = false;
            c.snap_to_glyph = false;
            c.snap_tangent = false;
            c.snap_perpendicular = false;
            c.guides = false;
            c.alignment_guides = false;
            c.construction_angles = &[];
            c.anchor_path_labels = true;
            c.measurement_labels = false;
            c.transform_tools_guides = false;
            let mut tool = TransformTool::new(TransformKind::Scale);
            // A normal pre-gesture Smart Guides toggle permits an exact reference click beside
            // the clamp scene's stroke; every observed drag still has the new option and guides ON.
            assert!(tool.pointer(&c, &ev(PointerKind::Down, origin.x, origin.y)).is_empty());
            assert!(tool.pointer(&c, &ev(PointerKind::Up, origin.x, origin.y)).is_empty());
            c.smart_guides = true;
            assert_eq!(tool.reference_point(&c), Some(origin));
            assert!(tool.pointer(&c, &ev(PointerKind::Down, grab.x, grab.y)).is_empty());
            assert_eq!(tool.drag.unwrap().pick, grab);
            let mods = Mods { shift, ..Mods::default() };
            let drag = tool.pointer(&c, &ev(PointerKind::Drag, raw.x, raw.y).with_mods(mods));
            assert!(
                matches!(drag.as_slice(), [Action::Begin(label), Action::Preview(command, _)] if label == "Scale" && command == "object.transform"),
                "{drag:?}"
            );
            let m = matrix(&drag[1]);
            let labels: Vec<Point> = tool
                .overlays(&c)
                .iter()
                .filter_map(|overlay| match overlay {
                    Overlay::Label { p, text, .. } if text == "stroke edge" => Some(*p),
                    _ => None,
                })
                .collect();
            let up = tool.pointer(&c, &ev(PointerKind::Up, raw.x, raw.y).with_mods(mods));
            assert!(matches!(up.as_slice(), [Action::Preview(_, _), Action::Commit]), "{up:?}");
            assert_eq!(matrix(&up[0]), m);
            assert!(!tool.busy());
            assert_eq!(d, before);
            observed.push((m, m * grab, labels));
        }
        for ((name, origin, _, _, _, shift, expected, label), (m, actual, labels)) in cases.into_iter().zip(observed) {
            assert!(actual.distance(expected) < 1e-9, "{name}: actual={actual:?}, expected={expected:?}, feedback={labels:?}");
            assert!((m * origin).distance(origin) < 1e-9, "{name}");
            let coefficients = m.as_coeffs();
            if shift {
                assert!((coefficients[0] - 1e-4).abs() < 1e-12 && (coefficients[3] - 1e-4).abs() < 1e-12, "{name}: {m:?}");
            } else {
                assert!((coefficients[0] - 1.0).abs() < 1e-12, "the local x axis stays locked: {name}: {m:?}");
            }
            match label {
                Some(contact) => {
                    assert_eq!(labels.len(), 1, "{name}: {labels:?}");
                    assert!(
                        labels[0].distance(contact) < 1e-8 && labels[0].distance(actual) < 1e-8,
                        "{name}: feedback must describe the actual final grabbed point: actual={actual:?}, labels={labels:?}"
                    );
                }
                None => assert!(labels.is_empty(), "{name}: infeasible contact must have no stroke-edge feedback: {labels:?}"),
            }
        }
    }
}
