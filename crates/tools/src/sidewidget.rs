//! The side widget: a small diamond beside the right side of a selected live polygon's bounding
//! box (<https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-live-shapes/add-or-remove-sides-from-polygons.html>).
//! Dragging it down adds sides and dragging it up removes them, one side per [`STEP_PX`] screen
//! pixels along the box's own vertical axis, from 3 to 11 sides. A polygon with more than 11
//! sides keeps its count until the drag goes up a step, which gives it 11. The drag sends
//! `object.setLiveShape`'s `sides` in one undo step, and none when the count ends where it began.
//! The Selection tool and the shape tools that show the box hit the widget, and the canvas draws it
//! from the same geometry.

use serde_json::json;
use vectorcraft_doc::{Document, LiveShape, NodeId, NodeKind, OrientedBox, Selection};
use vectorcraft_geom::Point;

use crate::select::shown_box;
use crate::{Action, Overlay, PointerEvent, ToolContext};

/// The widget's center sits this far (screen px) outside the middle of the box's right side, clear
/// of its handle.
const OFFSET_PX: f64 = 16.0;
/// Half the widget's diagonal (screen px), as the canvas draws it.
pub const RADIUS_PX: f32 = 4.0;
/// How near (screen px) the pointer must be to hit it.
const HIT_PX: f64 = 5.0;
/// How far (screen px) the pointer moves along the box's vertical axis per side.
pub const STEP_PX: f64 = 12.0;
/// The fewest sides a drag gives.
pub const MIN_SIDES: u32 = 3;
/// The most sides a drag gives.
pub const MAX_SIDES: u32 = 11;

/// The side widget of a selection that is exactly one editable live polygon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideWidget {
    pub id: NodeId,
    /// Center in document coordinates.
    pub at: Point,
    /// The polygon's side count.
    pub sides: u32,
}

impl SideWidget {
    /// The widget beside `bx`, the selection's bounding box, at `zoom` (screen px per document
    /// point). None unless the selection is one whole editable live polygon, with no anchors or
    /// segments of it selected.
    pub fn of(doc: &Document, selection: &Selection, bx: &OrientedBox, zoom: f64) -> Option<Self> {
        let [id] = selection.objects[..] else { return None };
        if !selection.anchors.is_empty() || selection.segments.contains_key(&id) || !doc.is_editable(id) || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let NodeKind::Path { live, .. } = &doc.node(id)?.kind else { return None };
        let Some(LiveShape::Polygon { sides, .. }) = live.as_deref() else { return None };
        let r = bx.rect;
        let at = bx.to_doc() * Point::new(r.x1 + OFFSET_PX / zoom, r.center().y);
        (at.x.is_finite() && at.y.is_finite()).then_some(Self { id, at, sides: *sides })
    }

    /// The widget the tool can drag (its bounding box showing).
    pub fn for_tool(cx: &ToolContext) -> Option<(Self, OrientedBox)> {
        // One object or none: a larger selection's box is not measured.
        if cx.selection.objects.len() != 1 {
            return None;
        }
        let bx = shown_box(cx)?;
        Some((Self::of(cx.doc, cx.selection, &bx, cx.zoom)?, bx))
    }

    /// The widget of `cx` and its box if the widget is under `p`.
    fn at(cx: &ToolContext, p: Point) -> Option<(Self, OrientedBox)> {
        Self::for_tool(cx).filter(|(w, _)| w.at.distance(p) <= cx.tol(HIT_PX))
    }
}

/// Is `p` over the side widget?
pub fn over_widget(cx: &ToolContext, p: Point) -> bool {
    SideWidget::at(cx, p).is_some()
}

/// The side count a drag of `steps` (down positive) gives a polygon of `from` sides: within 3 to 11,
/// and `from` itself while a polygon of more than 11 sides has not been dragged up.
pub fn sides_after(from: u32, steps: i64) -> u32 {
    let (min, max) = (i64::from(MIN_SIDES), i64::from(MAX_SIDES));
    let to = if from <= MAX_SIDES {
        i64::from(from).saturating_add(steps)
    } else if steps >= 0 {
        return from;
    } else {
        (max + 1).saturating_add(steps)
    };
    to.clamp(min, max) as u32
}

/// Dragging the side widget.
#[derive(Clone, Debug)]
pub struct SideDrag {
    widget: SideWidget,
    bx: OrientedBox,
    start: Point,
    /// The pointer has left the click.
    moved: bool,
    /// The undo step has begun: the count has differed from the polygon's.
    began: bool,
    /// The count last sent (the polygon's until the drag changes it).
    sides: u32,
    at: Point,
}

impl SideDrag {
    /// The drag has left the click.
    pub fn adjusting(&self) -> bool {
        self.moved
    }

    /// Start a drag when the press `ev` is on the side widget of the selection.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let (widget, bx) = SideWidget::at(cx, ev.pos)?;
        Some(Self { widget, bx, start: ev.pos, moved: false, began: false, sides: widget.sides, at: ev.pos })
    }

    /// A pointer position that is not finite leaves the count and the readout as they are. A drag
    /// too long to measure counts as the most steps in its direction.
    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        if !p.x.is_finite() || !p.y.is_finite() {
            return vec![];
        }
        self.at = p;
        if !self.moved {
            if p.distance(self.start) < cx.tol(3.0) {
                return vec![];
            }
            self.moved = true;
        }
        let down = (self.bx.to_local(p).y - self.bx.to_local(self.start).y) * cx.zoom;
        let steps = (down / STEP_PX).trunc();
        if steps.is_nan() {
            return vec![];
        }
        // The cast saturates: an infinite count of steps becomes i64::MAX or i64::MIN.
        let sides = sides_after(self.widget.sides, steps as i64);
        if sides == self.sides {
            return vec![];
        }
        let mut out = vec![];
        if !self.began {
            self.began = true;
            out.push(Action::Begin("Live Shape".into()));
        }
        self.sides = sides;
        out.push(Action::Preview("object.setLiveShape".into(), json!({ "id": self.widget.id.0, "sides": sides })));
        out
    }

    /// The release: one undo step when the count changed, and nothing kept when it came back.
    pub fn finish(self) -> Vec<Action> {
        match (self.began, self.sides == self.widget.sides) {
            (false, _) => vec![],
            (true, true) => vec![Action::Cancel],
            (true, false) => vec![Action::Commit],
        }
    }

    /// Escape: the polygon keeps the count it had.
    pub fn cancel(self) -> Vec<Action> {
        if self.began { vec![Action::Cancel] } else { vec![] }
    }

    /// The side count next to the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if !self.moved || !cx.measurement_labels {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: format!("Sides: {}", self.sides) }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::SelectionTool;
    use crate::testutil::*;
    use crate::{Cursor, Mods, PointerKind, Tool, ToolKey};
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::Affine;

    /// A document holding a live polygon of `sides` sides and radius 100 placed by `xf`, and a
    /// selection of it.
    fn polygon_doc(sides: u32, xf: Affine) -> (Document, NodeId, Selection) {
        let live = LiveShape::Polygon { radius: 100.0, sides, xf, radii: vec![], kinds: vec![], radii_relative: false };
        let mut d = Document::new(800.0, 800.0);
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

    fn sides_of(a: &[Action]) -> Vec<u64> {
        a.iter().filter_map(|a| if let Action::Preview(_, v) = a { v["sides"].as_u64() } else { None }).collect()
    }

    #[test]
    fn the_counts_stay_within_three_and_eleven() {
        assert_eq!(sides_after(5, 1), 6);
        assert_eq!(sides_after(5, -1), 4);
        assert_eq!(sides_after(5, 100), 11);
        assert_eq!(sides_after(5, -100), 3);
        assert_eq!(sides_after(3, -1), 3);
        assert_eq!(sides_after(11, 1), 11);
        // Over 11: unchanged until the drag goes up, which gives 11, then 10.
        assert_eq!(sides_after(40, 0), 40);
        assert_eq!(sides_after(40, 5), 40);
        assert_eq!(sides_after(40, -1), 11);
        assert_eq!(sides_after(40, -2), 10);
        assert_eq!(sides_after(1000, -100), 3);
    }

    #[test]
    fn the_longest_drags_stay_within_three_and_eleven() {
        assert_eq!(sides_after(5, i64::MAX), 11);
        assert_eq!(sides_after(5, i64::MIN), 3);
        assert_eq!(sides_after(3, i64::MAX), 11);
        assert_eq!(sides_after(11, i64::MAX), 11);
        assert_eq!(sides_after(11, i64::MIN), 3);
        assert_eq!(sides_after(40, i64::MAX), 40);
        assert_eq!(sides_after(40, i64::MIN), 3);
    }

    /// A drag far past the document, even one too long to measure, stops at 11 down and 3 up; a
    /// pointer position that is not finite keeps the count it had.
    #[test]
    fn a_huge_or_invalid_drag_stays_within_three_and_eleven() {
        let (d, _, s) = polygon_doc(5, Affine::translate((300.0, 300.0)));
        let p = paint();
        // At zoom 1 the step count is finite and past i64; at zoom 4 the distance itself is infinite.
        for zoom in [1.0, 4.0] {
            let c = ToolContext { zoom, ..cx(&d, &s, &p) };
            let (w, _) = SideWidget::for_tool(&c).unwrap();
            let far = |y: f64| Point::new(w.at.x, y);
            let mut t = SelectionTool::default();
            t.pointer(&c, &ev(PointerKind::Down, w.at));
            assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, far(1e308)))), [11], "zoom {zoom}");
            assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, far(-1e308)))), [3], "zoom {zoom}");
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, far(-1e308))), vec![Action::Commit]);
        }
        let c = cx(&d, &s, &p);
        let (w, _) = SideWidget::for_tool(&c).unwrap();
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at));
        let shown = w.at + vectorcraft_geom::Vec2::new(0.0, 24.0);
        assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, shown))), [7]);
        for bad in [Point::new(f64::NAN, w.at.y), Point::new(w.at.x, f64::INFINITY), Point::new(w.at.x, f64::NEG_INFINITY)] {
            assert!(t.pointer(&c, &ev(PointerKind::Drag, bad)).is_empty(), "{bad:?}");
        }
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { p, text } if *p == shown && text == "Sides: 7")));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, shown)), vec![Action::Commit]);
    }

    /// The widget sits 16 px right of the middle of the box's right side, at any zoom; only one
    /// whole editable live polygon shows it.
    #[test]
    fn one_whole_live_polygon_shows_the_widget() {
        let (d, id, s) = polygon_doc(6, Affine::translate((300.0, 300.0)));
        let p = paint();
        let c = cx(&d, &s, &p);
        let (w, bx) = SideWidget::for_tool(&c).expect("a widget");
        assert_eq!((w.id, w.sides), (id, 6));
        let want = |zoom: f64| Point::new(bx.rect.x1 + 16.0 / zoom, bx.rect.center().y);
        assert!(w.at.distance(want(1.0)) < 1e-9, "{w:?} {bx:?}");
        assert!(SideWidget::of(&d, &s, &bx, 4.0).unwrap().at.distance(want(4.0)) < 1e-9);
        assert!(over_widget(&c, w.at) && !over_widget(&c, w.at + vectorcraft_geom::Vec2::new(0.0, 8.0)));
        // Hidden with the bounding box hidden, with an anchor or segment selected, or with no zoom.
        assert!(SideWidget::for_tool(&ToolContext { show_bbox: false, ..cx(&d, &s, &p) }).is_none());
        let mut part = s.clone();
        part.segments.insert(id, Default::default());
        assert!(SideWidget::of(&d, &part, &bx, 1.0).is_none());
        assert!(SideWidget::of(&d, &s, &bx, 0.0).is_none() && SideWidget::of(&d, &s, &bx, f64::NAN).is_none());
        // A locked polygon, two polygons, nothing or a plain path: none.
        let mut locked = d.clone();
        locked.node_mut(id).unwrap().locked = true;
        assert!(SideWidget::of(&locked, &s, &bx, 1.0).is_none());
        let mut two = d.clone();
        let other = two.alloc_id();
        let mut n = two.node(id).unwrap().clone();
        n.id = other;
        let l = two.layers[0].id;
        two.insert(Some(l), 0, n).unwrap();
        let mut both = s.clone();
        both.add(other);
        assert!(SideWidget::of(&two, &both, &bx, 1.0).is_none());
        assert!(SideWidget::of(&d, &Selection::default(), &bx, 1.0).is_none());
        let (d, rect) = doc_with_rect();
        let mut s = Selection::default();
        s.add(rect);
        let bx = OrientedBox::aligned(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(SideWidget::of(&d, &s, &bx, 1.0).is_none());
    }

    /// With the Selection tool, dragging the widget down adds a side per 12 screen px and up removes
    /// them, within 3 to 11, in one undo step with the count shown next to the pointer.
    #[test]
    fn dragging_the_widget_changes_the_sides() {
        let (d, id, s) = polygon_doc(5, Affine::translate((300.0, 300.0)));
        let p = paint();
        for zoom in [1.0, 2.0] {
            let c = ToolContext { zoom, ..cx(&d, &s, &p) };
            let (w, _) = SideWidget::for_tool(&c).unwrap();
            let at = |px: f64| w.at + vectorcraft_geom::Vec2::new(0.0, px / zoom);
            let mut t = SelectionTool::default();
            assert_eq!(t.cursor(&c, w.at, Mods::default()), Cursor::ArrowHollow);
            assert!(t.pointer(&c, &ev(PointerKind::Down, w.at)).is_empty());
            // Past the click but short of a step: nothing yet.
            assert!(t.pointer(&c, &ev(PointerKind::Drag, at(6.0))).is_empty());
            assert!(t.transforming());
            let a = t.pointer(&c, &ev(PointerKind::Drag, at(12.0)));
            assert_eq!(a[0], Action::Begin("Live Shape".into()));
            assert_eq!(a[1], Action::Preview("object.setLiveShape".into(), json!({ "id": id.0, "sides": 6 })));
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "Sides: 6")));
            // The same count again sends nothing; far down stops at 11, far up at 3.
            assert!(t.pointer(&c, &ev(PointerKind::Drag, at(20.0))).is_empty());
            assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, at(500.0)))), [11]);
            assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, at(-500.0)))), [3]);
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, at(-500.0))), vec![Action::Commit]);
            assert!(!t.busy() && !t.transforming());
        }
        // A click sends nothing, and the readout follows Measurement Labels.
        let c = cx(&d, &s, &p);
        let (w, _) = SideWidget::for_tool(&c).unwrap();
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, w.at)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, w.at)).is_empty());
        t.pointer(&c, &ev(PointerKind::Down, w.at));
        t.pointer(&c, &ev(PointerKind::Drag, w.at + vectorcraft_geom::Vec2::new(0.0, 13.0)));
        let quiet = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        assert!(!t.overlays(&quiet).iter().any(|o| matches!(o, Overlay::Measure { .. })));
        // Back at the start: the release keeps nothing.
        assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, w.at))), [5]);
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, w.at)), vec![Action::Cancel]);
    }

    /// On a turned polygon the drag is read along the box's own vertical axis.
    #[test]
    fn a_turned_polygon_reads_the_drag_along_its_box() {
        let xf = Affine::translate((400.0, 400.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let (d, _, s) = polygon_doc(5, xf);
        let p = paint();
        let c = cx(&d, &s, &p);
        let (w, bx) = SideWidget::for_tool(&c).unwrap();
        let down = bx.to_doc() * Point::new(0.0, 1.0) - bx.to_doc() * Point::ZERO;
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at));
        assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, w.at + down * 25.0))), [7], "{bx:?}");
        assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, w.at - down * 13.0))), [4]);
    }

    /// A double-click on the widget with the Selection tool does nothing: in isolation mode it does
    /// not leave isolation, though no art is under the widget.
    #[test]
    fn a_double_click_on_the_widget_does_nothing() {
        let live = LiveShape::Polygon {
            radius: 100.0,
            sides: 5,
            xf: Affine::translate((300.0, 300.0)),
            radii: vec![],
            kinds: vec![],
            radii_relative: false,
        };
        let mut d = Document::new(800.0, 800.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut n = Node::path(id, live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(live));
        }
        let group = d.alloc_id();
        d.insert(Some(l), 0, Node::group(group, vec![std::sync::Arc::new(n)])).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let c = ToolContext { isolation: Some(group), ..cx(&d, &s, &p) };
        let (w, _) = SideWidget::for_tool(&c).unwrap();
        assert!(vectorcraft_doc::hit::hit_test(&d, w.at, c.hit_options()).is_none());
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::DoubleClick, w.at)).is_empty());
        // Away from the widget and the art, the double-click leaves isolation.
        let away = w.at + vectorcraft_geom::Vec2::new(40.0, 0.0);
        assert_eq!(t.pointer(&c, &ev(PointerKind::DoubleClick, away)), vec![Action::Exec("object.exitIsolation".into(), json!({}))]);
    }

    /// A polygon of more than 11 sides keeps its count on a press and a drag down; the first step
    /// up gives 11.
    #[test]
    fn a_polygon_over_eleven_sides_does_not_jump() {
        let (d, _, s) = polygon_doc(40, Affine::translate((300.0, 300.0)));
        let p = paint();
        let c = cx(&d, &s, &p);
        let (w, _) = SideWidget::for_tool(&c).unwrap();
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, w.at)).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Drag, w.at + vectorcraft_geom::Vec2::new(0.0, 40.0))).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, w.at + vectorcraft_geom::Vec2::new(0.0, 40.0))).is_empty());
        t.pointer(&c, &ev(PointerKind::Down, w.at));
        let a = t.pointer(&c, &ev(PointerKind::Drag, w.at - vectorcraft_geom::Vec2::new(0.0, 12.0)));
        assert_eq!(a[0], Action::Begin("Live Shape".into()));
        assert_eq!(sides_of(&a), [11]);
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "Sides: 11")));
        assert_eq!(sides_of(&t.pointer(&c, &ev(PointerKind::Drag, w.at - vectorcraft_geom::Vec2::new(0.0, 24.0)))), [10]);
    }

    /// Escape during a drag puts the count back; the release after it sends nothing.
    #[test]
    fn escape_cancels_a_side_widget_drag() {
        let (d, _, s) = polygon_doc(5, Affine::translate((300.0, 300.0)));
        let p = paint();
        let c = cx(&d, &s, &p);
        let (w, _) = SideWidget::for_tool(&c).unwrap();
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, w.at));
        t.pointer(&c, &ev(PointerKind::Drag, w.at + vectorcraft_geom::Vec2::new(0.0, 30.0)));
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy() && !t.transforming());
        assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { .. })));
        assert!(t.pointer(&c, &ev(PointerKind::Up, w.at)).is_empty());
    }
}
