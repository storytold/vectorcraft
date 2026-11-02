//! The Objects on Path tool (#2545): a click on a path attaches the selected objects to it
//! (`object.objectsOnPath.attach`). A path they can attach to is highlighted under the pointer,
//! and the pointer shows [`Cursor::ObjectsOnPath`] over it. The tool's options, `pivot` (0-8, row
//! by row from the top left; 4, the center) and `rotate` (degrees), go with each attach, so they
//! set how the objects attached next attach; groups attached before keep theirs.
//!
//! The Rotate All widget: with one Objects on Path group selected, the Selection tool and this
//! tool show a round handle beside the group's first object. Dragging it turns every object of the
//! group about its attachment point by the angle the pointer turns about the first object's
//! attachment point: the angle is added to the group's Rotate (`object.objectsOnPath.options`),
//! previewed while dragging and kept as one undo step on release. Shift turns in steps of 15°, and
//! Escape cancels the drag.
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>.

use serde_json::{Value, json};
use vectorcraft_doc::objects_on_path::{CENTER, attachable_path, is_objects_on_path, stops};
use vectorcraft_doc::{Document, NodeId, Selection};
use vectorcraft_geom::{BezPath, Point, Vec2};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The highlight of the path under the pointer.
const HIGHLIGHT: [u8; 3] = [0x4f, 0x9d, 0xff];

/// The Rotate All widget's center sits this far (screen px) beyond the first object, on the side
/// its top faces.
const ROTATE_ALL_OFFSET_PX: f64 = 16.0;
/// The Rotate All widget's radius (screen px), as the canvas draws it.
pub const ROTATE_ALL_RADIUS_PX: f32 = 4.0;
/// How near (screen px) the pointer must be to hit the Rotate All widget.
const ROTATE_ALL_HIT_PX: f64 = 6.0;
/// The step (degrees) a Shift-drag of the Rotate All widget turns by.
const ROTATE_ALL_STEP: f64 = 15.0;

/// The Rotate All widget of a selected Objects on Path group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotateAllWidget {
    pub group: NodeId,
    /// The first object's attachment point, in the document: a drag turns about it.
    pub pivot: Point,
    /// The widget's center in the document.
    pub at: Point,
    /// The group's Rotate (degrees).
    pub rotate: f64,
}

impl RotateAllWidget {
    /// The widget of the selection at `zoom` (screen px per document point): only when the
    /// selection is one editable Objects on Path group with at least one object on a path with
    /// length, and no anchors are Direct-Selected.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        if !selection.anchors.is_empty() || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let [group] = selection.objects.as_slice() else { return None };
        if !doc.editable()(*group) {
            return None;
        }
        let n = doc.node(*group)?;
        let rec = n.extras().objects_on_path.as_deref()?;
        let children = n.children()?;
        let path = attachable_path(children.iter().find(|c| c.id == rec.path)?)?;
        let first = rec.members.first()?;
        // The first object attaches at the path's start, whatever the number of objects.
        let (pivot, _) = *stops(path, 1)?.first()?;
        let art = children.iter().find(|c| c.id == first.id)?;
        let b = art.geometric_bounds_with(&doc.symbol_boxes())?;
        // The object's top is the -y direction of the frame it is drawn in (the document's y axis
        // points down), so a reflected or sheared frame moves the widget with the art's top.
        let [_, _, a2, a3, _, _] = first.frame.as_coeffs();
        let len = a2.hypot(a3);
        let up = if len.is_finite() && len > 1e-12 {
            Vec2::new(-a2 / len, -a3 / len)
        } else {
            let (sin, cos) = first.angle.sin_cos();
            Vec2::new(sin, -cos)
        };
        let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x0, b.y1), Point::new(b.x1, b.y1)];
        let reach = corners.iter().map(|c| (*c - pivot).dot(up)).fold(0.0, f64::max);
        let at = pivot + up * (reach + ROTATE_ALL_OFFSET_PX / zoom);
        let finite = at.x.is_finite() && at.y.is_finite() && pivot.x.is_finite() && pivot.y.is_finite();
        finite.then_some(Self { group: *group, pivot, at, rotate: rec.rotate })
    }

    /// The widget of `cx` when `p` is within reach of it.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        Self::of(cx.doc, cx.selection, cx.zoom).filter(|w| w.at.distance(p) <= cx.tol(ROTATE_ALL_HIT_PX))
    }
}

/// Is `p` over the Rotate All widget?
pub fn over_rotate_all(cx: &ToolContext, p: Point) -> bool {
    RotateAllWidget::at_point(cx, p).is_some()
}

/// Dragging the Rotate All widget: the group's Rotate follows the angle the pointer turns about
/// the first object's attachment point, counter-clockwise on the page, over as many turns as the
/// drag makes.
#[derive(Clone, Debug)]
pub struct RotateAllDrag {
    widget: RotateAllWidget,
    start: Point,
    began: bool,
    /// The pointer's direction from the pivot at the last drag (radians, the document's axes).
    last: f64,
    /// The degrees the pointer has turned counter-clockwise on the page since the press.
    turned: f64,
    /// The Rotate the drag sets.
    rotate: f64,
    at: Point,
}

impl RotateAllDrag {
    /// The drag has left the click: the angle is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on the Rotate All widget.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = RotateAllWidget::at_point(cx, ev.pos)?;
        let d = ev.pos - widget.pivot;
        let last = d.y.atan2(d.x);
        let last = if last.is_finite() { last } else { 0.0 };
        Some(Self { widget, start: ev.pos, began: false, last, turned: 0.0, rotate: widget.rotate, at: ev.pos })
    }

    /// The pointer moved to `p`; `shift` turns in steps of 15°.
    pub fn drag(&mut self, cx: &ToolContext, p: Point, shift: bool) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Objects on Path Options".into()));
        }
        let d = p - self.widget.pivot;
        if d.hypot() > 1e-9 {
            let now = d.y.atan2(d.x);
            let step = (now - self.last + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
            if step.is_finite() {
                self.turned -= step.to_degrees();
                self.last = now;
            }
        }
        let turned = if shift { (self.turned / ROTATE_ALL_STEP).round() * ROTATE_ALL_STEP } else { self.turned };
        let r = self.widget.rotate + turned;
        self.rotate = if r.abs() > 360.0 { r % 360.0 } else { r } + 0.0;
        self.at = p;
        out.push(Action::Preview("object.objectsOnPath.options".into(), json!({ "ids": [self.widget.group.0], "rotate": self.rotate })));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The Rotate readout next to the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: format!("{:.1}°", self.rotate) }]
    }
}

/// The path a click at `p` attaches the selected objects to: the path object hit there, when it can
/// take objects, is in no Objects on Path group, is not a clipping path or part of a compound path,
/// blend, envelope, repeat or Intertwine group ([`vectorcraft_doc::objects_on_path::held`]), and
/// something else is selected.
pub fn target(cx: &ToolContext, p: Point) -> Option<(NodeId, BezPath)> {
    let h = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options())?;
    let n = cx.doc.node(h.leaf)?;
    let path = attachable_path(n)?;
    if h.ancestry.iter().filter(|a| **a != h.leaf).any(|a| cx.doc.node(*a).is_some_and(is_objects_on_path)) {
        return None;
    }
    if vectorcraft_doc::objects_on_path::held(cx.doc, h.leaf).is_some() {
        return None;
    }
    cx.selection.objects.iter().any(|id| *id != h.leaf).then(|| (h.leaf, path.to_bezpath()))
}

pub struct ObjectsOnPathTool {
    /// The path under the pointer the selection can attach to, and its outline.
    hover: Option<(NodeId, BezPath)>,
    /// Where the press was, while the button is down.
    press: Option<Point>,
    /// The Rotate All widget drag, while the button is down after a press on the widget.
    rotating: Option<RotateAllDrag>,
    /// The reference point of each object attached next (0-8, row by row from the top left).
    pivot: u8,
    /// The angle of each object attached next, in degrees counter-clockwise from upright on the path.
    rotate: f64,
}

impl Default for ObjectsOnPathTool {
    fn default() -> Self {
        Self { hover: None, press: None, rotating: None, pivot: CENTER, rotate: 0.0 }
    }
}

impl Tool for ObjectsOnPathTool {
    fn id(&self) -> &'static str {
        "objectsOnPath"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        if let Some(d) = &mut self.rotating {
            return match ev.kind {
                PointerKind::Drag => d.drag(cx, p, ev.mods.shift),
                PointerKind::Up => self.rotating.take().map(RotateAllDrag::finish).unwrap_or_default(),
                _ => vec![],
            };
        }
        match ev.kind {
            PointerKind::Move => {
                self.hover = if over_rotate_all(cx, p) { None } else { target(cx, p) };
                vec![]
            }
            PointerKind::Down => {
                if let Some(d) = RotateAllDrag::hit(cx, ev) {
                    self.rotating = Some(d);
                    self.hover = None;
                    return vec![];
                }
                self.press = Some(p);
                vec![]
            }
            PointerKind::Up => {
                // A press that moved away is not a click.
                let Some(start) = self.press.take().filter(|q| q.distance(p) < cx.tol(3.0)) else { return vec![] };
                let Some((path, _)) = target(cx, start) else { return vec![] };
                self.hover = None;
                vec![Action::Exec("object.objectsOnPath.attach".into(), json!({ "path": path.0, "pivot": self.pivot, "rotate": self.rotate }))]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o: Vec<Overlay> =
            self.hover.iter().map(|(_, path)| Overlay::Path { path: path.clone(), color: HIGHLIGHT, width: 2.0, dashed: false }).collect();
        if let Some(d) = &self.rotating {
            o.extend(d.overlays(cx));
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.rotating.is_some() || over_rotate_all(cx, p) {
            Cursor::Rotate
        } else if target(cx, p).is_some() {
            Cursor::ObjectsOnPath
        } else {
            Cursor::Arrow
        }
    }

    /// Escape while the button is down cancels the click, so the release attaches nothing, or the
    /// Rotate All drag, so the objects go back to the Rotate they had.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.press = None;
            self.hover = None;
            if self.rotating.take().is_some_and(|d| d.adjusting()) {
                return vec![Action::Cancel];
            }
        }
        vec![]
    }

    fn options(&self) -> Value {
        json!({ "pivot": self.pivot, "rotate": self.rotate })
    }

    /// `pivot` takes a whole number from 0 to 8 and `rotate` a finite number, clamped to -360 to
    /// 360; other values are ignored.
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "pivot" => {
                if let Some(n) = value.as_u64().filter(|n| *n <= 8) {
                    self.pivot = n as u8;
                }
            }
            "rotate" => {
                if let Some(r) = value.as_f64().filter(|r| r.is_finite()) {
                    self.rotate = r.clamp(-360.0, 360.0);
                }
            }
            _ => {}
        }
    }

    fn busy(&self) -> bool {
        self.press.is_some() || self.rotating.is_some()
    }

    fn transforming(&self) -> bool {
        self.rotating.as_ref().is_some_and(RotateAllDrag::adjusting)
    }
}

pub fn create(id: &str) -> Option<Box<dyn crate::Tool>> {
    (id == "objectsOnPath").then(|| Box::new(ObjectsOnPathTool::default()) as Box<dyn crate::Tool>)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_doc::{Appearance, Node, NodeKind};
    use vectorcraft_geom::{Rect, shapes};

    use super::*;
    use crate::testutil::{cx, paint};

    fn doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let line = d.alloc_id();
        let square = d.alloc_id();
        d.insert(Some(l), 0, Node::path(line, shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), Appearance::default_art())).unwrap();
        d.insert(Some(l), 1, Node::path(square, shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0)), Appearance::default_art())).unwrap();
        (d, line, square)
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn a_click_on_a_path_attaches_the_selection_to_it() {
        let (d, line, square) = doc();
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        let on = Point::new(200.0, 300.0);
        assert_eq!(t.cursor(&cx, on, Mods::default()), Cursor::ObjectsOnPath);
        assert_eq!(t.cursor(&cx, Point::new(200.0, 500.0), Mods::default()), Cursor::Arrow, "away from paths");
        t.pointer(&cx, &ev(PointerKind::Move, on.x, on.y));
        assert_eq!(t.overlays(&cx).len(), 1, "the path is highlighted");
        t.pointer(&cx, &ev(PointerKind::Down, on.x, on.y));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, on.x, on.y));
        assert!(matches!(acts.as_slice(), [Action::Exec(id, p)] if id == "object.objectsOnPath.attach" && p["path"] == line.0), "{acts:?}");
    }

    #[test]
    fn the_click_sends_the_tool_options_pivot_and_rotate() {
        let (d, _, square) = doc();
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        assert_eq!(t.options(), json!({ "pivot": 4, "rotate": 0.0 }), "the center, upright");
        t.set_option("pivot", &json!(0));
        t.set_option("rotate", &json!(45.5));
        // Values out of range are ignored.
        t.set_option("pivot", &json!(9));
        t.set_option("rotate", &json!("x"));
        assert_eq!(t.options(), json!({ "pivot": 0, "rotate": 45.5 }));
        // Rotate is kept within the dialog's -360 to 360.
        t.set_option("rotate", &json!(450));
        assert_eq!(t.options()["rotate"], 360.0);
        t.set_option("rotate", &json!(-720.5));
        assert_eq!(t.options()["rotate"], -360.0);
        t.set_option("rotate", &json!(45.5));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0));
        assert!(matches!(acts.as_slice(), [Action::Exec(_, p)] if p["pivot"] == 0 && p["rotate"] == 45.5), "{acts:?}");
    }

    #[test]
    fn escape_during_the_press_cancels_the_click() {
        let (d, _, square) = doc();
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        t.pointer(&cx, &ev(PointerKind::Move, 200.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0));
        assert!(t.busy());
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(!t.busy() && t.overlays(&cx).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0)).is_empty(), "the release attaches nothing");
    }

    #[test]
    fn nothing_attaches_without_other_objects_selected() {
        let (d, line, _) = doc();
        let mut sel = Selection::default();
        sel.set([line]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        assert_eq!(t.cursor(&cx, Point::new(200.0, 300.0), Mods::default()), Cursor::Arrow);
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0)).is_empty());
        assert!(matches!(d.node(line).map(|n| &n.kind), Some(NodeKind::Path { .. })));
    }

    /// A document holding an Objects on Path group (the line from (0, 300) to (400, 300), with two
    /// 20 × 20 squares attached by their centers, the first on (0, 300)) whose Rotate is `rotate`,
    /// and a selection of the group.
    fn group_doc(rotate: f64) -> (Document, NodeId, Selection) {
        group_doc_framed(rotate, None)
    }

    /// [`group_doc`] with every object drawn in `frame` (mapping its square onto itself) before it
    /// is placed on the path.
    fn group_doc_framed(rotate: f64, frame: Option<vectorcraft_geom::Affine>) -> (Document, NodeId, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (line, a, b, g) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
        let line = Node::path(line, shapes::line(Point::new(0.0, 300.0), Point::new(400.0, 300.0)), Appearance::default_art());
        let a = Node::path(a, shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0)), Appearance::default_art());
        let b = Node::path(b, shapes::rectangle(Rect::new(100.0, 50.0, 120.0, 70.0)), Appearance::default_art());
        let mut rec = vectorcraft_doc::objects_on_path::ObjectsOnPath::new(line.id, &[&a, &b], CENTER, rotate);
        if let Some(f) = frame {
            for m in &mut rec.members {
                m.frame = f;
            }
        }
        let mut group = Node::new(g, NodeKind::Group { children: vec![Arc::new(line), Arc::new(a), Arc::new(b)], clip: false });
        group.extras_mut().objects_on_path = Some(Box::new(rec));
        vectorcraft_doc::objects_on_path::place(&mut group, &Document::new(1.0, 1.0).symbol_boxes());
        d.insert(Some(l), 0, group).unwrap();
        let mut sel = Selection::default();
        sel.set([g]);
        (d, g, sel)
    }

    /// The pointer `deg` degrees counter-clockwise on the page from straight above the first
    /// object's attachment point (0, 300), 26 points from it.
    fn around(deg: f64) -> Point {
        let (sin, cos) = deg.to_radians().sin_cos();
        Point::new(-26.0 * sin, 300.0 - 26.0 * cos)
    }

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    /// The Rotate a drag's last action previews.
    fn previewed(acts: &[Action]) -> Option<f64> {
        match acts.last() {
            Some(Action::Preview(cmd, p)) if cmd == "object.objectsOnPath.options" => p["rotate"].as_f64(),
            _ => None,
        }
    }

    #[test]
    fn the_rotate_all_widget_sits_above_the_first_object() {
        let (d, g, sel) = group_doc(0.0);
        let w = RotateAllWidget::of(&d, &sel, 1.0).unwrap();
        // The first square spans 290 to 310 down the page: the widget is 16 px above its top.
        assert_eq!(w.group, g);
        assert!(near(w.pivot, Point::new(0.0, 300.0)) && near(w.at, Point::new(0.0, 274.0)), "{w:?}");
        assert!(near(RotateAllWidget::of(&d, &sel, 2.0).unwrap().at, Point::new(0.0, 282.0)));
        // Turned by Rotate, the widget goes round with the object's top.
        let (d, _, sel) = group_doc(90.0);
        let w = RotateAllWidget::of(&d, &sel, 1.0).unwrap();
        assert!(near(w.at, Point::new(-26.0, 300.0)), "{w:?}");
        // Drawn upside down (reflected across the line through their centers), the objects have
        // their tops facing down the page, and the widget goes below the first one.
        let flip = vectorcraft_geom::Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 120.0]);
        let (df, _, sf) = group_doc_framed(0.0, Some(flip));
        let wf = RotateAllWidget::of(&df, &sf, 1.0).unwrap();
        assert!(near(wf.pivot, Point::new(0.0, 300.0)) && near(wf.at, Point::new(0.0, 326.0)), "{wf:?}");
        // No widget for nothing selected, for an object in the group, or for two objects.
        assert!(RotateAllWidget::of(&d, &Selection::default(), 1.0).is_none());
        let children: Vec<NodeId> = d.node(sel.objects[0]).and_then(|n| n.children()).unwrap().iter().map(|c| c.id).collect();
        let mut one = Selection::default();
        one.set([children[1]]);
        assert!(RotateAllWidget::of(&d, &one, 1.0).is_none());
        let (d2, line, square) = doc();
        let mut two = Selection::default();
        two.set([line, square]);
        assert!(RotateAllWidget::of(&d2, &two, 1.0).is_none());
        // Nor for a group that is locked.
        let (mut d3, g3, sel3) = group_doc(0.0);
        if let Some(n) = d3.node_mut(g3) {
            n.locked = true;
        }
        assert!(RotateAllWidget::of(&d3, &sel3, 1.0).is_none());
    }

    /// Dragging the widget a quarter turn counter-clockwise adds 90° to the group's Rotate, with
    /// the Selection tool and with the Objects on Path tool, previewed while dragging and kept as
    /// one undo step on release.
    #[test]
    fn dragging_the_rotate_all_widget_adds_the_angle_to_rotate() {
        let (d, g, sel) = group_doc(30.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let at = RotateAllWidget::of(&d, &sel, 1.0).unwrap().at;
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::Rotate, "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty(), "{}", t.id());
            // The press is 30° round from straight above the attachment point, as Rotate turns the
            // first object; the pointer goes on to 120°.
            let q = around(120.0);
            let a = t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y));
            assert_eq!(a.first(), Some(&Action::Begin("Objects on Path Options".into())), "{}", t.id());
            assert!(matches!(a.last(), Some(Action::Preview(_, p)) if p["ids"] == json!([g.0])), "{a:?}");
            assert!((previewed(&a).unwrap() - 120.0).abs() < 1e-9, "{a:?}");
            assert!(t.transforming() && t.cursor(&c, q, Mods::default()) == Cursor::Rotate);
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "120.0°")), "{}", t.id());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, q.x, q.y)), vec![Action::Commit], "{}", t.id());
            assert!(!t.busy());
            // A click on the widget that does not move changes nothing.
            assert!(t.pointer(&c, &ev(PointerKind::Down, at.x, at.y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{}", t.id());
        }
    }

    /// Shift turns in 15° steps, and a drag that goes on round the attachment point keeps adding
    /// to the angle past half a turn.
    #[test]
    fn shift_turns_the_objects_in_steps_of_fifteen_degrees() {
        let (d, _, sel) = group_doc(0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        let at = around(0.0);
        t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
        let q = around(20.0);
        let shift = PointerEvent { mods: Mods { shift: true, ..Mods::default() }, ..ev(PointerKind::Drag, q.x, q.y) };
        assert!((previewed(&t.pointer(&c, &shift)).unwrap() - 15.0).abs() < 1e-9);
        let q = around(-38.0);
        let shift = PointerEvent { mods: Mods { shift: true, ..Mods::default() }, ..ev(PointerKind::Drag, q.x, q.y) };
        assert!((previewed(&t.pointer(&c, &shift)).unwrap() + 45.0).abs() < 1e-9);
        let q = around(20.0);
        assert!((previewed(&t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y))).unwrap() - 20.0).abs() < 1e-9, "without Shift");
        let mut last = 0.0;
        for deg in [100.0, 180.0, 260.0] {
            let q = around(deg);
            last = previewed(&t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y))).unwrap();
        }
        assert!((last - 260.0).abs() < 1e-9, "{last}");
    }

    /// Escape during a Rotate All drag cancels it: the readout is removed, and the release keeps
    /// nothing.
    #[test]
    fn escape_cancels_a_rotate_all_drag() {
        let (d, _, sel) = group_doc(0.0);
        let paint = paint();
        let c = cx(&d, &sel, &paint);
        let tools: [Box<dyn Tool>; 2] = [Box::new(crate::select::SelectionTool::default()), Box::new(ObjectsOnPathTool::default())];
        for mut t in tools {
            let at = around(0.0);
            t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
            let q = around(60.0);
            assert!(previewed(&t.pointer(&c, &ev(PointerKind::Drag, q.x, q.y))).is_some());
            assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{}", t.id());
            assert!(!t.busy() && !t.transforming(), "{}", t.id());
            assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { .. })), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, at.x, at.y)).is_empty(), "{}", t.id());
            assert!(t.pointer(&c, &ev(PointerKind::Up, at.x, at.y)).is_empty(), "{}: the release keeps nothing", t.id());
        }
    }

    #[test]
    fn a_clipping_path_or_a_blend_key_is_not_a_target() {
        let (mut d, line, square) = doc();
        let l = d.layers[0].id;
        let key = d.alloc_id();
        d.insert(Some(l), 2, Node::path(key, shapes::line(Point::new(0.0, 100.0), Point::new(400.0, 100.0)), Appearance::default_art())).unwrap();
        let blend = d.alloc_id();
        d.wrap(&[line, key], |children| Node::new(blend, NodeKind::Blend { children, spec: Default::default() })).unwrap();
        let mask = d.alloc_id();
        let art = d.alloc_id();
        d.insert(Some(l), 2, Node::path(mask, shapes::rectangle(Rect::new(100.0, 380.0, 300.0, 430.0)), Appearance::default_art())).unwrap();
        d.insert(Some(l), 3, Node::path(art, shapes::rectangle(Rect::new(250.0, 390.0, 290.0, 420.0)), Appearance::default_art())).unwrap();
        let clip = d.alloc_id();
        d.wrap(&[mask, art], |children| {
            let mut g = Node::group(clip, children);
            g.set_clips(true);
            g
        })
        .unwrap();
        assert_eq!(d.node(clip).and_then(|g| g.children()).and_then(|c| c.first()).map(|c| c.id), Some(mask));
        let mut sel = Selection::default();
        sel.set([square]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = ObjectsOnPathTool::default();
        for (p, what) in [((150.0, 400.0), "a clipping path"), ((200.0, 300.0), "a blend's key path"), ((200.0, 100.0), "a blend's other key path")] {
            assert_eq!(t.cursor(&cx, Point::new(p.0, p.1), Mods::default()), Cursor::Arrow, "{what}");
            t.pointer(&cx, &ev(PointerKind::Move, p.0, p.1));
            assert!(t.overlays(&cx).is_empty(), "{what} is not highlighted");
            t.pointer(&cx, &ev(PointerKind::Down, p.0, p.1));
            assert!(t.pointer(&cx, &ev(PointerKind::Up, p.0, p.1)).is_empty(), "a click on {what} attaches nothing");
        }
    }
}
