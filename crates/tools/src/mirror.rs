//! Mirror Repeat's on-canvas axis. Drawing and hit testing share document-space geometry;
//! gestures preview the existing Repeat Options command and commit as one undo step.
use serde_json::json;
use vectorcraft_doc::{Document, NodeId, NodeKind, Selection, pattern::RepeatKind};
use vectorcraft_geom::{Point, Vec2};

use crate::{Action, Cursor, Mods, ToolContext};

pub const HANDLE_RADIUS: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct MirrorAxis {
    pub id: NodeId,
    pub center: Point,
    pub angle: f64,
    pub ends: [Point; 2],
}

impl MirrorAxis {
    /// A single editable mirror repeat selected whole. Multi-selection keeps ordinary transforms.
    pub fn of(doc: &Document, selection: &Selection, zoom: f64) -> Option<Self> {
        let [id] = selection.objects.as_slice() else { return None };
        if !selection.anchors.is_empty() || !selection.segments.is_empty() || !doc.is_editable(*id) || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let n = doc.node(*id)?;
        let NodeKind::Repeat(spec) = &n.kind else { return None };
        let RepeatKind::Mirror { angle, center } = spec.kind else { return None };
        if !angle.is_finite() || !finite(center) {
            return None;
        }
        let bounds = n.geometric_bounds_with(&doc.symbol_boxes())?;
        let (sin, cos) = angle.to_radians().sin_cos();
        let direction = Vec2::new(cos, sin);
        let mut extent = 0.0_f64;
        for p in
            [Point::new(bounds.x0, bounds.y0), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)]
        {
            let along = (p - center).dot(direction);
            if !along.is_finite() {
                return None;
            }
            extent = extent.max(along.abs());
        }
        // Keep the center move handle separate from both rotation handles, even on tiny artwork.
        extent = (extent + 24.0 / zoom).max(36.0 / zoom);
        let ends = [center - direction * extent, center + direction * extent];
        ends.iter().all(|p| finite(*p)).then_some(Self { id: *id, center, angle, ends })
    }

    fn hit(cx: &ToolContext, p: Point) -> Option<(Self, bool)> {
        if !cx.show_bbox || !finite(p) {
            return None;
        }
        let axis = Self::of(cx.doc, cx.selection, cx.zoom)?;
        if axis.ends.iter().any(|q| q.distance(p) <= cx.tol(9.0)) {
            return Some((axis, true));
        }
        // The center is its own move handle. Keep it ahead of bounding-box handles when the two
        // happen to overlap; the rest of the axis line yields to those handles below.
        if axis.center.distance(p) <= cx.tol(7.0) {
            return Some((axis, false));
        }
        let d = axis.ends[1] - axis.ends[0];
        let length = d.hypot();
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        let unit = d / length;
        let along = (p - axis.ends[0]).dot(unit).clamp(0.0, length);
        ((axis.ends[0] + unit * along).distance(p) <= cx.tol(7.0) && !crate::select::bbox_handle_at(cx, p)).then_some((axis, false))
    }
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

pub fn cursor(cx: &ToolContext, p: Point) -> Option<Cursor> {
    let (axis, rotate) = MirrorAxis::hit(cx, p)?;
    Some(if rotate { Cursor::rotate_about_in_view(axis.center, p, cx.view_angle()) } else { Cursor::Move })
}

#[derive(Clone, Debug)]
pub struct MirrorDrag {
    axis: MirrorAxis,
    start: Point,
    rotate: bool,
    began: bool,
}

impl MirrorDrag {
    pub fn hit(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt {
            return None;
        }
        let (axis, rotate) = MirrorAxis::hit(cx, p)?;
        Some(Self { axis, start: p, rotate, began: false })
    }

    pub fn adjusting(&self) -> bool {
        self.began
    }

    pub fn cursor(&self, cx: &ToolContext, p: Point) -> Cursor {
        if self.rotate { Cursor::rotate_about_in_view(self.axis.center, p, cx.view_angle()) } else { Cursor::Move }
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point, mods: Mods) -> Vec<Action> {
        if !finite(p) || (!self.began && p.distance(self.start) < cx.tol(3.0)) {
            return vec![];
        }
        let (center, angle) = if self.rotate {
            let from = self.start - self.axis.center;
            let to = p - self.axis.center;
            if to.hypot() < cx.tol(2.0) {
                return vec![];
            }
            let mut angle = self.axis.angle + (to.y.atan2(to.x) - from.y.atan2(from.x)).to_degrees();
            if mods.shift {
                angle = (angle / 45.0).round() * 45.0;
            }
            (self.axis.center, angle.rem_euclid(360.0))
        } else {
            (self.axis.center + (p - self.start), self.axis.angle)
        };
        if !finite(center) || !angle.is_finite() {
            return vec![];
        }
        let mut out = vec![];
        if !self.began {
            self.began = true;
            out.push(Action::Begin("Repeat Options".into()));
        }
        out.push(Action::Preview("object.repeat.options".into(), json!({"center": [center.x, center.y], "angle": angle})));
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
    use crate::testutil::{cx, doc_with_rect, paint};
    use std::sync::Arc;
    use vectorcraft_doc::pattern::RepeatSpec;

    #[test]
    fn mirror_axis_hit_targets_match_zoom_and_reject_invalid_geometry() {
        let (mut doc, id) = doc_with_rect();
        let mut source = doc.node(id).unwrap().clone();
        source.id = doc.alloc_id();
        doc.node_mut(id).unwrap().kind = NodeKind::Repeat(RepeatSpec::mirror(vec![Arc::new(source)], 90.0, 10.0));
        let mut selection = Selection::default();
        selection.set([id]);
        let paint = paint();
        for zoom in [0.1, 1.0, 10.0] {
            let mut cx = cx(&doc, &selection, &paint);
            cx.zoom = zoom;
            let axis = MirrorAxis::of(&doc, &selection, zoom).unwrap();
            assert_eq!(cursor(&cx, axis.center), Some(Cursor::Move));
            for end in axis.ends {
                assert!(MirrorAxis::hit(&cx, end).unwrap().1);
            }
            assert!(MirrorAxis::hit(&cx, axis.center + Vec2::new(6.0 / zoom, 0.0)).is_some());
            assert!(MirrorAxis::hit(&cx, axis.center + Vec2::new(10.0 / zoom, 0.0)).is_none());
            let mut drag = MirrorDrag::hit(&cx, axis.center, Mods::default()).unwrap();
            assert!(drag.drag(&cx, Point::new(f64::NAN, 0.0), Mods::default()).is_empty());
            assert!(!drag.adjusting());
            cx.show_bbox = false;
            assert!(MirrorAxis::hit(&cx, axis.center).is_none());
        }
        for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(MirrorAxis::of(&doc, &selection, zoom).is_none());
        }
        let NodeKind::Repeat(spec) = &mut doc.node_mut(id).unwrap().kind else { panic!() };
        spec.kind = RepeatKind::Mirror { angle: f64::NAN, center: Point::ZERO };
        assert!(MirrorAxis::of(&doc, &selection, 1.0).is_none());
    }
}
