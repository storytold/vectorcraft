//! Live Corners: the widgets inside each corner of a selected live rectangle. Dragging one rounds
//! (or sharpens) all four corners together; with corner anchors picked by the Direct Selection
//! tool only those corners show widgets, and a drag rounds just them. The Selection and Direct
//! Selection tools share this, and the canvas draws the widgets from the same geometry.

use serde_json::json;
use vectorcraft_doc::{Document, LiveShape, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Affine, Point, Vec2};

use crate::{Action, Overlay, ToolContext};

/// Widgets sit at least this far inside their corner (screen px), further in once the radius is.
const MIN_INSET_PX: f64 = 10.0;
/// Shapes whose shorter side is smaller than this on screen (px) hide their widgets.
const MIN_SIDE_PX: f64 = 3.0 * MIN_INSET_PX;
/// Corners in radii order (top-left, top-right, bottom-right, bottom-left): position as factors
/// of (w, h) and the inward diagonal.
const CORNERS: [((f64, f64), (f64, f64)); 4] =
    [((0.0, 0.0), (1.0, 1.0)), ((1.0, 0.0), (-1.0, 1.0)), ((1.0, 1.0), (-1.0, -1.0)), ((0.0, 1.0), (1.0, -1.0))];

/// The corner widgets of a selection that is exactly one editable live rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerWidgets {
    pub id: NodeId,
    w: f64,
    h: f64,
    radii: [f64; 4],
    xf: Affine,
    /// Widget centres in document coordinates, in radii order.
    pub points: [Point; 4],
    /// The corners with a widget: all four, or those of the direct-selected anchors.
    pub active: [bool; 4],
}

impl CornerWidgets {
    /// The widgets at `zoom` (screen px per document point), if the selection has them.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        let [id] = selection.objects[..] else { return None };
        if !doc.is_editable(id) {
            return None;
        }
        let Some(NodeKind::Path { live: Some(live @ LiveShape::Rectangle { w, h, radii, xf }), .. }) = doc.node(id).map(|n| &n.kind) else {
            return None;
        };
        let picked = live.rect_corners_of(selection.partial(id));
        let active = std::array::from_fn(|k| picked.as_ref().is_none_or(|c| c.contains(&k)));
        let (w, h, radii, xf) = (*w, *h, *radii, *xf);
        // Screen pixels per shape unit along each side (the shape may be scaled or skewed).
        let c = xf.as_coeffs();
        let (px, py) = (Vec2::new(c[0], c[1]).hypot() * zoom, Vec2::new(c[2], c[3]).hypot() * zoom);
        if xf.determinant().abs() < 1e-12 || (w * px).min(h * py) < MIN_SIDE_PX {
            return None;
        }
        let points = std::array::from_fn(|k| {
            let ((fx, fy), (sx, sy)) = CORNERS[k];
            let ix = radii[k].max(MIN_INSET_PX / px).min(w / 2.0);
            let iy = radii[k].max(MIN_INSET_PX / py).min(h / 2.0);
            xf * Point::new(fx * w + sx * ix, fy * h + sy * iy)
        });
        Some(Self { id, w, h, radii, xf, points, active })
    }

    /// The widgets the active tool can drag (View → Show Corner Widget on).
    pub fn for_tool(cx: &ToolContext) -> Option<Self> {
        if !cx.corner_widgets {
            return None;
        }
        Self::of(cx.doc, cx.selection, cx.zoom)
    }

    /// The centres of the widgets shown, with their corner index.
    pub fn shown(&self) -> impl Iterator<Item = (usize, Point)> + '_ {
        (0..4).filter(|k| self.active[*k]).map(|k| (k, self.points[k]))
    }

    /// Index of the shown widget nearest to `p` within `tol` (document units).
    pub fn hit(&self, p: Point, tol: f64) -> Option<usize> {
        (0..4)
            .filter(|k| self.active[*k] && self.points[*k].distance(p) <= tol)
            .min_by(|a, b| self.points[*a].distance(p).total_cmp(&self.points[*b].distance(p)))
    }
}

/// Is `p` over a corner widget the active tool would drag?
pub fn over_widget(cx: &ToolContext, p: Point) -> bool {
    CornerWidgets::for_tool(cx).and_then(|w| w.hit(p, cx.tol(5.0))).is_some()
}

/// Dragging a corner widget: the radius follows the pointer along the corner's diagonal (the engine
/// applies it to the picked corners, or all four).
#[derive(Clone, Copy, Debug)]
pub struct CornerDrag {
    widgets: CornerWidgets,
    corner: usize,
    start: Point,
    began: bool,
    radius: f64,
    at: Point,
}

impl CornerDrag {
    /// Start a drag when `p` is on a widget of the selection.
    pub fn hit(cx: &ToolContext, p: Point) -> Option<Self> {
        let widgets = CornerWidgets::for_tool(cx)?;
        let corner = widgets.hit(p, cx.tol(5.0))?;
        Some(Self { widgets, corner, start: p, began: false, radius: widgets.radii[corner], at: p })
    }

    /// The start radius plus the pointer's travel along the corner's inward diagonal (in the
    /// shape's own units), from square to fully round.
    fn radius_at(&self, p: Point) -> f64 {
        let w = &self.widgets;
        let inv = w.xf.inverse();
        let d = inv * p - inv * self.start;
        let (_, (sx, sy)) = CORNERS[self.corner];
        (w.radii[self.corner] + (d.x * sx + d.y * sy) / 2.0).clamp(0.0, w.w.min(w.h) / 2.0)
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Corner Radius".into()));
        }
        self.radius = self.radius_at(p);
        self.at = p;
        out.push(Action::Preview("object.setLiveShape".into(), json!({ "id": self.widgets.id.0, "radius": self.radius })));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The radius readout next to the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.began {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: format!("Radius: {}", cx.len(self.radius)) }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{PointerEvent, PointerKind};
    use vectorcraft_doc::{Appearance, Node};

    /// A live 100 × 100 rectangle at (100, 100) with corner radius `r`.
    fn live_rect(r: f64, xf: Affine) -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let live = LiveShape::Rectangle { w: 100.0, h: 100.0, radii: [r; 4], xf };
        let mut n = Node::path(id, live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(live);
        }
        d.insert(Some(l), 0, n).unwrap();
        (d, id)
    }

    fn selected(id: NodeId) -> Selection {
        let mut s = Selection::default();
        s.add(id);
        s
    }

    fn radius(a: &Action) -> f64 {
        let Action::Preview(c, v) = a else { panic!("not a preview: {a:?}") };
        assert_eq!(c, "object.setLiveShape");
        v["radius"].as_f64().unwrap()
    }

    #[test]
    fn widgets_sit_inside_each_corner() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let w = CornerWidgets::of(&d, &selected(id), 1.0).unwrap();
        assert_eq!(w.points, [Point::new(110.0, 110.0), Point::new(190.0, 110.0), Point::new(190.0, 190.0), Point::new(110.0, 190.0)]);
        // A larger radius moves them to the arc centres; zooming in keeps a 10 px minimum.
        let (d, id) = live_rect(20.0, Affine::translate((100.0, 100.0)));
        assert_eq!(CornerWidgets::of(&d, &selected(id), 1.0).unwrap().points[0], Point::new(120.0, 120.0));
        assert_eq!(CornerWidgets::of(&d, &selected(id), 4.0).unwrap().points[2], Point::new(180.0, 180.0));
        // Hidden when the shape is tiny on screen, for plain paths and without a single selection.
        assert!(CornerWidgets::of(&d, &selected(id), 0.2).is_none());
        let (d, id) = doc_with_rect();
        assert!(CornerWidgets::of(&d, &selected(id), 1.0).is_none());
        assert!(CornerWidgets::of(&d, &Selection::default(), 1.0).is_none());
    }

    #[test]
    fn dragging_a_widget_inward_rounds_all_corners_in_one_step() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        for mut t in [crate::create("selection"), crate::create("directSelection")] {
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 191.0)).is_empty());
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 175.0, 176.0));
            assert_eq!(a[0], Action::Begin("Corner Radius".into()));
            assert!((radius(&a[1]) - 15.0).abs() < 1e-9);
            assert!(matches!(&a[1], Action::Preview(_, v) if v["id"] == id.0));
            assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "Radius: 15.00 pt")));
            // Past the middle the corner is fully round; back outside it is square.
            assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 100.0))[0]), 50.0);
            assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 260.0, 260.0))[0]), 0.0);
            assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 260.0, 260.0)), vec![Action::Commit]);
        }
    }

    #[test]
    fn drag_follows_a_rotated_shape_and_a_click_does_nothing() {
        // Rotated 90°: the shape's top-left corner is at document (200, 100).
        let xf = Affine::translate((200.0, 100.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let (d, id) = live_rect(10.0, xf);
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let w = CornerWidgets::of(&d, &s, 1.0).unwrap();
        assert!(w.points[0].distance(Point::new(190.0, 110.0)) < 1e-9);
        let mut t = crate::create("directSelection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 120.0));
        assert!((radius(&a[1]) - 20.0).abs() < 1e-9);
        // A click without a drag leaves the shape (and the undo history) alone.
        let mut t = crate::create("selection");
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0)).is_empty());
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 190.0, 110.0)).is_empty());
    }

    #[test]
    fn picked_corner_anchors_keep_only_their_widgets() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        // The Direct Selection tool picked the top-right corner anchor.
        let mut s = selected(id);
        s.anchors.insert(id, [(0, 1)].into_iter().collect());
        let w = CornerWidgets::of(&d, &s, 1.0).unwrap();
        assert_eq!(w.active, [false, true, false, false]);
        assert_eq!(w.shown().collect::<Vec<_>>(), [(1, Point::new(190.0, 110.0))]);
        assert_eq!(w.hit(Point::new(110.0, 110.0), 5.0), None);
        // Its widget drags (the engine rounds the picked corner only).
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = crate::create("directSelection");
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 120.0));
        assert!((radius(&a[1]) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn widgets_off_or_elsewhere_keep_the_tools_behaviour() {
        let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
        let s = selected(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        assert!(over_widget(&c, Point::new(110.0, 110.0)));
        assert!(!over_widget(&c, Point::new(150.0, 150.0)));
        c.corner_widgets = false;
        assert!(!over_widget(&c, Point::new(110.0, 110.0)));
        // With the widgets hidden, a drag from the same spot moves the object.
        let mut t = crate::create("selection");
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 110.0, 110.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 110.0));
        assert_eq!(a[0], Action::Begin("Move".into()));
    }
}
