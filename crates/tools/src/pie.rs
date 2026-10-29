//! Pie widgets: two handles just outside a selected live ellipse, at its Pie Start Angle and Pie
//! End Angle. Where the two coincide, as on a whole ellipse (0° to 360°), a press takes the End
//! widget. The Selection tool shows them while the bounding box shows. Dragging one sets its angle
//! (`object.setLiveShape`'s `pieStart` or `pieEnd`) to the angle of the outline's point that lies
//! in the direction of the pointer from the ellipse's center, in one undo step; double-clicking
//! either makes the shape a whole ellipse again. The Selection tool hits the widgets and the canvas
//! draws them from the same geometry.

use serde_json::json;
use vectorcraft_doc::{Document, LiveShape, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Affine, Point};

use crate::{Action, Overlay, PointerEvent, ToolContext};

/// A widget's center sits this far (screen px) outside the outline, on the line from the
/// ellipse's center through the outline's point at its angle.
const OFFSET_PX: f64 = 12.0;
/// The widget's radius (screen px), as the canvas draws it.
pub const RADIUS_PX: f32 = 3.5;
/// How near (screen px) the pointer must be to hit a widget.
const HIT_PX: f64 = 5.0;

/// Which of a pie's two angles a widget sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieEnd {
    Start,
    End,
}

impl PieEnd {
    /// The `object.setLiveShape` param that sets this angle.
    fn key(self) -> &'static str {
        match self {
            PieEnd::Start => "pieStart",
            PieEnd::End => "pieEnd",
        }
    }
}

/// One pie widget of a selected live ellipse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieWidget {
    pub id: NodeId,
    pub end: PieEnd,
    /// The outline's point at the widget's angle, in the document.
    pub on: Point,
    /// The widget's center in the document.
    pub at: Point,
}

/// A live ellipse: `w` × `h` with its top-left at the origin, mapped into the document by `xf`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ellipse {
    w: f64,
    h: f64,
    xf: Affine,
}

impl Ellipse {
    /// Object `id`'s live ellipse and its pie angles, unless it has no area or `xf` can't be
    /// inverted.
    fn of(doc: &Document, id: NodeId) -> Option<(Self, (f64, f64))> {
        let NodeKind::Path { live, .. } = &doc.node(id)?.kind else { return None };
        let Some(LiveShape::Ellipse { w, h, pie, xf }) = live.as_deref() else { return None };
        let finite = w.is_finite() && h.is_finite() && xf.as_coeffs().iter().all(|c| c.is_finite());
        let usable = finite && w.abs() > 1e-9 && h.abs() > 1e-9 && xf.determinant().abs() > 1e-12;
        usable.then_some((Self { w: *w, h: *h, xf: *xf }, *pie))
    }

    fn radii(&self) -> (f64, f64) {
        (self.w.abs() / 2.0, self.h.abs() / 2.0)
    }

    fn center(&self) -> Point {
        self.xf * Point::new(self.w / 2.0, self.h / 2.0)
    }

    /// The outline's point at pie angle `deg` (degrees counterclockwise from the right, as
    /// `shapes::ellipse_pie` measures them), in the document. 360° gives the very point 0° does.
    fn point_at(&self, deg: f64) -> Point {
        let ((rx, ry), t) = (self.radii(), deg.rem_euclid(360.0).to_radians());
        self.xf * Point::new(self.w / 2.0 + rx * t.cos(), self.h / 2.0 - ry * t.sin())
    }

    /// The pie angle (0 to 360 degrees) whose outline point lies on the ray from the center
    /// toward `p` (0 when `p` is the center).
    fn angle_toward(&self, p: Point) -> f64 {
        let ((rx, ry), q) = (self.radii(), self.xf.inverse() * p);
        let a = (-(q.y - self.h / 2.0) / ry).atan2((q.x - self.w / 2.0) / rx).to_degrees();
        if a.is_finite() { a.rem_euclid(360.0) + 0.0 } else { 0.0 }
    }
}

impl PieWidget {
    /// The widgets of the selected live ellipses at `zoom` (screen px per document point), each
    /// ellipse's End widget before its Start widget. Empty while anchors are Direct-Selected.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Vec<Self> {
        if !selection.anchors.is_empty() || !zoom.is_finite() || zoom <= 0.0 {
            return vec![];
        }
        let editable = doc.editable();
        let shown = |id: &&NodeId| editable(**id) && !selection.segments.contains_key(*id);
        let mut out = vec![];
        for id in selection.objects.iter().filter(shown) {
            let Some((e, (start, end))) = Ellipse::of(doc, *id) else { continue };
            let c = e.center();
            for (which, deg) in [(PieEnd::End, end), (PieEnd::Start, start)] {
                let on = e.point_at(deg);
                let len = (on - c).hypot();
                if len > 1e-9 {
                    out.push(Self { id: *id, end: which, on, at: on + (on - c) * (OFFSET_PX / zoom / len) });
                }
            }
        }
        out
    }

    /// The widgets the Selection tool can drag (its bounding box showing).
    pub fn for_tool(cx: &ToolContext) -> Vec<Self> {
        if !cx.show_bbox {
            return vec![];
        }
        Self::of(cx.doc, cx.selection, cx.zoom)
    }

    /// The widget of `cx` nearest `p` within reach; of two at one spot, the first listed.
    fn at_point(cx: &ToolContext, p: Point) -> Option<Self> {
        let d = |w: &Self| w.at.distance(p);
        Self::for_tool(cx).into_iter().filter(|w| d(w) <= cx.tol(HIT_PX)).min_by(|a, b| d(a).total_cmp(&d(b)))
    }
}

/// Is `p` over a pie widget?
pub fn over_widget(cx: &ToolContext, p: Point) -> bool {
    PieWidget::at_point(cx, p).is_some()
}

/// A double-click on a pie widget makes its ellipse whole: 0° to 360°.
pub fn double_click(cx: &ToolContext, p: Point) -> Option<Action> {
    let w = PieWidget::at_point(cx, p)?;
    Some(Action::Exec("object.setLiveShape".into(), json!({ "id": w.id.0, "pieStart": 0, "pieEnd": 360 })))
}

/// Dragging a pie widget: its angle follows the outline's point that lies in the direction of the
/// pointer from the ellipse's center.
#[derive(Clone, Debug)]
pub struct PieDrag {
    widget: PieWidget,
    ellipse: Ellipse,
    start: Point,
    began: bool,
    angle: f64,
    at: Point,
}

impl PieDrag {
    /// The drag has left the click: the angle is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// Start a drag when the press `ev` is on a pie widget of the selection.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let widget = PieWidget::at_point(cx, ev.pos)?;
        let (ellipse, _) = Ellipse::of(cx.doc, widget.id)?;
        Some(Self { widget, ellipse, start: ev.pos, began: false, angle: 0.0, at: ev.pos })
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Live Shape".into()));
        }
        self.angle = self.ellipse.angle_toward(p);
        self.at = p;
        let mut params = json!({ "id": self.widget.id.0 });
        params[self.widget.end.key()] = json!(self.angle);
        out.push(Action::Preview("object.setLiveShape".into(), params));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The angle readout next to the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: format!("{:.1}°", self.angle) }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::SelectionTool;
    use crate::testutil::*;
    use crate::{Cursor, Mods, PointerKind, Tool};
    use vectorcraft_doc::{Appearance, Node};

    /// A document holding a live 200 × 100 ellipse at (100, 100) with pie angles `pie`, and a
    /// selection of it.
    fn doc_with_ellipse(pie: (f64, f64)) -> (Document, NodeId, Selection) {
        ellipse_doc(200.0, 100.0, pie, Affine::translate((100.0, 100.0)))
    }

    /// A document holding a live `w` × `h` ellipse placed by `xf` with pie angles `pie`, and a
    /// selection of it.
    fn ellipse_doc(w: f64, h: f64, pie: (f64, f64), xf: Affine) -> (Document, NodeId, Selection) {
        let live = LiveShape::Ellipse { w, h, pie, xf };
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut n = Node::path(id, live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(live));
        }
        d.insert(Some(l), 0, n).unwrap();
        let mut s = Selection::default();
        s.add(id);
        (d, id, s)
    }

    fn ev(kind: PointerKind, p: Point) -> PointerEvent {
        PointerEvent::new(kind, p.x, p.y)
    }

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn the_widgets_sit_outside_the_outline_at_the_pie_angles() {
        // A whole ellipse: both at 0°, 12 px right of the outline at any zoom.
        let (d, id, s) = doc_with_ellipse((0.0, 360.0));
        let w = PieWidget::of(&d, &s, 1.0);
        assert_eq!(w.iter().map(|w| w.end).collect::<Vec<_>>(), [PieEnd::End, PieEnd::Start]);
        assert!(w.iter().all(|w| w.id == id && near(w.on, Point::new(300.0, 150.0)) && near(w.at, Point::new(312.0, 150.0))), "{w:?}");
        assert!(near(PieWidget::of(&d, &s, 4.0)[0].at, Point::new(303.0, 150.0)));
        // A quarter: the End widget above the center, the Start widget right of it.
        let (d, _, s) = doc_with_ellipse((0.0, 90.0));
        let w = PieWidget::of(&d, &s, 1.0);
        assert!(near(w[0].on, Point::new(200.0, 100.0)) && near(w[0].at, Point::new(200.0, 88.0)), "{w:?}");
        assert!(near(w[1].at, Point::new(312.0, 150.0)), "{w:?}");
        // No widgets with the bounding box hidden, with nothing selected, or on another shape.
        let p = paint();
        assert!(PieWidget::for_tool(&ToolContext { show_bbox: false, ..cx(&d, &s, &p) }).is_empty());
        assert!(PieWidget::of(&d, &Selection::default(), 1.0).is_empty());
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        assert!(PieWidget::of(&d, &s, 1.0).is_empty());
    }

    /// Dragging the End widget of a whole ellipse from 0° to straight up sets Pie End Angle to 90
    /// in one step, with the angle shown next to the pointer.
    #[test]
    fn dragging_the_end_widget_sets_the_end_angle() {
        let (d, id, s) = doc_with_ellipse((0.0, 360.0));
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert_eq!(t.cursor(&c, Point::new(312.0, 150.0), Mods::default()), Cursor::ArrowHollow);
        assert!(t.pointer(&c, &ev(PointerKind::Down, Point::new(312.0, 150.0))).is_empty());
        let a = t.pointer(&c, &ev(PointerKind::Drag, Point::new(200.0, 20.0)));
        assert_eq!(a[0], Action::Begin("Live Shape".into()));
        let Action::Preview(cmd, v) = &a[1] else { panic!("{a:?}") };
        assert_eq!((cmd.as_str(), &v["id"]), ("object.setLiveShape", &json!(id.0)));
        assert!((v["pieEnd"].as_f64().unwrap() - 90.0).abs() < 1e-9, "{v}");
        assert!(v.get("pieStart").is_none(), "{v}");
        assert!(t.transforming());
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "90.0°")));
        let quiet = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        assert!(!t.overlays(&quiet).iter().any(|o| matches!(o, Overlay::Measure { .. })));
        // Below the center and to the left: 225°, on the ellipse's own angles (not the page's).
        let a = t.pointer(&c, &ev(PointerKind::Drag, Point::new(100.0, 200.0)));
        assert!(matches!(&a[0], Action::Preview(_, v) if (v["pieEnd"].as_f64().unwrap() - 225.0).abs() < 1e-9), "{a:?}");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, Point::new(100.0, 200.0))), vec![Action::Commit]);
        // A click that doesn't move sends no action.
        assert!(t.pointer(&c, &ev(PointerKind::Down, Point::new(312.0, 150.0))).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, Point::new(312.0, 150.0))).is_empty());
    }

    /// Escape during a pie widget drag with the Selection tool cancels it: the angle readout is removed,
    /// and the release after it commits nothing.
    #[test]
    fn escape_cancels_a_pie_widget_drag() {
        let (d, _, s) = doc_with_ellipse((0.0, 360.0));
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, Point::new(312.0, 150.0))).is_empty());
        let a = t.pointer(&c, &ev(PointerKind::Drag, Point::new(200.0, 20.0)));
        assert_eq!(a[0], Action::Begin("Live Shape".into()));
        assert_eq!(t.key(&c, crate::ToolKey::Escape, Mods::default()), vec![Action::Cancel], "Escape cancels the drag");
        assert!(!t.busy() && !t.transforming());
        assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { .. })), "the readout is removed");
        assert!(t.pointer(&c, &ev(PointerKind::Drag, Point::new(100.0, 200.0))).is_empty(), "the drag is over");
        assert!(t.pointer(&c, &ev(PointerKind::Up, Point::new(100.0, 200.0))).is_empty(), "the release commits nothing");
    }

    /// On a whole ellipse the two widgets sit at the very same point, whatever its size, place or
    /// angle, so a press there always takes the End widget.
    #[test]
    fn a_whole_ellipse_gives_a_press_its_end_widget() {
        let p = paint();
        for (w, h, xf) in [
            (100.0, 100.0, Affine::IDENTITY),
            (37.5, 81.25, Affine::translate((13.0, 7.0))),
            (120.0, 60.0, Affine::translate((200.0, 150.0)) * Affine::rotate(0.7)),
        ] {
            let (d, _, s) = ellipse_doc(w, h, (0.0, 360.0), xf);
            let ws = PieWidget::of(&d, &s, 1.0);
            assert_eq!(ws[0].at, ws[1].at, "{ws:?}");
            let c = cx(&d, &s, &p);
            let mut t = SelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, ws[0].at));
            let center = xf * Point::new(w / 2.0, h / 2.0);
            let a = t.pointer(&c, &ev(PointerKind::Drag, center + (xf * Point::new(w / 2.0, 0.0) - center) * 2.0));
            assert!(matches!(&a[1], Action::Preview(_, v) if v.get("pieEnd").is_some() && v.get("pieStart").is_none()), "{a:?}");
        }
    }

    /// The Start widget sets Pie Start Angle and leaves the end where it is.
    #[test]
    fn dragging_the_start_widget_sets_the_start_angle() {
        let (d, _, s) = doc_with_ellipse((0.0, 90.0));
        let p = paint();
        let c = cx(&d, &s, &p);
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, Point::new(312.0, 150.0)));
        let a = t.pointer(&c, &ev(PointerKind::Drag, Point::new(50.0, 150.0)));
        let Action::Preview(_, v) = &a[1] else { panic!("{a:?}") };
        assert!((v["pieStart"].as_f64().unwrap() - 180.0).abs() < 1e-9, "{v}");
        assert!(v.get("pieEnd").is_none(), "{v}");
    }

    /// A double-click on either widget makes the ellipse whole again.
    #[test]
    fn a_double_click_on_a_widget_resets_the_pie() {
        let (d, id, s) = doc_with_ellipse((30.0, 120.0));
        let p = paint();
        let c = cx(&d, &s, &p);
        let reset = vec![Action::Exec("object.setLiveShape".into(), json!({ "id": id.0, "pieStart": 0, "pieEnd": 360 }))];
        for w in PieWidget::of(&d, &s, 1.0) {
            let mut t = SelectionTool::default();
            for k in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up] {
                assert_eq!(t.pointer(&c, &ev(k, w.at)), vec![], "{k:?}");
            }
            assert_eq!(t.pointer(&c, &ev(PointerKind::DoubleClick, w.at)), reset);
        }
    }
}
