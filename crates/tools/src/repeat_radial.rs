//! Radial Repeat's on-canvas handles: a handle below the repeat adds or removes instances, and a
//! handle on the circle the instances sit on sets its radius, as the public "Create radial
//! repeats" page describes. Drawing and hit testing share the document-space geometry of
//! [`RadialHandles::of`]; a drag previews the existing Repeat Options command and commits as one
//! undo step.
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, OrientedBox, RepeatKind, Selection};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::Handle;
use crate::{Action, Cursor, Mods, ToolContext};

/// The radius of the instances and radius handles (screen points).
pub const HANDLE_RADIUS: f32 = 5.0;

/// How far (screen points) the instances handle sits below the bounding box, clear of its own
/// handles and rotate zones.
pub const HANDLE_GAP: f64 = 22.0;

/// How far (screen points) the instances handle is dragged for each instance it adds or removes.
pub const INSTANCE_STEP: f64 = 16.0;

/// How near (screen points) a press must come to a handle to take it.
const REACH: f64 = 8.0;

/// The most instances a radial repeat has (as `object.repeat.options` accepts them).
const MAX_INSTANCES: f64 = 1000.0;

/// A control of [`RadialHandles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialPart {
    /// Below the repeat: drag down for more instances, up for fewer.
    Instances,
    /// On the circle, between the first two instances: drag outward for a larger radius, inward
    /// for a smaller one.
    Radius,
}

/// The controls of a selected radial repeat, in document space.
#[derive(Clone, Copy, Debug)]
pub struct RadialHandles {
    pub id: NodeId,
    instances: u32,
    /// The circle the instances' centers sit on.
    pub center: Point,
    pub radius: f64,
    /// The unit direction from the center to the radius handle.
    out: Vec2,
    /// Document points of drag for each instance.
    step: f64,
    pub instances_at: Point,
    pub radius_at: Point,
}

impl RadialHandles {
    /// The controls of a single editable radial repeat selected whole, the instances handle placed
    /// below `frame` (the bounding box the Selection tool shows; its geometric bounds without one)
    /// at `zoom`.
    pub fn of(doc: &Document, selection: &Selection, frame: Option<OrientedBox>, zoom: f64) -> Option<Self> {
        let [id] = selection.objects.as_slice() else { return None };
        if !selection.anchors.is_empty() || !selection.segments.is_empty() || !doc.is_editable(*id) || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let n = doc.node(*id)?;
        let NodeKind::Repeat(spec) = &n.kind else { return None };
        let RepeatKind::Radial { instances, radius, center, start_angle, end_angle, .. } = spec.kind else { return None };
        if ![radius, center.x, center.y, start_angle, end_angle].iter().all(|v| v.is_finite()) {
            return None;
        }
        let source = spec.source_bounds()?;
        if !finite(source.center()) {
            return None;
        }
        // As `repeat_transforms` lays the instances out: the first one at `start_angle` from the
        // source's own direction, the rest a step apart.
        let instances = instances.clamp(1, 1000);
        let mut d = source.center() - center;
        if d.hypot() < 1e-9 {
            d = Vec2::new(0.0, -1.0);
        }
        let sweep = end_angle - start_angle;
        let full = (sweep.abs() - 360.0).abs() < 1e-9;
        let step = if full || instances == 1 { sweep / f64::from(instances) } else { sweep / f64::from(instances - 1) };
        // Halfway between the first two instances (opposite the only one of a full turn).
        let out = (Affine::rotate((start_angle + step / 2.0).to_radians()) * d.normalize().to_point()).to_vec2();
        let radius = radius.max(0.0);
        let frame = match frame {
            Some(f) => aabb(f.corners())?,
            None => n.geometric_bounds_with(&doc.symbol_boxes())?,
        };
        let h = Self {
            id: *id,
            instances,
            center,
            radius,
            out,
            step: INSTANCE_STEP / zoom,
            instances_at: Point::new(frame.center().x, frame.y1 + HANDLE_GAP / zoom),
            radius_at: center + out * radius,
        };
        (finite(h.instances_at) && finite(h.radius_at) && h.step.is_finite() && h.step > 0.0).then_some(h)
    }

    /// Both controls with where they are.
    pub fn parts(&self) -> [(RadialPart, Point); 2] {
        [(RadialPart::Instances, self.instances_at), (RadialPart::Radius, self.radius_at)]
    }

    /// The control under `p` (the nearer within reach) and the repeat's controls, unless a
    /// bounding-box handle is there, which keeps its scaling.
    fn hit(cx: &ToolContext, p: Point) -> Option<(Self, RadialPart)> {
        if !cx.show_bbox || !finite(p) {
            return None;
        }
        let h = Self::of(cx.doc, cx.selection, crate::select::selection_box(cx), cx.zoom)?;
        let reach = cx.tol(REACH);
        let (part, _) =
            h.parts().into_iter().map(|(part, q)| (part, q.distance(p))).filter(|(_, d)| *d <= reach).min_by(|a, b| a.1.total_cmp(&b.1))?;
        (!crate::select::bbox_handle_at(cx, p)).then_some((h, part))
    }

    /// The Repeat Options that dragging `part` by `d` (document points) from the press gives;
    /// none when the drag is too far to measure.
    fn options(&self, part: RadialPart, d: Vec2) -> Option<Value> {
        match part {
            RadialPart::Instances => {
                let n = f64::from(self.instances) + (d.y / self.step).round();
                // NaN only comes from an infinite drag; the clamp keeps a huge one at the limit.
                (!n.is_nan()).then(|| json!({ "instances": n.clamp(1.0, MAX_INSTANCES) as u32 }))
            }
            RadialPart::Radius => {
                // Only the part of the drag along the line from the center moves the circle.
                let r = ((self.radius + d.dot(self.out)).max(0.0) * 100.0).round() / 100.0;
                r.is_finite().then(|| json!({ "radius": r }))
            }
        }
    }

    /// The direction (counter-clockwise degrees) in which the radius handle moves the circle out.
    fn out_angle(&self) -> f64 {
        (-self.out.y).atan2(self.out.x).to_degrees()
    }
}

/// The page-square bounds of `corners`.
fn aabb(corners: [Point; 4]) -> Option<Rect> {
    let r = corners.iter().skip(1).fold(Rect::from_points(corners[0], corners[0]), |r, p| r.union_pt(*p));
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then_some(r)
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The resize cursor of `part`, turned with the view: up and down for the instances handle, along
/// the line from the center for the radius handle.
fn part_cursor(cx: &ToolContext, h: &RadialHandles, part: RadialPart) -> Cursor {
    match part {
        RadialPart::Instances => crate::select::handle_cursor(Handle::Bottom, -cx.view_angle()),
        RadialPart::Radius => crate::select::handle_cursor(Handle::Right, h.out_angle() - cx.view_angle()),
    }
}

/// The cursor over a radial repeat's control at `p`, if one is there.
pub fn cursor(cx: &ToolContext, p: Point) -> Option<Cursor> {
    let (h, part) = RadialHandles::hit(cx, p)?;
    Some(part_cursor(cx, &h, part))
}

/// A drag of one of a radial repeat's controls.
#[derive(Clone, Debug)]
pub struct RadialDrag {
    handles: RadialHandles,
    part: RadialPart,
    start: Point,
    /// The options the press found, and the last ones previewed.
    from: Value,
    last: Option<Value>,
}

impl RadialDrag {
    pub fn hit(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt {
            return None;
        }
        let (handles, part) = RadialHandles::hit(cx, p)?;
        let from = handles.options(part, Vec2::ZERO)?;
        Some(Self { handles, part, start: p, from, last: None })
    }

    /// Whether the drag has changed the repeat (and so previews it).
    pub fn adjusting(&self) -> bool {
        self.last.is_some()
    }

    pub fn cursor(&self, cx: &ToolContext) -> Cursor {
        part_cursor(cx, &self.handles, self.part)
    }

    /// Preview the options the pointer at `p` gives, when they change; the first change opens
    /// the undo step.
    pub fn drag(&mut self, p: Point) -> Vec<Action> {
        if !finite(p) {
            return vec![];
        }
        let Some(v) = self.handles.options(self.part, p - self.start) else { return vec![] };
        if self.last.as_ref().unwrap_or(&self.from) == &v {
            return vec![];
        }
        let mut out = vec![];
        if self.last.is_none() {
            out.push(Action::Begin("Repeat Options".into()));
        }
        out.push(Action::Preview("object.repeat.options".into(), v.clone()));
        self.last = Some(v);
        out
    }

    /// Commit the drag; one that ends where it began leaves no undo step.
    pub fn finish(self) -> Vec<Action> {
        match self.last {
            Some(v) if v == self.from => vec![Action::Cancel],
            Some(_) => vec![Action::Commit],
            None => vec![],
        }
    }

    pub fn cancel(self) -> Vec<Action> {
        if self.last.is_some() { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cx, doc_with_rect, paint};
    use std::sync::Arc;
    use vectorcraft_doc::pattern::RepeatSpec;

    /// [`doc_with_rect`]'s 100 × 100 square at (100, 100) as a radial repeat of eight instances
    /// with a radius of 100 pt about (150, 250), selected. The instances span (0, 100)–(300, 400).
    fn radial() -> (Document, Selection) {
        let (mut doc, id) = doc_with_rect();
        let mut source = doc.node(id).unwrap().clone();
        source.id = doc.alloc_id();
        doc.node_mut(id).unwrap().kind = NodeKind::Repeat(RepeatSpec::radial(vec![Arc::new(source)], 8, Some(100.0)));
        let mut selection = Selection::default();
        selection.set([id]);
        (doc, selection)
    }

    fn set(doc: &mut Document, selection: &Selection, f: impl FnOnce(&mut u32, &mut f64, &mut f64, &mut f64)) {
        let NodeKind::Repeat(spec) = &mut doc.node_mut(selection.objects[0]).unwrap().kind else { panic!("a repeat") };
        let RepeatKind::Radial { instances, radius, start_angle, end_angle, .. } = &mut spec.kind else { panic!("a radial repeat") };
        f(instances, radius, start_angle, end_angle);
    }

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-6
    }

    fn preview(out: &[Action]) -> Option<&Value> {
        out.iter().find_map(|a| match a {
            Action::Preview(cmd, v) if cmd == "object.repeat.options" => Some(v),
            _ => None,
        })
    }

    #[test]
    fn the_handles_sit_below_the_repeat_and_on_its_circle_at_every_zoom() {
        let (doc, selection) = radial();
        // Halfway between the first instance (straight up from the center) and the second, 45°
        // clockwise from it.
        let half = 22.5_f64.to_radians();
        let on_circle = Point::new(150.0 + 100.0 * half.sin(), 250.0 - 100.0 * half.cos());
        for zoom in [0.02, 1.0, 4.0] {
            let h = RadialHandles::of(&doc, &selection, None, zoom).unwrap();
            assert!(near(h.instances_at, Point::new(150.0, 400.0 + HANDLE_GAP / zoom)), "{zoom}: {:?}", h.instances_at);
            assert!(near(h.radius_at, on_circle), "{zoom}: {:?}", h.radius_at);
            assert_eq!((h.center, h.radius), (Point::new(150.0, 250.0), 100.0));
        }
        // Placed below the bounding box the Selection tool shows.
        let frame = OrientedBox::aligned(Rect::new(-10.0, 90.0, 310.0, 410.0));
        let h = RadialHandles::of(&doc, &selection, Some(frame), 1.0).unwrap();
        assert!(near(h.instances_at, Point::new(150.0, 410.0 + HANDLE_GAP)));
    }

    #[test]
    fn the_radius_handle_follows_the_instances_angles() {
        let (mut doc, selection) = radial();
        // One instance: opposite it.
        set(&mut doc, &selection, |n, _, _, _| *n = 1);
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert!(near(h.radius_at, Point::new(150.0, 350.0)), "{:?}", h.radius_at);
        // Three instances over a quarter turn from 90°: between those at 90° and 135°.
        set(&mut doc, &selection, |n, _, a, b| (*n, *a, *b) = (3, 90.0, 180.0));
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        let a = 112.5_f64.to_radians();
        assert!(near(h.radius_at, Point::new(150.0 + 100.0 * a.sin(), 250.0 - 100.0 * a.cos())), "{:?}", h.radius_at);
        // A radius of 0: at the center, still apart from the instances handle.
        set(&mut doc, &selection, |_, r, _, _| *r = 0.0);
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert!(near(h.radius_at, Point::new(150.0, 250.0)));
        assert!(h.radius_at.distance(h.instances_at) > 2.0 * REACH);
    }

    #[test]
    fn no_handles_without_a_single_valid_radial_repeat() {
        let (mut doc, mut selection) = radial();
        for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(RadialHandles::of(&doc, &selection, None, zoom).is_none(), "{zoom}");
        }
        let paint = paint();
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        {
            let mut c = cx(&doc, &selection, &paint);
            assert!(cursor(&c, h.instances_at).is_some());
            c.show_bbox = false;
            assert!(cursor(&c, h.instances_at).is_none(), "the bounding box is hidden");
        }
        let id = selection.objects[0];
        doc.node_mut(id).unwrap().locked = true;
        assert!(RadialHandles::of(&doc, &selection, None, 1.0).is_none(), "a locked repeat");
        doc.node_mut(id).unwrap().locked = false;
        doc.node_mut(id).unwrap().visible = false;
        assert!(RadialHandles::of(&doc, &selection, None, 1.0).is_none(), "a hidden repeat");
        doc.node_mut(id).unwrap().visible = true;
        for bad in [f64::NAN, f64::INFINITY] {
            set(&mut doc, &selection, |_, r, _, _| *r = bad);
            assert!(RadialHandles::of(&doc, &selection, None, 1.0).is_none(), "radius {bad}");
            set(&mut doc, &selection, |_, r, a, _| (*r, *a) = (100.0, bad));
            assert!(RadialHandles::of(&doc, &selection, None, 1.0).is_none(), "start angle {bad}");
            set(&mut doc, &selection, |_, _, a, _| *a = 0.0);
        }
        let other = doc.alloc_id();
        let mut second = doc.node(id).unwrap().clone();
        second.id = other;
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), 1, second).unwrap();
        selection.set([id, other]);
        assert!(RadialHandles::of(&doc, &selection, None, 1.0).is_none(), "two objects selected");
        selection.set([id]);
        let NodeKind::Repeat(spec) = &mut doc.node_mut(id).unwrap().kind else { panic!("a repeat") };
        spec.kind = RepeatKind::Grid {
            h_spacing: 10.0,
            v_spacing: 10.0,
            rows: 3,
            cols: 3,
            grid_type: Default::default(),
            flip_rows: false,
            flip_cols: false,
        };
        assert!(RadialHandles::of(&doc, &selection, None, 1.0).is_none(), "a grid repeat");
    }

    #[test]
    fn dragging_the_instances_handle_steps_the_count() {
        let (doc, selection) = radial();
        let paint = paint();
        let mut c = cx(&doc, &selection, &paint);
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), 1.0).unwrap();
        let mut d = RadialDrag::hit(&c, h.instances_at, Mods::default()).unwrap();
        assert!(d.drag(h.instances_at + Vec2::new(0.0, 7.0)).is_empty(), "less than half a step changes nothing");
        assert!(!d.adjusting());
        // Two steps down, a little sideways: ten instances, one undo step.
        let out = d.drag(h.instances_at + Vec2::new(5.0, 32.0));
        assert!(matches!(out.first(), Some(Action::Begin(l)) if l == "Repeat Options"), "{out:?}");
        assert_eq!(preview(&out), Some(&json!({"instances": 10})));
        assert!(d.drag(h.instances_at + Vec2::new(0.0, 36.0)).is_empty(), "the same count isn't previewed again");
        assert!(d.adjusting());
        assert!(matches!(d.finish().as_slice(), [Action::Commit]));
        // Far up: one instance at least; far down: 1,000 at most.
        let mut d = RadialDrag::hit(&c, h.instances_at, Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(h.instances_at - Vec2::new(0.0, 900.0))), Some(&json!({"instances": 1})));
        assert_eq!(preview(&d.drag(h.instances_at + Vec2::new(0.0, 1e300))), Some(&json!({"instances": 1000})));
        assert!(d.drag(Point::new(f64::NAN, 0.0)).is_empty(), "a non-finite pointer");
        // Back where it began: no undo step.
        assert_eq!(preview(&d.drag(h.instances_at)), Some(&json!({"instances": 8})));
        assert!(matches!(d.finish().as_slice(), [Action::Cancel]));
        // The step is the same on screen at every zoom: 16 px is a quarter point at 64.
        c.zoom = 64.0;
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), c.zoom).unwrap();
        let mut d = RadialDrag::hit(&c, h.instances_at, Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(h.instances_at - Vec2::new(0.0, 0.25))), Some(&json!({"instances": 7})));
        assert!(matches!(d.cancel().as_slice(), [Action::Cancel]));
        // Alt, Cmd and Ctrl keep the Selection tool's own gestures.
        assert!(RadialDrag::hit(&c, h.instances_at, Mods { alt: true, ..Mods::default() }).is_none());
    }

    #[test]
    fn dragging_the_radius_handle_moves_the_circle_in_and_out() {
        let (doc, selection) = radial();
        let paint = paint();
        let c = cx(&doc, &selection, &paint);
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), 1.0).unwrap();
        let out = (h.radius_at - h.center) / 100.0;
        let across = Vec2::new(-out.y, out.x);
        let mut d = RadialDrag::hit(&c, h.radius_at, Mods::default()).unwrap();
        // 25 pt out and 40 pt across: only the outward part counts.
        let o = d.drag(h.radius_at + out * 25.0 + across * 40.0);
        assert!(matches!(o.first(), Some(Action::Begin(l)) if l == "Repeat Options"), "{o:?}");
        assert_eq!(preview(&o), Some(&json!({"radius": 125.0})));
        assert!(d.drag(h.radius_at + across * 30.0).iter().any(|a| matches!(a, Action::Preview(_, v) if v == &json!({"radius": 100.0}))));
        // Past the center: a radius of 0.
        assert_eq!(preview(&d.drag(h.radius_at - out * 250.0)), Some(&json!({"radius": 0.0})));
        // Too far to measure: nothing previewed.
        assert!(d.drag(Point::new(f64::MAX, -f64::MAX)).is_empty());
        assert!(matches!(d.finish().as_slice(), [Action::Commit]));
        // Escape before any change leaves nothing to undo.
        let d = RadialDrag::hit(&c, h.radius_at, Mods::default()).unwrap();
        assert!(d.cancel().is_empty());
    }

    #[test]
    fn a_bounding_box_handle_wins_where_they_overlap() {
        let (doc, selection) = radial();
        let paint = paint();
        let mut c = cx(&doc, &selection, &paint);
        // Zoomed far out (the 300 pt repeat 6 px across), the radius handle lies on a
        // bounding-box handle, which keeps its scaling; the instances handle stays clear of the box.
        c.zoom = 0.02;
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), c.zoom).unwrap();
        assert!(crate::select::bbox_handle_at(&c, h.radius_at));
        assert!(RadialDrag::hit(&c, h.radius_at, Mods::default()).is_none());
        assert!(cursor(&c, h.radius_at).is_none());
        assert!(!crate::select::bbox_handle_at(&c, h.instances_at));
        assert!(RadialDrag::hit(&c, h.instances_at, Mods::default()).is_some());
        // At full size both are the repeat's, with resize cursors along their drags.
        c.zoom = 1.0;
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), c.zoom).unwrap();
        assert!(RadialDrag::hit(&c, h.radius_at, Mods::default()).is_some());
        assert_eq!(cursor(&c, h.instances_at), Some(Cursor::ResizeV));
        // The radius handle points 22.5° clockwise from straight up: the up-and-down cursor.
        assert_eq!(cursor(&c, h.radius_at), Some(Cursor::ResizeV));
    }
}
