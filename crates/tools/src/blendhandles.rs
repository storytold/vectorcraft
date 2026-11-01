//! The on-canvas controls of a selected live blend: the step-count handle on the bbox edge (dragging it
//! down or up sets the Specified Steps count, on an exponential scale — 1 at the bottom, 1000 at the top
//! — and the handle rests where the current count lands instead of always at the top), the Ease knob
//! at the blend's midpoint (its drag sets the Type and the Ramp) and the Colour handle under the spine.
//! The Selection tool and the Blend tool hit-test through here, and the canvas draws the handles white
//! with a dark outline, like the other on-canvas handles (`ui-egui::canvas::tool_overlays`).

use serde_json::{Value, json};
use vectorcraft_doc::live::{BlendEase, BlendEaseType, BlendSpec, Spine, blend_key_stops, blend_step_count};
use vectorcraft_doc::{Document, NodeId, NodeKind, OrientedBox, Selection};
use vectorcraft_geom::{Point, Vec2};

use crate::{Action, Cursor, Mods, ToolContext};

/// Radius of a handle, in screen points (the grid repeat handles').
pub const HANDLE_RADIUS: f32 = 6.0;

/// Gap between the selection's bounding box and the step-count handle, in screen points.
const HANDLE_GAP: f64 = 22.0;

/// How far the pointer reaches for a handle, in screen points.
const REACH: f64 = 8.0;

/// The largest Specified Steps count, where dragging above the box tops out.
const STEPS_MAX: f64 = 1000.0;

/// Which handle of a blend a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendPart {
    /// The step count, on the bounding box's right edge.
    Steps,
    /// The easing Type and Ramp together: dragging toward a key eases toward it.
    Ease,
    /// The easing of the colours, independent of the positions (Shift).
    Color,
}

/// The handles of one live blend, in document space.
#[derive(Clone, Debug)]
pub struct BlendHandles {
    pub id: NodeId,
    /// Where the step-count handle sits for the blend's count now.
    pub steps_at: Point,
    /// The blend's step count now (the first pair's), the top of the steps scale.
    pub steps_now: u32,
    /// The right edge the steps handle slides along: its bottom end, the unit vector up the edge
    /// and its length (the box's height). The box's own frame, so the handle keeps its place on
    /// the box when it rotates.
    edge_base: Point,
    /// The direction up the edge, in document space (the chevrons follow it).
    pub edge_up: Vec2,
    edge_out: Vec2,
    edge_gap: f64,
    edge_h: f64,
    /// The spine the steps run along, with each key's arc length on it.
    spine: Spine,
    stops: Vec<f64>,
    /// The easing the knob's drag starts from (a symmetric Type is left as it is).
    ease: BlendEase,
    /// The box's own angle, counter-clockwise degrees, for the step knob's cursor.
    frame_angle: f64,
    easing: Option<EasingHandles>,
}

/// Where the easing handles sit: the Ease knob on the spine and the Colour handle on the spine's
/// other side.
#[derive(Clone, Copy, Debug)]
pub struct EasingHandles {
    pub ease: Point,
    pub color: Point,
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
        let frame_angle = if frame.angle.is_finite() { frame.angle } else { 0.0 };
        let to_doc = frame.to_doc();
        let (rect, h) = (frame.rect, frame.rect.height());
        // The handles sit where the steps go: on the spine when it is followed, else on the
        // straight lines between the key centres. A replaced spine hidden by Spine Never must not
        // stretch the distance the steps are spaced by (blend_expand measures the straight line).
        let straight;
        let spec: &BlendSpec = if spec.spine_followed() {
            spec
        } else {
            straight = BlendSpec { spine: None, ..spec.as_ref().clone() };
            &straight
        };
        let (spine, stops) = blend_key_stops(children, spec)?;
        if !stops.iter().all(|s| s.is_finite()) {
            return None;
        }
        // The right edge of the box on the page: from its bottom corner up, with the outward
        // normal the handle hangs off. A flat box (two horizontal lines) has no height: give it a
        // screen-sized fallback edge at the spine's right end, so its handles stay reachable.
        let flat = !(h.is_finite() && h > 1e-6);
        let (edge_base, edge_up, edge_out, edge_h) = if flat {
            let (end, _) = spine.at(1.0);
            let up = to_doc * Point::new(0.0, -1.0) - to_doc * Point::ZERO;
            let out = to_doc * Point::new(1.0, 0.0) - to_doc * Point::ZERO;
            let half = 12.0 / zoom;
            (end - up * half, up, out, half * 2.0)
        } else {
            let base = to_doc * Point::new(rect.x1, rect.y1);
            // The unit vectors up the edge and off it, from the corners (a rotation keeps lengths).
            let up = (to_doc * Point::new(rect.x1, rect.y0) - base) / h;
            let out = to_doc * Point::new(rect.x1 + 1.0, rect.y1) - base;
            (base, up, out, h)
        };
        let gap = HANDLE_GAP / zoom;
        // The count handle rests at the height that matches the blend's count on the fixed
        // 1..1000 scale (bottom is 1, top is 1000), so a blend with few steps sits low and a
        // blend near 1000 sits at the top. A drag reads the pointer's height on the edge, and
        // the handle stays where the released count lands instead of snapping back.
        let pair_len = match (stops.first(), stops.get(1)) {
            (Some(a), Some(b)) => (b - a).abs(),
            _ => spine.length(),
        };
        let steps_now = match (children.first(), children.get(1)) {
            (Some(a), Some(b)) => blend_step_count(a, b, spec.spacing, pair_len).clamp(1, STEPS_MAX as usize) as u32,
            _ => 1,
        };
        // The steps scale runs from 1 at the box's bottom to 1000 at its top on an exponential
        // scale, so the handle's height shows the count now. Dragging above the box keeps the
        // knob at the 1000 cap; releasing leaves it at the count's position.
        let steps_height = (steps_now as f64).ln() / STEPS_MAX.ln();
        let steps_at = steps_place(edge_base, edge_up, edge_out, gap, edge_h, steps_height.clamp(0.0, 1.0));
        // The Ease knob sits at the shape's halfway point under the easing, and the Colour handle
        // sits at the colour's halfway point on the spine's other side. The easing moves the steps
        // between any two keys (spine or not), so these show in every spine mode.
        let total = spine.length();
        let easing = (total > 1e-9).then(|| {
            let (a, b) = (stops.first().copied().unwrap_or(0.0) / total, stops.get(1).copied().unwrap_or(total) / total);
            let (ease, angle) = spine.at(a + (b - a) * ease_half(spec.ease.kind, spec.ease.ramp));
            let (shift, _) = spine.at(a + (b - a) * spec.color_position(0.5));
            EasingHandles { ease, color: shift + Vec2::new(-angle.sin(), angle.cos()) * (14.0 / zoom) }
        });
        Some(Self {
            id,
            steps_at,
            steps_now,
            edge_base,
            edge_up,
            edge_out,
            edge_gap: gap,
            edge_h,
            spine,
            stops,
            ease: spec.ease,
            frame_angle,
            easing,
        })
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
        blends_of_selection(doc, selection).iter().filter_map(|id| Self::for_blend(doc, *id, doc.oriented_bounds(&[*id], false), zoom)).collect()
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
    /// the exponential scale from 1 at the bottom to 1000 at the top.
    fn steps_params(&self, p: Point) -> Value {
        json!({ "spacing": "steps", "steps": self.steps_count_at(p) })
    }

    /// The easing Type and Ramp the pointer at `p` gives: the pair of keys it is between, and
    /// where in that pair — toward a key bunches the steps toward it. A symmetric Type (In Out,
    /// Out In) has no halfway point to drag: it keeps its Type and Ramp.
    fn ease_params(&self, p: Point) -> Value {
        let local = self.local_at(p);
        let (kind, ramp) = match self.ease.kind {
            BlendEaseType::InOut | BlendEaseType::OutIn => (self.ease.kind, self.ease.ramp),
            _ => ease_at(local),
        };
        json!({ "easingType": kind.id(), "ramp": ramp * 100.0 })
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

    /// The count the pointer at `p` gives on the steps scale: 1 at the box's bottom, 1000 at
    /// its top, exponential in between. Dragging above the box clamps to the 1000 cap.
    fn steps_count_at(&self, p: Point) -> u32 {
        let h = self.steps_height_at(p).min(1.0);
        let n = STEPS_MAX.powf(h);
        n.round().clamp(1.0, STEPS_MAX) as u32
    }

    /// Where the steps handle sits for the pointer at `p`: on the edge, at the pointer's height
    /// (so the knob follows it while it drags, and stays at the count's position when let go).
    pub fn steps_point_at(&self, p: Point) -> Point {
        steps_place(self.edge_base, self.edge_up, self.edge_out, self.edge_gap, self.edge_h, self.steps_height_at(p))
    }

    /// How many box heights up the edge `p` sits, clamped to the scale's reach.
    fn steps_height_at(&self, p: Point) -> f64 {
        ((p - self.edge_base).dot(self.edge_up) / self.edge_h).clamp(0.0, 1.0)
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

/// Where the Ease knob rests for `kind` at `ramp`: the shape's halfway point between two keys.
/// The inverse for In and Out is [`ease_at`]; In Out and Out In rest at the middle.
fn ease_half(kind: BlendEaseType, ramp: f64) -> f64 {
    let half = match kind {
        BlendEaseType::In => 0.25,
        BlendEaseType::Out => 0.75,
        BlendEaseType::InOut | BlendEaseType::OutIn => 0.5,
    };
    0.5 + ramp.clamp(0.0, 1.0) * (half - 0.5)
}

/// The easing Type and Ramp whose halfway point sits at `local` (0..1) between two keys, so the
/// knob rests where it is dropped: Ease In bunches the steps toward the first key, Ease Out
/// toward the second; Ramp 1 is the strongest either way.
fn ease_at(local: f64) -> (BlendEaseType, f64) {
    let local = local.clamp(0.0, 1.0);
    let ramp = (4.0 * (local - 0.5).abs()).clamp(0.0, 1.0);
    if local <= 0.5 { (BlendEaseType::In, ramp) } else { (BlendEaseType::Out, ramp) }
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
        if mods.cmd || mods.ctrl || mods.alt || !cx.bounding_box || !finite(p) {
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

    fn new(handles: BlendHandles, part: BlendPart, _start: Point) -> Self {
        // Compare against the handle's resting value, not the press point (which can be a few
        // points off): dragging back to the handle then leaves no undo step.
        let e = handles.easing_handles();
        let from = match part {
            BlendPart::Steps => handles.steps_params(handles.steps_at),
            BlendPart::Ease => handles.ease_params(e.map_or(handles.steps_at, |e| e.ease)),
            BlendPart::Color => handles.color_params(e.map_or(handles.steps_at, |e| e.color)),
        };
        Self { handles, part, from, last: None, label: None }
    }

    /// Which blend and handle this drag moves.
    pub fn grab(&self) -> (NodeId, BlendPart) {
        (self.handles.id, self.part)
    }

    /// Which blend and handle this drag moves, and where the handle is now: the step knob
    /// follows the pointer while it drags; the others sit where the blend's options put them.
    pub fn handle(&self) -> (NodeId, BlendPart, Point) {
        let at = match self.part {
            BlendPart::Steps => self.label.as_ref().map_or(self.handles.steps_at, |(p, _)| self.handles.steps_point_at(*p)),
            BlendPart::Ease | BlendPart::Color => {
                let Some(e) = self.handles.easing_handles() else { return (self.handles.id, self.part, self.handles.steps_at) };
                match self.part {
                    BlendPart::Ease => e.ease,
                    _ => e.color,
                }
            }
        };
        (self.handles.id, self.part, at)
    }

    /// Whether the drag has changed the blend (and so previews it).
    pub fn adjusting(&self) -> bool {
        self.last.is_some()
    }

    pub fn cursor(&self, cx: &ToolContext) -> Cursor {
        match self.part {
            BlendPart::Steps => crate::select::handle_cursor(crate::bbox::Handle::Bottom, self.handles.frame_angle - cx.view_angle()),
            BlendPart::Ease | BlendPart::Color => Cursor::Move,
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
        BlendPart::Ease => {
            format!("{} {}%", ease_name(v["easingType"].as_str().unwrap_or("easeIn")), v["ramp"].as_f64().unwrap_or_default().round())
        }
        BlendPart::Color => {
            let shift = v["colorShift"].as_f64().unwrap_or_default();
            format!("Shift {}%", if shift < 0.0 { format!("-{}", (-shift).round()) } else { format!("{}", shift.round()) })
        }
    }
}

/// The plain name of an easing Type, as the drag label reads it.
fn ease_name(id: &str) -> &'static str {
    match id {
        "easeOut" => "Ease Out",
        "easeInOut" => "Ease In Out",
        "easeOutIn" => "Ease Out In",
        _ => "Ease In",
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
    use crate::select::SelectionTool;
    use crate::testutil::{cx, paint};
    use crate::{PointerEvent, PointerKind, Tool};
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, live::BlendSpacing, live::BlendSpineMode};
    use vectorcraft_geom::{Affine, PathData, Rect, SubPath, shapes};

    /// Two 20 pt squares blended (boxes 0..20 and 100..120 on x): the blend's box spans
    /// (0, 0)..(120, 20), its spine (10, 10)..(110, 10).
    fn blend_with(spec: BlendSpec) -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let key = |id: NodeId, x: f64| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), Appearance::default_art()));
        let (k1, k2, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let blend = Node::new(b, NodeKind::Blend { children: vec![key(k1, 0.0), key(k2, 100.0)], spec: Box::new(spec) });
        d.insert(Some(l), 0, blend).unwrap();
        (d, b)
    }

    fn blend(mode: BlendSpineMode, spacing: BlendSpacing) -> (Document, NodeId) {
        blend_with(BlendSpec { spacing, spine_mode: mode, ..Default::default() })
    }

    /// Two horizontal 20 × 0 lines at y = 10 (x 0..20 and 100..120): the blend's box has no
    /// height, but its spine runs 100 pt from (10, 10) to (110, 10).
    fn flat_blend() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let key = |id: NodeId, x: f64| {
            let path = PathData::single(SubPath::polyline(&[Point::new(x, 10.0), Point::new(x + 20.0, 10.0)], false));
            Arc::new(Node::path(id, path, Appearance::default_art()))
        };
        let (k1, k2, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let spec = BlendSpec { spacing: BlendSpacing::Steps(4), spine_mode: BlendSpineMode::Always, ..Default::default() };
        let blend = Node::new(b, NodeKind::Blend { children: vec![key(k1, 0.0), key(k2, 100.0)], spec: Box::new(spec) });
        d.insert(Some(l), 0, blend).unwrap();
        (d, b)
    }

    #[test]
    fn a_flat_blend_keeps_its_handles() {
        let (d, b) = flat_blend();
        let h = BlendHandles::for_blend(&d, b, None, 1.0).expect("a flat blend still has handles");
        assert!(h.steps_at.is_finite(), "{:?}", h.steps_at);
        assert!(h.easing_handles().is_some(), "the Ease and Colour handles still show");
        // The fallback edge is 24 screen points tall at the spine's right end, so the knob is
        // reachable and a drag still sets a count.
        assert!((h.edge_h - 24.0).abs() < 1e-9, "{}", h.edge_h);
        assert!(h.steps_params(Point::new(h.steps_at.x, h.steps_at.y - 10.0))["steps"].as_u64().unwrap_or(0) > 4);
    }

    #[test]
    fn the_ease_knob_and_its_drag_map_round_trip() {
        for (kind, ramp, local) in [
            (BlendEaseType::In, 1.0, 0.25),
            (BlendEaseType::In, 0.5, 0.375),
            (BlendEaseType::In, 0.0, 0.5),
            (BlendEaseType::Out, 0.5, 0.625),
            (BlendEaseType::Out, 1.0, 0.75),
        ] {
            assert!((ease_half(kind, ramp) - local).abs() < 1e-9, "{kind:?} {ramp}: {}", ease_half(kind, ramp));
            assert_eq!(ease_at(local), (kind, ramp));
        }
        // The knob rests where the drag reads it back, so the first move does not snap the ramp.
        let (d, b) = blend_with(BlendSpec {
            ease: BlendEase { kind: BlendEaseType::In, ramp: 0.5 },
            spine_mode: BlendSpineMode::Always,
            ..Default::default()
        });
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        let e = h.easing_handles().unwrap();
        assert!(e.ease.distance(Point::new(10.0 + 100.0 * 0.375, 10.0)) < 1e-6, "{:?}", e.ease);
        assert_eq!(h.ease_params(e.ease), json!({"easingType": "easeIn", "ramp": 50.0}));
    }

    #[test]
    fn a_symmetric_ease_keeps_its_type_when_dragged() {
        for kind in [BlendEaseType::InOut, BlendEaseType::OutIn] {
            let (d, b) = blend_with(BlendSpec { ease: BlendEase { kind, ramp: 0.5 }, spine_mode: BlendSpineMode::Always, ..Default::default() });
            let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
            let e = h.easing_handles().unwrap();
            let at = Point::new(e.ease.x + 2.0, e.ease.y);
            assert_eq!(h.ease_params(at), json!({"easingType": kind.id(), "ramp": 50.0}));
            let mut drag = BlendDrag::new(h, BlendPart::Ease, e.ease);
            assert!(drag.drag(at).is_empty(), "{kind:?} changed");
            assert_eq!(drag.finish(), vec![]);
        }
    }

    #[test]
    fn a_click_off_a_step_knob_changes_nothing() {
        let (d, b) = blend(BlendSpineMode::Always, BlendSpacing::Steps(4));
        let mut s = Selection::default();
        s.set([b]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        let at = h.steps_at + Vec2::new(0.0, 4.0);
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y)).is_empty());
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Up, at.x, at.y));
        assert!(out.is_empty(), "a click without a drag must not commit: {out:?}");
    }

    #[test]
    fn a_blend_handle_drag_is_not_a_transform() {
        let (d, b) = blend(BlendSpineMode::Always, BlendSpacing::Steps(4));
        let mut s = Selection::default();
        s.set([b]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        let mut t = SelectionTool::default();
        assert!(!t.transforming());
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, h.steps_at.x, h.steps_at.y));
        assert!(!t.transforming());
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, h.steps_at.x, h.steps_at.y - 5.0));
        assert!(!t.transforming(), "a blend-handle drag keeps the selection box");
    }

    #[test]
    fn blend_handles_stay_live_when_edges_are_hidden() {
        let (d, b) = blend(BlendSpineMode::Always, BlendSpacing::Steps(4));
        let mut s = Selection::default();
        s.set([b]);
        let p = paint();
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        let hidden_edges = ToolContext { show_bbox: false, bounding_box: true, ..cx(&d, &s, &p) };
        assert!(BlendDrag::hit(&hidden_edges, h.steps_at, Mods::default()).is_some());
        let hidden_box = ToolContext { show_bbox: false, bounding_box: false, ..cx(&d, &s, &p) };
        assert!(BlendDrag::hit(&hidden_box, h.steps_at, Mods::default()).is_none());
    }

    #[test]
    fn a_rotated_blend_keeps_its_knob_on_its_own_edge() {
        let (mut d, b) = blend(BlendSpineMode::Always, BlendSpacing::Steps(4));
        let turn = Affine::translate((60.0, 10.0)) * Affine::rotate(-30.0_f64.to_radians()) * Affine::translate((-60.0, -10.0));
        d.node_mut(b).unwrap().transform(turn, false);
        let frame = d.oriented_bounds(&[b], false).unwrap();
        assert!((frame.angle - 30.0).abs() < 1e-9, "{frame:?}");
        let mut s = Selection::default();
        s.set([b]);
        let handles = BlendHandles::of_selection(&d, &s, 1.0);
        let h = handles.first().expect("the blend has handles");
        let expected_up = frame.to_doc() * Point::new(0.0, -1.0) - frame.to_doc() * Point::ZERO;
        assert!((h.edge_up - expected_up).hypot() < 1e-9, "{:?} vs {expected_up:?}", h.edge_up);
        let p = paint();
        let c = cx(&d, &s, &p);
        let drag = BlendDrag::new(h.clone(), BlendPart::Steps, h.steps_at);
        assert_eq!(drag.cursor(&c), crate::select::handle_cursor(crate::bbox::Handle::Bottom, 30.0));
    }

    #[test]
    fn the_steps_handle_rests_at_the_count_height_on_the_edge() {
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Never, BlendSpacing::Steps(3));
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        assert_eq!(h.steps_now, 3);
        // The handle rests at the height matching 3 on the fixed 1..1000 scale: low on the box,
        // not pinned at the top corner.
        assert!(h.steps_at.y > 10.0, "handle should sit low for few steps, got {:?}", h.steps_at);
        assert!(h.steps_at.distance(Point::new(142.0, 0.0)) > 5.0, "not at the top corner: {:?}", h.steps_at);
        // The scale: bottom 1, middle ~32, top 1000; above the box caps at 1000.
        assert_eq!(h.steps_params(Point::new(142.0, 20.0))["steps"], 1);
        assert_eq!(h.steps_params(Point::new(142.0, 10.0))["steps"], 32);
        assert_eq!(h.steps_params(Point::new(142.0, 0.0))["steps"], 1000);
        assert_eq!(h.steps_params(Point::new(142.0, -10.0))["steps"], 1000);
        assert_eq!(h.steps_params(Point::new(142.0, -20.0))["steps"], 1000);
        // The knob follows the pointer up the edge.
        assert!(h.steps_point_at(Point::new(142.0, 10.0)).distance(Point::new(142.0, 10.0)) < 1e-6);
        // A count of 100 sits two thirds of the way up (log scale).
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Never, BlendSpacing::Steps(100));
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        assert_eq!(h.steps_now, 100);
        assert_eq!(h.steps_params(Point::new(142.0, 20.0))["steps"], 1);
        assert_eq!(h.steps_params(Point::new(142.0, 10.0))["steps"], 32);
        assert_eq!(h.steps_params(Point::new(142.0, 0.0))["steps"], 1000);
        assert_eq!(h.steps_params(Point::new(142.0, -10.0))["steps"], 1000);
        // A Smooth Color blend shows its own count (50) at its height.
        let (d, b) = blend(vectorcraft_doc::live::BlendSpineMode::Never, BlendSpacing::SmoothColor);
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        assert_eq!(h.steps_now, 50);
        assert_eq!(h.steps_params(Point::new(142.0, 0.0))["steps"], 1000);
        assert_eq!(h.steps_params(Point::new(142.0, 20.0))["steps"], 1);
    }

    #[test]
    fn a_hidden_replaced_spine_does_not_stretch_the_distance_spacing() {
        // The keys' centres are 100 apart, but the replaced spine detours far away. Spine Never
        // spaces the steps by the straight line (one step at 50 pt), so the knob reads 1, not the
        // count the hidden spine's arc length would give.
        let spine = PathData::single(SubPath::polyline(&[Point::new(10.0, 10.0), Point::new(10.0, 1000.0), Point::new(110.0, 10.0)], false));
        let spec = BlendSpec { spacing: BlendSpacing::Distance(50.0), spine_mode: BlendSpineMode::Never, spine: Some(spine), ..Default::default() };
        let (d, b) = blend_with(spec);
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        assert_eq!(h.steps_now, 1, "the straight line between the centres, not the hidden spine");
    }

    #[test]
    fn the_ease_handle_sets_type_and_ramp() {
        let (d, b) = blend(BlendSpineMode::Always, BlendSpacing::Steps(3));
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        // Toward the second key: Ease Out; toward the first: Ease In; Ramp 1 at the keys.
        assert_eq!(ease_at(0.75), (BlendEaseType::Out, 1.0));
        assert_eq!(ease_at(0.25), (BlendEaseType::In, 1.0));
        // With the default Ease In at Ramp 0, the knob is at the spine's middle and the colour
        // handle 14 pt below.
        let e = h.easing_handles().unwrap();
        assert!(e.ease.distance(Point::new(60.0, 10.0)) < 1e-6, "{:?}", e.ease);
        assert!(e.color.distance(Point::new(60.0, 24.0)) < 1e-6, "{:?}", e.color);
    }

    #[test]
    fn handles_hit_and_the_drag_previews_the_options() {
        let (d, b) = blend(BlendSpineMode::Always, BlendSpacing::Steps(3));
        let mut s = Selection::default();
        s.set([b]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let h = BlendHandles::for_blend(&d, b, None, 1.0).unwrap();
        // The handle rests at its count's height on the fixed 1..1000 scale, so rest there.
        let rest = h.steps_at.y;
        // The steps handle at its count height, the ease knob on the spine and the colour handle
        // below.
        for (at, part) in
            [(Point::new(142.0, rest), BlendPart::Steps), (Point::new(60.0, 10.0), BlendPart::Ease), (Point::new(60.0, 24.0), BlendPart::Color)]
        {
            assert_eq!(BlendDrag::hit_any(&c, at, Mods::default()).unwrap().grab(), (b, part), "{part:?}");
        }
        // The Selection tool reaches the same handles on the blend selected whole.
        assert_eq!(BlendDrag::hit(&c, Point::new(142.0, rest), Mods::default()).unwrap().grab(), (b, BlendPart::Steps));
        assert_eq!(BlendDrag::hit(&c, Point::new(60.0, 10.0), Mods::default()).unwrap().grab(), (b, BlendPart::Ease));
        // Dragging the steps handle up from its count height goes past the count now (3): the
        // knob rides at the pointer's height and the undo step opens with the first change.
        let mut drag = BlendDrag::hit_any(&c, Point::new(142.0, rest), Mods::default()).unwrap();
        assert!(drag.drag(Point::new(142.0, rest)).is_empty(), "no change yet");
        let acts = drag.drag(Point::new(142.0, -10.0));
        assert!(
            matches!(&acts[..], [Action::Begin(name), Action::Preview(id, p)]
                if name == "Blend Options" && id == "object.blend.options"
                    && p["spacing"] == json!("steps") && p["steps"] == json!(1000)),
            "{acts:?}"
        );
        assert_eq!(drag.label(), Some((Point::new(142.0, -10.0), "1000".into())));
        let (id, part, at) = drag.handle();
        assert_eq!((id, part), (b, BlendPart::Steps));
        // Dragging above the box clamps the knob to the top (count 1000).
        assert!(at.distance(Point::new(142.0, 0.0)) < 1e-6, "{at:?}");
        assert_eq!(drag.finish(), vec![Action::Commit]);
        // Dragging the Ease knob toward the second key eases out.
        let mut drag = BlendDrag::hit_any(&c, Point::new(60.0, 10.0), Mods::default()).unwrap();
        let acts = drag.drag(Point::new(80.0, 10.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options" && p["easingType"] == json!("easeOut")
                    && (p["ramp"].as_f64().unwrap_or_default() - 80.0).abs() < 0.1),
            "{acts:?}"
        );
        assert_eq!(drag.label(), Some((Point::new(80.0, 10.0), "Ease Out 80%".into())));
        assert_eq!(drag.finish(), vec![Action::Commit]);
        // The colour shift leans the colours late.
        let mut drag = BlendDrag::hit_any(&c, Point::new(60.0, 24.0), Mods::default()).unwrap();
        let acts = drag.drag(Point::new(70.0, 24.0));
        assert!(
            matches!(&acts[..], [Action::Begin(_), Action::Preview(id, p)]
                if id == "object.blend.options" && (p["colorShift"].as_f64().unwrap_or_default() + 22.02).abs() < 0.01),
            "{acts:?}"
        );
        assert_eq!(drag.label(), Some((Point::new(70.0, 24.0), "Shift -22%".into())));
        assert_eq!(drag.finish(), vec![Action::Commit]);
        // Pressing a few points off the steps handle and dragging back to it leaves no undo step:
        // the drag compares against the handle's own value, not the press point.
        let mut drag = BlendDrag::hit_any(&c, Point::new(142.0, rest + 3.0), Mods::default()).unwrap();
        assert!(!drag.drag(Point::new(142.0, rest + 3.0)).is_empty(), "the off-centre press reads another count");
        drag.drag(Point::new(142.0, rest));
        assert_eq!(drag.finish(), vec![Action::Cancel]);
        // A drag that ends where it began leaves no undo step.
        let mut drag = BlendDrag::hit_any(&c, Point::new(142.0, rest), Mods::default()).unwrap();
        drag.drag(Point::new(142.0, rest));
        assert_eq!(drag.finish(), vec![]);
    }
}
