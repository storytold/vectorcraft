//! The on-canvas controls of a selected live blend: the step-count handle at the top of the
//! selection's bounding box's right edge (dragging it down or up sets the Specified Steps count,
//! on an exponential scale — a box height covers 1 to 100 steps and half a box above it reaches
//! 1000 — and it returns to the top when let go), and on the spine the Ease knob at the blend's
//! midpoint with a linked Ramp handle beside it (bezier-handle style: its distance from the knob
//! is the Ramp) and the Colour handle under the spine. The Selection tool and the Blend tool
//! hit-test through here, and the canvas draws the handles white with a dark outline, like the
//! other on-canvas handles (`ui-egui::canvas::tool_overlays`).

use serde_json::{Value, json};
use vectorcraft_doc::live::{BlendEaseType, Spine, blend_key_stops};
use vectorcraft_doc::{Document, NodeId, NodeKind, OrientedBox, Selection};
use vectorcraft_geom::{Point, Vec2};

use crate::{Action, Cursor, Mods, ToolContext};

/// Radius of a handle, in screen points (the grid repeat handles').
pub const HANDLE_RADIUS: f32 = 6.0;

/// Radius of the linked Ramp handle, in screen points.
pub const RAMP_RADIUS: f32 = 4.0;

/// Gap between the selection's bounding box and the step-count handle, in screen points.
const HANDLE_GAP: f64 = 22.0;

/// How far the pointer reaches for a handle, in screen points.
const REACH: f64 = 8.0;

/// Counts per box height on the steps scale: a box height covers 1–100 steps, so small counts get
/// most of the edge to themselves.
const STEPS_PER_BOX_HEIGHT: f64 = 100.0;

/// How far above the box's top the steps scale reaches, in box heights (100^1.5 = 1000): more
/// than the box's range of counts takes dragging the handle above the box.
const STEPS_BOXES_ABOVE: f64 = 0.5;

/// Where the linked Ramp handle rests at Ramp 0, and how far past that it goes at Ramp 100, in
/// screen points from the Ease knob, perpendicular to the spine.
const RAMP_REST: f64 = 10.0;
const RAMP_REACH: f64 = 26.0;

/// Which handle of a blend a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendPart {
    /// The step count, on the bounding box's right edge.
    Steps,
    /// The easing of the steps' positions along the spine (Type and Ramp together: where the
    /// knob sits along the pair is where the eased midpoint lands).
    Ease,
    /// The Ramp alone: how far the linked handle sits from the Ease knob.
    Ramp,
    /// The easing of the colours, independent of the positions (Shift).
    Color,
}

/// The handles of one live blend, in document space.
#[derive(Clone, Debug)]
pub struct BlendHandles {
    pub id: NodeId,
    /// Where the step-count handle sits for the blend's count now.
    pub steps_at: Point,
    /// The right edge the steps handle slides along: its bottom end, the unit vector up the edge
    /// and its length (the box's height). The box's own frame, so the handle keeps its place on
    /// the box when it rotates.
    edge_base: Point,
    edge_up: Vec2,
    edge_h: f64,
    /// The spine the steps run along, with each key's arc length on it.
    spine: Spine,
    stops: Vec<f64>,
    easing: Option<EasingHandles>,
    /// The blend's easing Type (the Ramp handle's drag keeps it).
    ease_kind: BlendEaseType,
}

/// Where the easing handles sit: the Ease knob on the spine, the linked Ramp handle hanging off
/// it perpendicular to the spine, and the Colour handle on the spine's other side.
#[derive(Clone, Copy, Debug)]
pub struct EasingHandles {
    pub ease: Point,
    pub ramp: Point,
    pub color: Point,
    /// The Ramp handle's rest and total reach from the Ease knob, in document points.
    ramp_rest: f64,
    ramp_reach: f64,
}

impl BlendHandles {
    /// The handles of blend `id`, placed on `frame` (the bounding box shown; the blend's own
    /// bounds, square to the page, without one) at `zoom`.
    pub fn for_blend(doc: &Document, id: NodeId, frame: Option<OrientedBox>, zoom: f64) -> Option<Self> {
        if !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let n = doc.node(id)?;
        let NodeKind::Blend { children, spec } = &n.kind else { return None };
        if children.len() < 2 || !doc.is_editable(id) {
            return None;
        }
        let frame = match frame {
            Some(f) => f,
            None => OrientedBox::aligned(n.geometric_bounds_with(&doc.symbol_boxes())?),
        };
        // The right edge of the box on the page: from its bottom corner up, with the outward
        // normal the handle hangs off.
        let (rect, h) = (frame.rect, frame.rect.height());
        if !(h.is_finite() && h > 1e-6) {
            return None;
        }
        let to_doc = frame.to_doc();
        let edge_base = to_doc * Point::new(rect.x1, rect.y1);
        // The unit vectors up the edge and off it, from the corners (a rotation keeps lengths).
        let edge_up = (to_doc * Point::new(rect.x1, rect.y0) - edge_base) / h;
        let edge_out = to_doc * Point::new(rect.x1 + 1.0, rect.y1) - edge_base;
        let gap = HANDLE_GAP / zoom;
        // The count handle rests at the box's top corner and returns there when a drag ends; the
        // drag itself reads the pointer's height on the edge, so counts stay easy to hit.
        let steps_at = steps_place(edge_base, edge_up, edge_out, gap, h, 1.0);
        let (spine, stops) = blend_key_stops(children, spec)?;
        if !stops.iter().all(|s| s.is_finite()) {
            return None;
        }
        // The Ease knob sits at the shape's halfway point under the easing, the linked Ramp
        // handle hangs off it perpendicular to the spine (its distance the Ramp), and the Colour
        // handle sits at the colour's halfway point on the spine's other side. The easing moves
        // the steps between any two keys (spine or not), so these show in every spine mode.
        let (a, b) = (stops[0] / spine.length(), stops.get(1).copied().unwrap_or(stops[0] + spine.length()) / spine.length());
        let (ease, angle) = spine.at(a + (b - a) * spec.ease.apply(0.5));
        let (shift, _) = spine.at(a + (b - a) * spec.color_position(0.5));
        let (rest, reach) = (RAMP_REST / zoom, RAMP_REACH / zoom);
        let easing = Some(EasingHandles {
            ease,
            ramp: ease + Vec2::new(angle.sin(), -angle.cos()) * (rest + spec.ease.ramp.clamp(0.0, 1.0) * (reach - rest)),
            color: shift + Vec2::new(-angle.sin(), angle.cos()) * (14.0 / zoom),
            ramp_rest: rest,
            ramp_reach: reach,
        });
        Some(Self { id, steps_at, edge_base, edge_up, edge_h: h, spine, stops, easing, ease_kind: spec.ease.kind })
    }

    /// The handles of a single blend selected whole (`frame` is the bounding box shown).
    pub fn selected(doc: &Document, selection: &Selection, frame: Option<OrientedBox>, zoom: f64) -> Option<Self> {
        let [id] = selection.objects.as_slice() else { return None };
        if !selection.anchors.is_empty() || !selection.segments.is_empty() {
            return None;
        }
        Self::for_blend(doc, *id, frame, zoom)
    }

    /// Every selected blend with handles, or the blend a selected key belongs to (the Blend tool's).
    pub fn of_selection(doc: &Document, selection: &Selection, zoom: f64) -> Vec<Self> {
        blends_of_selection(doc, selection).iter().filter_map(|id| Self::for_blend(doc, *id, None, zoom)).collect()
    }

    /// Where the easing handles sit: the easing moves the steps between the first two keys in
    /// every spine mode, so they show whenever the blend has a spine to measure them on.
    pub fn easing_handles(&self) -> Option<EasingHandles> {
        self.easing
    }

    /// The whole-spine fraction pair `(from, span)` of the keys `f` (0..1) sits between.
    fn pair_span(&self, f: f64) -> (f64, f64) {
        let total = self.spine.length();
        let at = f * total;
        let k = self.stops.len().max(2);
        let index = self.stops.windows(2).position(|w| at >= w[0] && at <= w[1]).unwrap_or(k - 2);
        let (a, b) = match (self.stops.get(index), self.stops.get(index + 1)) {
            (Some(&a), Some(&b)) if (b - a).abs() > 1e-9 => (a, b),
            _ => (0.0, total / (k - 1).max(1) as f64),
        };
        let span = ((b - a).abs() / total).clamp(1e-6, 1.0);
        ((a / total).clamp(0.0, 1.0 - span), span)
    }

    /// The Specified Steps the steps handle at `p` stands for: its height on the box's edge, on
    /// the exponential scale (the document's 1-1000 range).
    fn steps_params(&self, p: Point) -> Value {
        let n = STEPS_PER_BOX_HEIGHT.powf(self.steps_height_at(p)).round().clamp(1.0, 1000.0) as u32;
        json!({ "spacing": "steps", "steps": n })
    }

    /// The midpoint the pointer at `p` gives: where the blend's halfway step lands between its
    /// first two keys (the Ease knob's place; the Ramp handle shapes how the steps get there).
    fn ease_params(&self, p: Point) -> Value {
        json!({ "midpoint": self.local_at(p) * 100.0 })
    }

    /// The Ramp the linked handle at `p` gives: its distance from the Ease knob, past its rest,
    /// keeping the blend's easing Type.
    fn ramp_params(&self, p: Point) -> Value {
        let Some(e) = self.easing else { return json!({}) };
        let ramp = (e.ease.distance(p) - e.ramp_rest).max(0.0) / (e.ramp_reach - e.ramp_rest);
        json!({ "easingType": self.ease_kind.id(), "ramp": ramp.clamp(0.0, 1.0) * 100.0 })
    }

    /// The colour Shift the pointer at `p` gives, in its pair.
    fn color_params(&self, p: Point) -> Value {
        json!({ "colorShift": shift_at(self.local_at(p)) })
    }

    /// Where `p` sits along its pair of keys, 0..1 (0 and 1 at the keys themselves).
    fn local_at(&self, p: Point) -> f64 {
        let f = nearest_fraction(&self.spine, p);
        let (from, span) = self.pair_span(f);
        ((f - from) / span).clamp(0.0, 1.0)
    }

    /// How many box heights up the edge `p` sits, clamped to the scale's reach.
    fn steps_height_at(&self, p: Point) -> f64 {
        ((p - self.edge_base).dot(self.edge_up) / self.edge_h).clamp(0.0, 1.0 + STEPS_BOXES_ABOVE)
    }
}

/// Where the steps handle sits `heights` box heights up the edge, plus the gap off the edge
/// along `out`.
fn steps_place(base: Point, up: Vec2, out: Vec2, gap: f64, edge_h: f64, heights: f64) -> Point {
    base + up * (edge_h * heights) + out * gap
}

/// The blends of a selection: each selected blend, and the blend each selected key belongs to.
pub fn blends_of_selection(doc: &Document, selection: &Selection) -> Vec<NodeId> {
    let mut ids = vec![];
    for id in &selection.objects {
        let blend = match doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Blend { .. }) => Some(*id),
            _ => doc.parent_of(*id),
        };
        if let Some(b) = blend
            && doc.node(b).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. }))
            && !ids.contains(&b)
        {
            ids.push(b);
        }
    }
    ids
}

/// The handle of `handles` nearest `p`, within `reach`.
pub fn hit_part(handles: &[BlendHandles], p: Point, reach: f64) -> Option<(usize, BlendPart)> {
    let mut best: Option<(usize, BlendPart, f64)> = None;
    for (i, h) in handles.iter().enumerate() {
        let mut take = |part: BlendPart, at: Option<Point>| {
            if let Some(at) = at
                && at.is_finite()
                && let d = at.distance(p)
                && d <= reach
                && best.is_none_or(|(_, _, bd)| d < bd)
            {
                best = Some((i, part, d));
            }
        };
        take(BlendPart::Steps, Some(h.steps_at));
        let Some(e) = h.easing_handles() else { continue };
        take(BlendPart::Ease, Some(e.ease));
        take(BlendPart::Ramp, Some(e.ramp));
        take(BlendPart::Color, Some(e.color));
    }
    best.map(|(i, part, _)| (i, part))
}

/// The fraction (0..1) of the spine nearest `p`, sampled then refined.
fn nearest_fraction(spine: &Spine, p: Point) -> f64 {
    let (mut best, mut best_d) = (0.0, f64::INFINITY);
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..2 {
        let n = 128;
        for i in 0..=n {
            let f = lo + (hi - lo) * i as f64 / n as f64;
            let d = spine.at(f).0.distance(p);
            if d < best_d {
                best_d = d;
                best = f;
            }
        }
        let step = (hi - lo) / n as f64;
        lo = (best - step).max(0.0);
        hi = (best + step).min(1.0);
    }
    best
}

/// The colour Shift whose halfway point sits at `local` (0..1) between two keys.
fn shift_at(local: f64) -> f64 {
    // Shift +-100 reaches 0.5^4 and 0.5^0.25.
    let t = local.clamp(0.0625, 0.5f64.powf(0.25));
    (50.0 * (-t.log2()).log2()).clamp(-100.0, 100.0)
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// A drag of one of a blend's handles.
#[derive(Clone, Debug)]
pub struct BlendDrag {
    handles: BlendHandles,
    part: BlendPart,
    from: Value,
    last: Option<Value>,
    /// Where the drag's label sits and what it says, while it drags.
    label: Option<(Point, String)>,
}

impl BlendDrag {
    /// The handle under `p` of the one blend selected whole, unless a bounding-box handle is
    /// there, which keeps its scaling.
    pub fn hit(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt || !cx.show_bbox || !finite(p) {
            return None;
        }
        let handles = BlendHandles::selected(cx.doc, cx.selection, crate::select::selection_box(cx), cx.zoom)?;
        let (_, part) = hit_part(std::slice::from_ref(&handles), p, cx.tol(REACH))?;
        (!crate::select::bbox_handle_at(cx, p)).then(|| Self::new(handles, part, p))
    }

    /// The handle under `p` of any selected blend (the Blend tool's).
    pub fn hit_any(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt || !finite(p) {
            return None;
        }
        let handles = BlendHandles::of_selection(cx.doc, cx.selection, cx.zoom);
        let (i, part) = hit_part(&handles, p, cx.tol(REACH))?;
        handles.get(i).map(|h| Self::new(h.clone(), part, p))
    }

    fn new(handles: BlendHandles, part: BlendPart, start: Point) -> Self {
        let from = match part {
            BlendPart::Steps => handles.steps_params(start),
            BlendPart::Ease => handles.ease_params(start),
            BlendPart::Ramp => handles.ramp_params(start),
            BlendPart::Color => handles.color_params(start),
        };
        Self { handles, part, from, last: None, label: None }
    }

    /// Which blend and handle this drag moves.
    pub fn grab(&self) -> (NodeId, BlendPart) {
        (self.handles.id, self.part)
    }

    /// Whether the drag has changed the blend (and so previews it).
    pub fn adjusting(&self) -> bool {
        self.last.is_some()
    }

    pub fn cursor(&self, cx: &ToolContext) -> Cursor {
        match self.part {
            BlendPart::Steps => crate::select::handle_cursor(crate::bbox::Handle::Bottom, -cx.view_angle()),
            BlendPart::Ease | BlendPart::Ramp | BlendPart::Color => Cursor::Move,
        }
    }

    /// The drag's label: where it sits and what it says, while it drags.
    pub fn label(&self) -> Option<(Point, String)> {
        self.label.clone()
    }

    /// Preview the options the pointer at `p` gives, when they change; the first change opens
    /// the undo step.
    pub fn drag(&mut self, p: Point) -> Vec<Action> {
        if !finite(p) {
            return vec![];
        }
        let v = match self.part {
            BlendPart::Steps => self.handles.steps_params(p),
            BlendPart::Ease => self.handles.ease_params(p),
            BlendPart::Ramp => self.handles.ramp_params(p),
            BlendPart::Color => self.handles.color_params(p),
        };
        self.label = Some((p, drag_text(self.part, &v)));
        if self.last.as_ref().unwrap_or(&self.from) == &v {
            return vec![];
        }
        let mut out = vec![];
        if self.last.is_none() {
            out.push(Action::Begin("Blend Options".into()));
        }
        out.push(Action::Preview("object.blend.options".into(), v.clone()));
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

/// What the drag's label says beside the pointer.
fn drag_text(part: BlendPart, v: &Value) -> String {
    match part {
        BlendPart::Steps => format!("{}", v["steps"]),
        BlendPart::Ease => format!("Midpoint {}%", v["midpoint"].as_f64().unwrap_or(50.0).round()),
        BlendPart::Ramp => format!("Ease {}%", v["ramp"].as_f64().unwrap_or_default().round()),
        BlendPart::Color => {
            let shift = v["colorShift"].as_f64().unwrap_or_default();
            format!("Shift {}%", if shift < 0.0 { format!("-{}", (-shift).round()) } else { format!("{}", shift.round()) })
        }
    }
}

/// The cursor over a blend handle at `p`, if one is there, for the one blend selected whole.
pub fn cursor(cx: &ToolContext, p: Point) -> Option<Cursor> {
    let d = BlendDrag::hit(cx, p, Mods::default())?;
    Some(d.cursor(cx))
}

/// The cursor over a blend handle at `p` among every selected blend (the Blend tool's).
pub fn cursor_any(cx: &ToolContext, p: Point) -> Option<Cursor> {
    let d = BlendDrag::hit_any(cx, p, Mods::default())?;
    Some(d.cursor(cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cx, paint};
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, live::BlendSpacing};
    use vectorcraft_geom::{Rect, shapes};

    /// Two 20 pt squares blended (boxes 0..20 and 100..120 on x): the blend's box spans
    /// (0, 0)..(120, 20), its spine (10, 10)..(110, 10).
    fn blend(mode: vectorcraft_doc::live::BlendSpineMode, spacing: BlendSpacing) -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let key = |id: NodeId, x: f64| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), Appearance::default_art()));
        let (k1, k2, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let spec = vectorcraft_doc::live::BlendSpec { spacing, spine_mode: mode, ..Default::default() };
        let blend = Node::new(b, NodeKind::Blend { children: vec![key(k1, 0.0), key(k2, 100.0)], spec: Box::new(spec) });
        d.insert(Some(l), 0, blend).unwrap();
        (d, b)
    }

    #[test]
    fn the_steps_handle_rests_at_the_box_top_and_drags_the_count() {
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Never, BlendSpacing::Steps(3));
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        // The handle rests at the box's top corner, 22 pt off its right edge, whatever the count.
        assert!(h.steps_at.distance(Point::new(142.0, 0.0)) < 1e-6, "{:?}", h.steps_at);
        // Its height while dragging is the count, on the exponential scale: halfway up the edge
        // is 10 steps, the top corner 100, half a box above it the 1000 cap, the bottom corner 1.
        assert_eq!(h.steps_params(Point::new(142.0, 15.229))["steps"], 3);
        assert_eq!(h.steps_params(Point::new(142.0, 10.0))["steps"], 10);
        assert_eq!(h.steps_params(Point::new(142.0, 0.0))["steps"], 100);
        assert_eq!(h.steps_params(Point::new(142.0, -10.0))["steps"], 1000);
        assert_eq!(h.steps_params(Point::new(142.0, 30.0))["steps"], 1);
        // A Smooth Color blend still drags to Specified Steps.
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Never, BlendSpacing::SmoothColor);
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        assert!(h.steps_at.distance(Point::new(142.0, 0.0)) < 1e-6, "{:?}", h.steps_at);
        assert_eq!(h.steps_params(Point::new(142.0, 10.0))["steps"], 10);
    }

    #[test]
    fn the_ease_knob_is_the_midpoint_and_the_ramp_hangs_off_it() {
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Always, BlendSpacing::Steps(3));
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        // The knob's place is the midpoint the halfway step lands at; the Ramp handle is the
        // easing's strength, on its own.
        let mid = |x: f64| h.ease_params(Point::new(x, 10.0))["midpoint"].as_f64().unwrap_or_default();
        assert!((mid(80.0) - 70.0).abs() < 0.1, "{}", mid(80.0));
        assert!((mid(40.0) - 30.0).abs() < 0.1, "{}", mid(40.0));
        // With the default midpoint at 0.5, the knob is at the spine's middle, the linked Ramp
        // handle 10 pt above it, and the colour handle 14 pt below.
        let e = h.easing_handles().unwrap();
        assert!(e.ease.distance(Point::new(60.0, 10.0)) < 1e-6, "{:?}", e.ease);
        assert!(e.ramp.distance(Point::new(60.0, 0.0)) < 1e-6, "{:?}", e.ramp);
        assert!(e.color.distance(Point::new(60.0, 24.0)) < 1e-6, "{:?}", e.color);
        // The Ramp handle's distance past its rest is the Ramp.
        assert_eq!(h.ramp_params(Point::new(60.0, 0.0))["ramp"], 0.0);
        assert_eq!(h.ramp_params(Point::new(60.0, -16.0))["ramp"], 100.0);
        assert_eq!(h.ramp_params(Point::new(60.0, -3.0))["ramp"], 18.75);
    }

    #[test]
    fn handles_hit_and_the_drag_previews_the_options() {
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Always, BlendSpacing::Steps(3));
        let mut s = Selection::default();
        s.set([b]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let rest = 0.0;
        // The steps handle at the box's top corner, the ease knob on the spine, the linked Ramp
        // handle above it and the colour handle below.
        for (at, part) in [
            (Point::new(142.0, rest), BlendPart::Steps),
            (Point::new(60.0, 10.0), BlendPart::Ease),
            (Point::new(60.0, 0.0), BlendPart::Ramp),
            (Point::new(60.0, 24.0), BlendPart::Color),
        ] {
            assert_eq!(BlendDrag::hit_any(&c, at, Mods::default()).unwrap().grab(), (b, part), "{part:?}");
        }
        // The Selection tool reaches the same handles on the blend selected whole.
        assert_eq!(BlendDrag::hit(&c, Point::new(142.0, rest), Mods::default()).unwrap().grab(), (b, BlendPart::Steps));
        assert_eq!(BlendDrag::hit(&c, Point::new(60.0, 10.0), Mods::default()).unwrap().grab(), (b, BlendPart::Ease));
        // Dragging the steps handle up adds steps (its height is the count), down takes them
        // away; the label travels beside the pointer and the undo step opens with the first
        // change.
        let mut drag = BlendDrag::hit_any(&c, Point::new(142.0, rest), Mods::default()).unwrap();
        assert!(drag.drag(Point::new(142.0, rest)).is_empty(), "no change yet");
        let acts = drag.drag(Point::new(142.0, 10.0));
        assert!(
            matches!(&acts[..], [Action::Begin(name), Action::Preview(id, p)]
                if name == "Blend Options" && id == "object.blend.options"
                    && p["spacing"] == json!("steps") && p["steps"] == json!(10)),
            "{acts:?}"
        );
        assert_eq!(drag.label(), Some((Point::new(142.0, 10.0), "10".into())));
        assert_eq!(drag.finish(), vec![Action::Commit]);
        // Dragging the linked Ramp handle away from the Ease knob raises the Ramp, keeping the
        // Type.
        let mut drag = BlendDrag::hit_any(&c, Point::new(60.0, 0.0), Mods::default()).unwrap();
        let acts = drag.drag(Point::new(60.0, -16.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options" && p["easingType"] == json!("easeIn") && p["ramp"] == json!(100.0)),
            "{acts:?}"
        );
        assert_eq!(drag.label(), Some((Point::new(60.0, -16.0), "Ease 100%".into())));
        assert_eq!(drag.finish(), vec![Action::Commit]);
        // Dragging the Ease knob to seven tenths of the pair puts the midpoint there.
        let mut drag = BlendDrag::hit_any(&c, Point::new(60.0, 10.0), Mods::default()).unwrap();
        let acts = drag.drag(Point::new(80.0, 10.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options"
                    && (p["midpoint"].as_f64().unwrap_or_default() - 70.0).abs() < 0.1),
            "{acts:?}"
        );
        assert_eq!(drag.label(), Some((Point::new(80.0, 10.0), "Midpoint 70%".into())));
        assert_eq!(drag.finish(), vec![Action::Commit]);
        // The colour shift leans the colours late.
        let mut drag = BlendDrag::hit_any(&c, Point::new(60.0, 24.0), Mods::default()).unwrap();
        let acts = drag.drag(Point::new(70.0, 24.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options" && (p["colorShift"].as_f64().unwrap_or_default() + 22.02).abs() < 0.01),
            "{acts:?}"
        );
        // A drag that ends where it began leaves no undo step.
        let mut drag = BlendDrag::hit_any(&c, Point::new(60.0, 10.0), Mods::default()).unwrap();
        drag.drag(Point::new(60.0, 10.0));
        assert_eq!(drag.finish(), vec![]);
    }
}
