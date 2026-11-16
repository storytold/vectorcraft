//! Radial Repeat's on-canvas handles: a handle below the repeat adds or removes instances, a
//! handle on the circle the instances sit on sets its radius, and a splitter on the circle hides
//! instances at the end of the ring while the others keep their places, as the public "Create
//! radial repeats" page describes. Drawing and hit testing share the document-space geometry of
//! [`RadialHandles::of`]; a drag previews the existing Repeat Options command and commits as one
//! undo step.
//!
//! A split ring is kept in the repeat's existing options: `instances` counts the instances shown,
//! and `endAngle` places them a full ring's step apart from `startAngle` (one step past it for a
//! single instance). The full ring's slot count is read back from that step, so dragging the
//! splitter forward shows the hidden instances again.
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, OrientedBox, RepeatKind, Selection};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::bbox::Handle;
use crate::{Action, Cursor, Mods, ToolContext};

/// The radius of the instances, radius and splitter handles (screen points).
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
    /// On the circle, half a step past the last instance shown: drag it back around the circle to
    /// hide the instances it passes, forward to show them again.
    Splitter,
}

/// The full ring a radial repeat's instances sit on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ring {
    /// The instances a full turn holds at the repeat's step.
    slots: u32,
    /// Degrees from one instance to the next (negative when the instances run the other way).
    step: f64,
}

impl Ring {
    /// The ring of `instances` laid out from `start` to `end` degrees, when their step fits a
    /// whole number of times (2 to 1,000, and no fewer than the instances) in a full turn.
    fn of(instances: u32, start: f64, end: f64) -> Option<Self> {
        let step = instance_step(instances, end - start);
        let slots = 360.0 / step.abs();
        let whole = slots.round();
        ((slots - whole).abs() < 1e-6 && (2.0..=MAX_INSTANCES).contains(&whole) && whole >= f64::from(instances))
            .then(|| Self { slots: whole as u32, step: (360.0 / whole).copysign(step) })
    }

    /// The Repeat Options that show the first `shown` instances of the ring (1 to all of them),
    /// laid out from `start` degrees.
    fn options(&self, shown: u32, start: f64) -> Value {
        let shown = shown.clamp(1, self.slots.max(1));
        let sweep = if shown == self.slots { 360.0_f64.copysign(self.step) } else { self.step * f64::from(shown.saturating_sub(1).max(1)) };
        json!({ "instances": shown, "endAngle": start + sweep })
    }

    /// The instances shown with the splitter `turn` degrees around the ring from the first
    /// instance: those it has passed, one at least and the full ring at most.
    fn shown(&self, turn: f64) -> u32 {
        let k = (turn / self.step.abs()).floor() + 1.0;
        if k.is_nan() { 1 } else { k.clamp(1.0, f64::from(self.slots)) as u32 }
    }
}

/// Degrees between neighboring instances, as `repeat_transforms` spaces `instances` over `sweep`.
fn instance_step(instances: u32, sweep: f64) -> f64 {
    let full = (sweep.abs() - 360.0).abs() < 1e-9;
    if full || instances <= 1 { sweep / f64::from(instances.max(1)) } else { sweep / f64::from(instances - 1) }
}

/// `v` turned by `deg` degrees as `repeat_transforms` turns the instances.
fn turned(v: Vec2, deg: f64) -> Vec2 {
    (Affine::rotate(deg.to_radians()) * v.to_point()).to_vec2()
}

/// The direction of `v` in counter-clockwise screen degrees.
fn screen_angle(v: Vec2) -> f64 {
    (-v.y).atan2(v.x).to_degrees()
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
    start_angle: f64,
    ring: Option<Ring>,
    pub instances_at: Point,
    pub radius_at: Point,
    /// Where the splitter is: none when the instances' step does not divide a full turn, for a
    /// single instance to a turn, or with a radius of 0.
    pub splitter_at: Option<Point>,
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
        let first = turned(d.normalize(), start_angle);
        let step = instance_step(instances, end_angle - start_angle);
        // Halfway between the first two instances; opposite a single one.
        let out = turned(first, if instances == 1 { 180.0 } else { step / 2.0 });
        let radius = radius.max(0.0);
        let ring = Ring::of(instances, start_angle, end_angle);
        // Half a step past the last instance shown.
        let splitter_at = ring.filter(|_| radius > 0.0).map(|r| center + turned(first, r.step * (f64::from(instances) - 0.5)) * radius);
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
            start_angle,
            ring,
            instances_at: Point::new(frame.center().x, frame.y1 + HANDLE_GAP / zoom),
            radius_at: center + out * radius,
            splitter_at: splitter_at.filter(|p| finite(*p)),
        };
        (finite(h.instances_at) && finite(h.radius_at) && h.step.is_finite() && h.step > 0.0).then_some(h)
    }

    /// The controls with where they are.
    pub fn parts(&self) -> Vec<(RadialPart, Point)> {
        let mut parts = vec![(RadialPart::Instances, self.instances_at), (RadialPart::Radius, self.radius_at)];
        parts.extend(self.splitter_at.map(|at| (RadialPart::Splitter, at)));
        parts
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
    /// none when the drag is too far to measure. The splitter's options are those it shows at the
    /// press.
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
            RadialPart::Splitter => self.ring.map(|r| r.options(self.instances, self.start_angle)),
        }
    }

    /// Degrees around the ring from the first instance to the splitter, in the direction the
    /// instances run.
    fn splitter_turn(&self) -> f64 {
        self.ring.map_or(0.0, |r| (f64::from(self.instances) - 0.5) * r.step.abs())
    }

    /// The direction of `p` from the center in degrees, as the instances are turned; none at the
    /// center.
    fn bearing(&self, p: Point) -> Option<f64> {
        let v = p - self.center;
        (finite(p) && v.hypot() > 1e-9).then(|| v.y.atan2(v.x).to_degrees())
    }

    /// The direction (counter-clockwise degrees) in which the radius handle moves the circle out.
    fn out_angle(&self) -> f64 {
        screen_angle(self.out)
    }

    /// The direction (counter-clockwise degrees) along the circle at the splitter.
    fn splitter_angle(&self) -> f64 {
        self.splitter_at.map_or(0.0, |at| screen_angle(at - self.center) + 90.0)
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
/// the line from the center for the radius handle, and along the circle for the splitter.
fn part_cursor(cx: &ToolContext, h: &RadialHandles, part: RadialPart) -> Cursor {
    match part {
        RadialPart::Instances => crate::select::handle_cursor(Handle::Bottom, -cx.view_angle()),
        RadialPart::Radius => crate::select::handle_cursor(Handle::Right, h.out_angle() - cx.view_angle()),
        RadialPart::Splitter => crate::select::handle_cursor(Handle::Right, h.splitter_angle() - cx.view_angle()),
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
    /// For the splitter: its degrees around the ring from the first instance, and the pointer's
    /// last direction from the center.
    turn: f64,
    bearing: Option<f64>,
}

impl RadialDrag {
    pub fn hit(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt {
            return None;
        }
        let (handles, part) = RadialHandles::hit(cx, p)?;
        let from = handles.options(part, Vec2::ZERO)?;
        Some(Self { handles, part, start: p, from, last: None, turn: handles.splitter_turn(), bearing: handles.bearing(p) })
    }

    /// Whether the drag has changed the repeat (and so previews it).
    pub fn adjusting(&self) -> bool {
        self.last.is_some()
    }

    pub fn cursor(&self, cx: &ToolContext) -> Cursor {
        part_cursor(cx, &self.handles, self.part)
    }

    /// The options of the splitter with the pointer at `p`: it follows the pointer around the
    /// circle, each move counted by the shorter way round.
    fn split(&mut self, p: Point) -> Option<Value> {
        let ring = self.handles.ring?;
        if let Some(b) = self.handles.bearing(p) {
            if let Some(last) = self.bearing {
                self.turn += ((b - last + 180.0).rem_euclid(360.0) - 180.0) * ring.step.signum();
            }
            self.bearing = Some(b);
        }
        Some(ring.options(ring.shown(self.turn), self.handles.start_angle))
    }

    /// Preview the options the pointer at `p` gives, when they change; the first change opens
    /// the undo step.
    pub fn drag(&mut self, p: Point) -> Vec<Action> {
        if !finite(p) {
            return vec![];
        }
        let v = match self.part {
            RadialPart::Splitter => self.split(p),
            part => self.handles.options(part, p - self.start),
        };
        let Some(v) = v else { return vec![] };
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

    /// The point on the 100 pt circle about (150, 250) at `deg` degrees clockwise from straight up.
    fn on_circle(deg: f64) -> Point {
        let a = deg.to_radians();
        Point::new(150.0 + 100.0 * a.sin(), 250.0 - 100.0 * a.cos())
    }

    /// Apply the `instances` and `endAngle` of a preview to the repeat.
    fn apply(doc: &mut Document, selection: &Selection, v: &Value) {
        let n = v["instances"].as_u64().unwrap() as u32;
        let end = v["endAngle"].as_f64().unwrap();
        set(doc, selection, |i, _, _, e| (*i, *e) = (n, end));
    }

    /// Where the repeat's instances are, in paint order.
    fn transforms(doc: &Document, selection: &Selection) -> Vec<Affine> {
        let NodeKind::Repeat(spec) = &doc.node(selection.objects[0]).unwrap().kind else { panic!("a repeat") };
        spec.transforms(Some(doc))
    }

    /// Drag the splitter from `from` to `to` degrees clockwise from straight up, 5° a move, and
    /// return the last preview.
    fn sweep(d: &mut RadialDrag, from: f64, to: f64) -> Option<Value> {
        let mut last = None;
        let moves = ((to - from).abs() / 5.0).ceil() as i32;
        for k in 1..=moves {
            if let Some(v) = preview(&d.drag(on_circle(from + (to - from) * f64::from(k) / f64::from(moves)))) {
                last = Some(v.clone());
            }
        }
        last
    }

    #[test]
    fn the_handles_sit_below_the_repeat_and_on_its_circle_at_every_zoom() {
        let (doc, selection) = radial();
        // Halfway between the first instance (straight up from the center) and the second, 45°
        // clockwise from it.
        let on_circle = on_circle(22.5);
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

    #[test]
    fn the_splitter_sits_half_a_step_past_the_last_instance_shown() {
        let (mut doc, selection) = radial();
        // A full ring of eight: between the last instance (315°) and the first.
        for zoom in [0.02, 1.0, 4.0] {
            let h = RadialHandles::of(&doc, &selection, None, zoom).unwrap();
            assert!(near(h.splitter_at.unwrap(), on_circle(337.5)), "{zoom}: {:?}", h.splitter_at);
            assert!(h.parts().contains(&(RadialPart::Splitter, h.splitter_at.unwrap())));
        }
        // Five of the eight shown: past the fifth (180°).
        set(&mut doc, &selection, |n, _, _, e| (*n, *e) = (5, 180.0));
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert!(near(h.splitter_at.unwrap(), on_circle(202.5)), "{:?}", h.splitter_at);
        // The instances running the other way from 90°: three of six, the splitter at -60 - 30.
        set(&mut doc, &selection, |n, _, a, e| (*n, *a, *e) = (3, 90.0, -30.0));
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert!(near(h.splitter_at.unwrap(), on_circle(-60.0)), "{:?}", h.splitter_at);
        // One of the eight shown: half a step past it, and the radius handle opposite it.
        set(&mut doc, &selection, |n, _, a, e| (*n, *a, *e) = (1, 0.0, 45.0));
        let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert!(near(h.splitter_at.unwrap(), on_circle(22.5)), "{:?}", h.splitter_at);
        assert!(near(h.radius_at, on_circle(180.0)), "{:?}", h.radius_at);
    }

    #[test]
    fn no_splitter_where_the_ring_cannot_be_split() {
        let (mut doc, selection) = radial();
        // A single instance to a full turn, no turn at all, a step that does not divide a full
        // turn, and a radius of 0.
        for (n, end, r) in [(1, 360.0, 100.0), (2, 0.0, 100.0), (3, 100.0, 100.0), (8, 360.0, 0.0), (8, f64::MAX, 100.0)] {
            set(&mut doc, &selection, |i, rad, _, e| (*i, *rad, *e) = (n, r, end));
            let h = RadialHandles::of(&doc, &selection, None, 1.0).unwrap();
            assert_eq!(h.splitter_at, None, "{n} over {end}° at {r}");
            assert!(!h.parts().iter().any(|(p, _)| *p == RadialPart::Splitter));
        }
        // Two instances to a turn can be split.
        set(&mut doc, &selection, |i, rad, _, e| (*i, *rad, *e) = (2, 100.0, 360.0));
        assert!(RadialHandles::of(&doc, &selection, None, 1.0).unwrap().splitter_at.is_some());
    }

    #[test]
    fn dragging_the_splitter_hides_instances_and_the_others_keep_their_places() {
        let (mut doc, selection) = radial();
        let full = transforms(&doc, &selection);
        assert_eq!(full.len(), 8);
        let paint = paint();
        let (v, mut d) = {
            let c = cx(&doc, &selection, &paint);
            let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), 1.0).unwrap();
            let mut d = RadialDrag::hit(&c, h.splitter_at.unwrap(), Mods::default()).unwrap();
            assert!(cursor(&c, h.splitter_at.unwrap()).is_some());
            // Back to 320°, short of the last instance: nothing changes.
            assert_eq!(sweep(&mut d, 337.5, 320.0), None);
            assert!(!d.adjusting());
            // Back past the last instance (315°): seven shown, the full ring's step apart.
            let out = d.drag(on_circle(300.0));
            assert!(matches!(out.first(), Some(Action::Begin(l)) if l == "Repeat Options"), "{out:?}");
            let v = preview(&out).unwrap().clone();
            assert_eq!(v, json!({"instances": 7, "endAngle": 270.0}));
            (v, d)
        };
        apply(&mut doc, &selection, &v);
        let seven = transforms(&doc, &selection);
        assert_eq!(seven.len(), 7);
        for (k, (a, b)) in seven.iter().zip(&full).enumerate() {
            assert!(a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9), "instance {k} kept its place");
        }
        // Further back, past 0°: one shown at least.
        assert_eq!(sweep(&mut d, 300.0, -200.0), Some(json!({"instances": 1, "endAngle": 45.0})));
        // Forward again, past the start of the ring: all eight, a full turn.
        assert_eq!(sweep(&mut d, -200.0, 400.0), Some(json!({"instances": 8, "endAngle": 360.0})));
        assert!(matches!(d.finish().as_slice(), [Action::Cancel]), "back where it began: no undo step");

        // From the seven shown, forward past 315°: the eighth shows again where it was.
        let c = cx(&doc, &selection, &paint);
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), 1.0).unwrap();
        assert!(near(h.splitter_at.unwrap(), on_circle(292.5)));
        let mut d = RadialDrag::hit(&c, h.splitter_at.unwrap(), Mods::default()).unwrap();
        let v = sweep(&mut d, 292.5, 330.0).unwrap();
        assert_eq!(v, json!({"instances": 8, "endAngle": 360.0}));
        assert!(matches!(d.finish().as_slice(), [Action::Commit]));
        let mut d = RadialDrag::hit(&c, h.splitter_at.unwrap(), Mods::default()).unwrap();
        assert!(d.drag(h.center).is_empty(), "the center gives no direction");
        assert_eq!(sweep(&mut d, 292.5, 250.0), Some(json!({"instances": 6, "endAngle": 225.0})));
        assert!(matches!(d.cancel().as_slice(), [Action::Cancel]));
        apply(&mut doc, &selection, &v);
        assert_eq!(transforms(&doc, &selection), full);
    }

    #[test]
    fn the_instances_handle_spreads_a_split_ring_over_the_same_arc() {
        let (mut doc, selection) = radial();
        // Seven of eight shown: the handle sets the count alone, as for any partial repeat.
        set(&mut doc, &selection, |n, _, _, e| (*n, *e) = (7, 270.0));
        let paint = paint();
        let c = cx(&doc, &selection, &paint);
        let h = RadialHandles::of(&doc, &selection, crate::select::selection_box(&c), 1.0).unwrap();
        let mut d = RadialDrag::hit(&c, h.instances_at, Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(h.instances_at + Vec2::new(0.0, 32.0))), Some(&json!({"instances": 9})));
        assert!(matches!(d.finish().as_slice(), [Action::Commit]));
    }
}
