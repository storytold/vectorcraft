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
//! onto another object's anchor scales exactly onto it.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::target_overlays;
use crate::bbox::rotate_for_drag;
use crate::guides::{PointSnap, Targets, snap_pick};
use crate::select::{matrix_json, selection_bounds, selection_box};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

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
    last: Affine,
}

pub struct TransformTool {
    kind: TransformKind,
    /// Custom reference point and the selection it was set for.
    origin: Option<(Point, Vec<NodeId>)>,
    drag: Option<Drag>,
    measure: Option<(Point, String)>,
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
fn preview(m: Affine, copy: bool) -> Action {
    Action::Preview("object.transform".into(), json!({ "matrix": matrix_json(m), "copy": copy, "anchors": true }))
}

/// Reflection across the line through the origin at angle `theta` (radians, screen space).
pub fn reflect_matrix(o: Point, theta: f64) -> Affine {
    about(o, Affine::rotate(theta) * Affine::scale_non_uniform(1.0, -1.0) * Affine::rotate(-theta))
}

impl TransformTool {
    pub fn new(kind: TransformKind) -> Self {
        Self { kind, origin: None, drag: None, measure: None, hover: vec![], snap: None }
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
        vectorcraft_effects::preview::anchor_bounds(cx.selection, cx.doc, cx.preview_bounds)
            .map(|b| b.center())
            .or_else(|| selection_box(cx).map(|b| b.center()))
    }

    /// Transform for dragging from `start` to `p` about `o`; returns the matrix and a readout.
    pub fn matrix_for(&self, o: Point, start: Point, p: Point, shift: bool, bounds: Rect) -> (Affine, String) {
        match self.kind {
            TransformKind::Rotate => {
                let (a, deg) = rotate_for_drag(o, start, p, shift);
                (a, format!("{:.1}°", -deg))
            }
            TransformKind::Reflect => {
                let mut v = p - o;
                if shift {
                    v = vectorcraft_geom::constrain_angle(v, 45.0);
                }
                let theta = if v.hypot() < 1e-9 { std::f64::consts::FRAC_PI_2 } else { v.y.atan2(v.x) };
                (reflect_matrix(o, theta), format!("{:.1}°", vectorcraft_geom::normalize_deg(-theta.to_degrees())))
            }
            TransformKind::Scale => {
                let d0 = start - o;
                let d1 = p - o;
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
                (about(o, Affine::scale_non_uniform(sx, sy)), format!("W: {:.1}%\nH: {:.1}%", sx * 100.0, sy * 100.0))
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
                (m, format!("{:.1}°", (-k.atan()).to_degrees()))
            }
        }
    }

    /// Ends the drag, its snapping and readouts with it.
    fn end_drag(&mut self) -> Option<Drag> {
        self.measure = None;
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

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                let Some(bounds) = selection_bounds(cx) else { return vec![] };
                self.hover.clear();
                let (pick, _) = snap_pick(cx, p);
                // Alt: decided on release. A click sets the point and opens the dialog; a drag
                // transforms a copy about the reference point as it was.
                let origin = self.reference_point(cx).unwrap_or(bounds.center());
                self.drag = Some(Drag { start: p, pick, origin, began: false, alt: ev.mods.alt, bounds, last: Affine::IDENTITY });
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
                    // The selection moves, so only the other objects are targets.
                    self.snap = Some(PointSnap::new(cx, || Targets::collect(cx.doc, &cx.selection.objects, None)));
                    out.push(Action::Begin(self.kind.label().into()));
                }
                let (p, guides) = self.snap.as_ref().map_or((p, vec![]), |s| s.snap(cx, p));
                self.hover = guides;
                let (m, text) = self.matrix_for(d.origin, d.pick, p, ev.mods.shift, d.bounds);
                d.last = m;
                self.drag = Some(d);
                self.measure = Some((p, text));
                out.push(preview(m, ev.mods.alt));
                out
            }
            PointerKind::Up => {
                let Some(d) = self.end_drag() else { return vec![] };
                if d.began {
                    // The Alt state at release decides whether the result is a copy.
                    vec![preview(d.last, ev.mods.alt), Action::Commit]
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
            PointerKind::Move => {
                if self.drag.is_none() {
                    self.hover = if cx.selection.is_empty() { vec![] } else { snap_pick(cx, p).1 };
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
