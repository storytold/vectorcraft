//! The divider of an Image Trace object in the Split view: the source image shows on its left
//! (in the image's own pixels) and the tracing result on its right. Drawing and hit testing share
//! the document-space geometry here; a drag previews `imageTrace.setView` and commits it as one
//! undo step.
use serde_json::json;
use vectorcraft_doc::{Document, NodeId, NodeKind, Selection, TraceView};
use vectorcraft_geom::{Affine, Point};

use crate::{Action, Cursor, Mods, ToolContext};

/// How near (screen points) the pointer must come to the divider to take it.
pub const HIT_PX: f64 = 5.0;

/// The divider's half length past each end of its grip, in screen points.
pub const GRIP_HALF: f32 = 10.0;

/// The Split view's divider of one selected Image Trace object, in document space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceDivider {
    /// The Image Trace object.
    pub id: NodeId,
    /// The fraction of the source image's width, from its left edge, that shows the image.
    pub split: f64,
    /// Where the divider meets the image's top and bottom edges.
    pub ends: [Point; 2],
    /// The source image's pixel-to-document transform.
    xf: Affine,
    width: f64,
}

impl TraceDivider {
    /// The divider of the one Image Trace object selected whole, when it shows the Split view.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        let [id] = selection.objects.as_slice() else { return None };
        if !selection.anchors.is_empty() || !selection.segments.is_empty() || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let n = doc.node(*id)?;
        if n.extras().trace.is_none() || n.trace_view() != TraceView::Split {
            return None;
        }
        let NodeKind::Group { children, clip: false } = &n.kind else { return None };
        let NodeKind::Image(im) = &children.first()?.kind else { return None };
        // After the kind: on a locked artboard it measures the art of the layer object holding `id`.
        if !doc.is_editable(*id) {
            return None;
        }
        let (width, height) = (f64::from(im.width), f64::from(im.height));
        let det = im.xf.determinant();
        if width <= 0.0 || height <= 0.0 || !det.is_finite() || det.abs() < 1e-12 || !im.xf.as_coeffs().iter().all(|c| c.is_finite()) {
            return None;
        }
        let split = n.trace_split();
        let at = width * split;
        let ends = [im.xf * Point::new(at, 0.0), im.xf * Point::new(at, height)];
        ends.iter().all(|p| finite(*p)).then_some(Self { id: *id, split, ends, xf: im.xf, width })
    }

    /// The divider position for document point `p`: the fraction of the image's width left of
    /// it, from 0 to 1, rounded to a ten-thousandth.
    pub fn split_at(&self, p: Point) -> Option<f64> {
        let local = self.xf.inverse() * p;
        let f = (local.x / self.width).clamp(0.0, 1.0);
        f.is_finite().then(|| (f * 10_000.0).round() / 10_000.0)
    }

    /// How far the image's left-to-right direction is turned (counter-clockwise degrees on the
    /// page), for the resize cursor.
    fn angle(&self) -> f64 {
        let across = self.xf * Point::new(1.0, 0.0) - self.xf * Point::ZERO;
        (-across.y).atan2(across.x).to_degrees()
    }

    /// The divider under document point `p`. Nothing with the bounding box hidden or in Outline
    /// mode, where the canvas draws neither the divider nor the split.
    fn hit(cx: &ToolContext, p: Point) -> Option<Self> {
        if !cx.show_bbox || cx.outline || !finite(p) {
            return None;
        }
        let d = Self::of(cx.doc, cx.selection, cx.zoom)?;
        let [a, b] = d.ends;
        let v = b - a;
        let length = v.hypot();
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        let unit = v / length;
        let along = (p - a).dot(unit).clamp(0.0, length);
        // A bounding-box handle or a shown center point under the pointer keeps its own drag.
        ((a + unit * along).distance(p) <= cx.tol(HIT_PX)
            && !crate::select::bbox_handle_at(cx, p)
            && crate::select::hit_center(cx, p, cx.point_tol()).is_none())
        .then_some(d)
    }
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The resize cursor along the divider `d`, turned with the image and the view.
pub fn resize_cursor(d: &TraceDivider, cx: &ToolContext) -> Cursor {
    crate::select::handle_cursor(crate::bbox::Handle::Right, d.angle() - cx.view_angle())
}

/// The resize cursor over the divider at `p`, turned with the image and the view.
pub fn cursor(cx: &ToolContext, p: Point) -> Option<Cursor> {
    TraceDivider::hit(cx, p).map(|d| resize_cursor(&d, cx))
}

/// A drag of the divider: it previews `imageTrace.setView` while the pointer moves and commits
/// it as one undo step.
#[derive(Clone, Debug)]
pub struct SplitDrag {
    divider: TraceDivider,
    start: Point,
    began: bool,
}

impl SplitDrag {
    pub fn hit(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt {
            return None;
        }
        let divider = TraceDivider::hit(cx, p)?;
        Some(Self { divider, start: p, began: false })
    }

    pub fn adjusting(&self) -> bool {
        self.began
    }

    pub fn cursor(&self, cx: &ToolContext) -> Cursor {
        resize_cursor(&self.divider, cx)
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        if !finite(p) || (!self.began && p.distance(self.start) < cx.tol(3.0)) {
            return vec![];
        }
        let Some(split) = self.divider.split_at(p) else { return vec![] };
        let mut out = vec![];
        if !self.began {
            self.began = true;
            out.push(Action::Begin("Image Trace View".into()));
        }
        out.push(Action::Preview("imageTrace.setView".into(), json!({"id": self.divider.id.0, "view": TraceView::Split.id(), "split": split})));
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    pub fn cancel(self) -> Vec<Action> {
        if self.began { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cx, paint};
    use std::sync::Arc;
    use vectorcraft_doc::{ImageObject, Node};

    /// A document with an Image Trace object over a 100 × 50 image placed by `xf`, in `view`.
    fn traced(xf: Affine, view: &str, split: f64) -> (Document, NodeId) {
        let mut doc = Document::new(400.0, 400.0);
        let im = ImageObject { key: "k".into(), width: 100, height: 50, xf, link: None, placement: Default::default(), pixels: None };
        let mut source = Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)));
        source.visible = false;
        let mut g = Node::group(doc.alloc_id(), vec![Arc::new(source)]);
        g.extras_mut().trace = Some(Box::new(json!({"view": view, "split": split})));
        let id = g.id;
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), 0, g).unwrap();
        (doc, id)
    }

    #[test]
    fn the_divider_shows_only_for_one_selected_split_view_and_follows_the_image() {
        let paint = paint();
        let mut selection = Selection::default();
        let (doc, id) = traced(Affine::translate((10.0, 20.0)), "split", 0.25);
        assert!(TraceDivider::of(&doc, &selection, 1.0).is_none(), "nothing selected");
        selection.set([id]);
        let d = TraceDivider::of(&doc, &selection, 1.0).unwrap();
        assert_eq!((d.split, d.ends), (0.25, [Point::new(35.0, 20.0), Point::new(35.0, 70.0)]));
        for zoom in [0.0, -1.0, f64::NAN] {
            assert!(TraceDivider::of(&doc, &selection, zoom).is_none());
        }
        let (other, id) = traced(Affine::IDENTITY, "sourceImage", 0.25);
        selection.set([id]);
        assert!(TraceDivider::of(&other, &selection, 1.0).is_none(), "another view");
        // A turned and scaled image: the divider runs along the image's own height.
        let xf = Affine::translate((200.0, 100.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::scale(2.0);
        let (doc, id) = traced(xf, "split", 0.5);
        selection.set([id]);
        let d = TraceDivider::of(&doc, &selection, 1.0).unwrap();
        assert!(d.ends[0].distance(Point::new(200.0, 200.0)) < 1e-9 && d.ends[1].distance(Point::new(100.0, 200.0)) < 1e-9, "{:?}", d.ends);
        assert_eq!(d.split_at(Point::new(150.0, 150.0)), Some(0.25));
        assert_eq!(d.split_at(Point::new(150.0, 900.0)), Some(1.0), "clamped");
        let c = cx(&doc, &selection, &paint);
        assert_eq!(cursor(&c, Point::new(150.0, 200.0)), Some(Cursor::ResizeV));
        assert_eq!(cursor(&c, Point::new(150.0, 210.0)), None, "past the hit distance");
        let mut c = cx(&doc, &selection, &paint);
        c.show_bbox = false;
        assert_eq!(cursor(&c, Point::new(150.0, 200.0)), None, "bounding box hidden");
        assert!(SplitDrag::hit(&c, Point::new(150.0, 200.0), Mods::default()).is_none(), "bounding box hidden");
        let mut c = cx(&doc, &selection, &paint);
        c.outline = true;
        assert!(SplitDrag::hit(&c, Point::new(150.0, 200.0), Mods::default()).is_none(), "Outline mode");
    }

    #[test]
    fn dragging_the_divider_previews_the_view_and_commits_once() {
        let paint = paint();
        let mut selection = Selection::default();
        let (doc, id) = traced(Affine::translate((10.0, 20.0)), "split", 0.5);
        selection.set([id]);
        for zoom in [0.5, 1.0, 4.0] {
            let mut c = cx(&doc, &selection, &paint);
            c.zoom = zoom;
            let on = Point::new(60.0 + 4.0 / zoom, 40.0);
            assert_eq!(cursor(&c, on), Some(Cursor::ResizeH), "zoom {zoom}");
            assert!(SplitDrag::hit(&c, Point::new(60.0 + 6.0 / zoom, 40.0), Mods::default()).is_none(), "zoom {zoom}");
            assert!(SplitDrag::hit(&c, on, Mods { alt: true, ..Default::default() }).is_none());
            let mut drag = SplitDrag::hit(&c, on, Mods::default()).unwrap();
            assert!(drag.drag(&c, on + vectorcraft_geom::Vec2::new(1.0 / zoom, 0.0)).is_empty(), "under the drag threshold");
            assert!(drag.drag(&c, Point::new(f64::NAN, 0.0)).is_empty());
            let out = drag.drag(&c, Point::new(85.0, 300.0));
            assert!(
                matches!(&out[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "imageTrace.setView" && p["split"] == json!(0.75) && p["view"] == "split"),
                "{out:?}"
            );
            let out = drag.drag(&c, Point::new(-50.0, 30.0));
            assert!(matches!(&out[..], [Action::Preview(_, p)] if p["split"] == json!(0.0)), "{out:?}");
            assert!(drag.adjusting());
            assert!(matches!(drag.clone().finish()[..], [Action::Commit]));
            assert!(matches!(drag.cancel()[..], [Action::Cancel]));
        }
    }

    /// With Show Center on, a press on the center point where the divider crosses it moves the
    /// object; the rest of the divider still takes a press.
    #[test]
    fn the_center_point_keeps_its_drag_on_the_divider() {
        use vectorcraft_doc::{Appearance, LiveShape};
        let paint = paint();
        let mut selection = Selection::default();
        let (mut doc, id) = traced(Affine::translate((10.0, 20.0)), "split", 0.5);
        let xf = Affine::translate((10.0, 20.0));
        let shape = LiveShape::Rectangle { w: 100.0, h: 50.0, radii: [0.0; 4], kinds: Default::default(), radii_relative: false, xf };
        let path = Node::path(doc.alloc_id(), shape.to_path(), Appearance::default_art());
        doc.insert(Some(id), 1, path).unwrap();
        doc.node_mut(id).unwrap().edit_attrs(|a| a.show_center = Some(true));
        selection.set([id]);
        let c = cx(&doc, &selection, &paint);
        assert_eq!(crate::select::hit_center(&c, Point::new(60.0, 45.0), c.point_tol()).map(|(i, _)| i), Some(id));
        assert_eq!(cursor(&c, Point::new(60.0, 45.0)), None, "the center point");
        assert!(SplitDrag::hit(&c, Point::new(60.0, 45.0), Mods::default()).is_none(), "the center point");
        assert_eq!(cursor(&c, Point::new(60.0, 33.0)), Some(Cursor::ResizeH));
    }
}
