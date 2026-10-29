//! Gradient tool (G) and the gradient annotator.
//!
//! Drag across selected objects to set the gradient vector of the paint behind the active proxy
//! (fill or stroke; type objects' runs), a solid paint becoming the default gradient. With nothing
//! selected, the press targets the object under the
//! pointer; a click inside selected art applies the gradient from that point.
//!
//! The annotator is a bar from a round start handle to a square end handle. Drag the start handle
//! (or the bar) to move the gradient, the end handle to change its length and angle, and just past
//! the end to rotate it. Dragged handles snap to anchors, edges and smart guides; Shift instead
//! constrains the vector to 45° steps from the Constrain Angle preference. Colour stops sit under the bar: click the bar to add
//! one, drag one to move it (Alt drags a copy), drag it off the bar to delete it, and drag a
//! diamond above the bar to move a midpoint. Double-clicking a stop opens its popover. The
//! selected stop (`gradient.selectStop`) is shared with the Gradient and Color panels: Delete or
//! Backspace removes it (never below two stops) and the arrow keys nudge it.
//!
//! A radial gradient also shows its extent: a dashed ellipse around the centre (the start, a
//! ring). Drag the dot on the ellipse across the bar to change the aspect ratio and the ellipse
//! elsewhere to rotate it; the end handle sets the radius. The dot inside the centre ring is the
//! focal point (where the first stop sits): drag it to make the gradient off-centre, back onto
//! the centre to centre it.
//!
//! On a freeform gradient the tool edits its points instead (see `freeform`).

use serde_json::{Value, json};
use vectorcraft_color::gradient::{duplicate_stop, insert_stop, midpoint_from_pos, midpoint_pos, move_stop, remove_stop, set_midpoint};
use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientStop};
use vectorcraft_doc::NodeId;
use vectorcraft_doc::hit::hit_test;
use vectorcraft_geom::{Affine, Point, Shape, Vec2, constrain_angle_from};

use super::freeform::{self, Annotator as FreeformAnnotator};
use super::paint_owner;
use crate::guides::snap_draw;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

const BAR: [u8; 3] = [0x20, 0x20, 0x20];
const LIGHT: [u8; 3] = [0xf0, 0xf0, 0xf0];

// Annotator layout and hit radii, in screen pixels.
/// Start and end handles.
const HANDLE: f64 = 5.0;
/// The rotate zone reaches this far past the end handle.
const ROTATE: f64 = 14.0;
/// Stop chips sit this far below the bar.
const STOP_GAP: f64 = 10.0;
const STOP_HIT: f64 = 6.0;
/// Midpoint diamonds sit this far above the bar.
const MID_GAP: f64 = 7.0;
const MID_HIT: f64 = 5.0;
/// The bar (where a click adds a stop) reaches this far above it, and down through the stop row.
const BAR_HIT: f64 = 4.0;
/// A radial's focal dot, and the centre ring around it (the start handle).
const FOCAL_HIT: f64 = 3.0;
const CENTRE: f64 = 4.5;
const CENTRE_HIT: f64 = 8.0;
/// A stop dragged farther than this from the bar is deleted on release.
const OFF_BAR: f64 = 24.0;
/// A press becomes a drag after this much movement.
const DRAG_START: f64 = 3.0;
/// Arrow-key nudges of the selected stop (Shift: the big step).
const NUDGE: f32 = 0.01;
const NUDGE_BIG: f32 = 0.1;

/// The gradient annotator: the linear or radial gradient behind the active proxy of the first
/// selected object that has one, placed in document coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotator {
    pub geom: GradientGeom,
    pub gradient: Gradient,
}

/// A part of the annotator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Stop(usize),
    /// The midpoint diamond after stop `i`.
    Mid(usize),
    Start,
    End,
    /// Just past the end (or a radial's extent ellipse): rotates the vector about the start.
    Rotate,
    /// A radial's focal point.
    Focal,
    /// The handle on a radial's extent ellipse, across the bar: the aspect ratio.
    Aspect,
    /// The bar at an offset (0..1): a click adds a stop there.
    Bar(f32),
}

impl Annotator {
    pub fn of(cx: &ToolContext) -> Option<Self> {
        cx.selection.objects.iter().find_map(|id| {
            let (g, geom) = cx.doc.node(*id)?.proxy_gradient(!cx.fill_active, cx.appearance_item)?;
            (g.gradient.kind != GradientKind::Freeform).then(|| Self { geom, gradient: g.gradient.clone() })
        })
    }
    /// An annotator showing `stops` on `geom` (the state a drag started from).
    fn with(geom: GradientGeom, stops: &[GradientStop]) -> Self {
        Self { geom, gradient: Gradient::new(GradientKind::Linear, stops.to_vec()) }
    }
    fn radial(&self) -> bool {
        self.gradient.kind == GradientKind::Radial
    }
    /// A radial's aspect handle: on the extent ellipse, across the bar from the stops.
    pub fn aspect_point(&self) -> Point {
        self.geom.start - self.normal() * (self.vector().hypot() * self.geom.aspect)
    }
    /// How far `p` is from a radial's extent ellipse (approximately, along the ray from the
    /// centre).
    fn off_ellipse(&self, p: Point) -> Option<f64> {
        let m = self.geom.unit_frame()?;
        let rho = (m.inverse() * p).to_vec2().hypot();
        (rho > 1e-12).then(|| p.distance(self.geom.start) * (1.0 - 1.0 / rho).abs())
    }
    fn vector(&self) -> Vec2 {
        self.geom.end - self.geom.start
    }
    /// The point at offset `t` along the bar.
    pub fn at(&self, t: f64) -> Point {
        self.geom.start + self.vector() * t
    }
    /// Unit normal to the bar, towards the stop row.
    fn normal(&self) -> Vec2 {
        let v = self.vector();
        let len = v.hypot();
        if len < 1e-12 { Vec2::new(0.0, 1.0) } else { Vec2::new(-v.y, v.x) / len }
    }
    /// Where stop `i`'s chip sits.
    pub fn stop_point(&self, cx: &ToolContext, i: usize) -> Option<Point> {
        let s = self.gradient.stops.get(i)?;
        Some(self.at(s.offset as f64) + self.normal() * cx.tol(STOP_GAP))
    }
    /// Where the midpoint diamond after stop `i` sits.
    pub fn mid_point(&self, cx: &ToolContext, i: usize) -> Option<Point> {
        Some(self.at(midpoint_pos(&self.gradient.stops, i)? as f64) - self.normal() * cx.tol(MID_GAP))
    }
    /// `p` as (offset along the bar, signed distance from it towards the stop row).
    fn project(&self, p: Point) -> (f64, f64) {
        let v = self.vector();
        let l2 = v.hypot2();
        let d = p - self.geom.start;
        if l2 < 1e-18 {
            return (0.0, d.hypot());
        }
        (d.dot(v) / l2, d.dot(self.normal()))
    }
    /// The part under `p`: stops, then midpoints, the handles, the rotate zone and the bar.
    pub fn hit(&self, cx: &ToolContext, p: Point) -> Option<Part> {
        let near = |q: Option<Point>, r: f64| q.is_some_and(|q| q.distance(p) <= cx.tol(r));
        let n = self.gradient.stops.len();
        // The topmost (last drawn) chip wins where chips overlap.
        if let Some(i) = (0..n).rev().find(|i| near(self.stop_point(cx, *i), STOP_HIT)) {
            return Some(Part::Stop(i));
        }
        if let Some(i) = (0..n.saturating_sub(1)).find(|i| near(self.mid_point(cx, *i), MID_HIT)) {
            return Some(Part::Mid(i));
        }
        if near(Some(self.geom.end), HANDLE) {
            return Some(Part::End);
        }
        let radial = self.radial();
        if radial && near(Some(self.geom.focal_point()), FOCAL_HIT) {
            return Some(Part::Focal);
        }
        if near(Some(self.geom.start), if radial { CENTRE_HIT } else { HANDLE }) {
            return Some(Part::Start);
        }
        if radial && near(Some(self.aspect_point()), HANDLE) {
            return Some(Part::Aspect);
        }
        let (t, d) = self.project(p);
        if t > 1.0 && near(Some(self.geom.end), ROTATE) {
            return Some(Part::Rotate);
        }
        let across = -cx.tol(BAR_HIT)..=cx.tol(STOP_GAP + STOP_HIT);
        if self.vector().hypot() > 1e-12 && (0.0..=1.0).contains(&t) && across.contains(&d) {
            return Some(Part::Bar(t as f32));
        }
        if radial && self.off_ellipse(p).is_some_and(|d| d <= cx.tol(BAR_HIT)) {
            return Some(Part::Rotate);
        }
        None
    }
}

/// What a press grabbed.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    /// The art (or empty canvas): a click applies the gradient from there, a drag draws a vector.
    Art,
    /// The start handle or the bar: a drag moves the gradient (a click on the bar adds a stop).
    Move {
        geom: GradientGeom,
        bar: Option<f32>,
    },
    End(GradientGeom),
    /// Rotates the vector, by the pointer's turn about the start from `from` (radians).
    Rotate {
        geom: GradientGeom,
        from: f64,
    },
    Focal(GradientGeom),
    Aspect(GradientGeom),
    Stop {
        index: usize,
        from: Annotator,
        copy: bool,
    },
    Mid {
        index: usize,
        from: Annotator,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Gesture {
    grab: Grab,
    /// Where the press happened.
    at: Point,
    /// Moved past the drag threshold (an interaction is open).
    began: bool,
    /// The vector drawn so far (Art drags).
    vector: Option<(Point, Point)>,
    /// Where a dragged stop ends up (selected on release): its index, or None while it is dragged
    /// off the bar.
    stop: Option<Option<usize>>,
}

#[derive(Default)]
pub struct GradientTool {
    gesture: Option<Gesture>,
    /// A press on a freeform gradient's annotator.
    free: Option<freeform::Gesture>,
    /// The line Lines mode is drawing on a freeform gradient.
    lines: freeform::Lines,
    /// Smart-guide feedback of the handle being dragged.
    guides: Vec<Overlay>,
}

/// Overlays of an annotator: the bar, the handles, the midpoint diamonds and a chip per stop; a
/// radial's extent ellipse with its aspect handle, centre ring and focal dot.
fn annotator_overlays(cx: &ToolContext, a: &Annotator) -> Vec<Overlay> {
    let (s, e) = (a.geom.start, a.geom.end);
    let shade = Vec2::new(0.0, cx.tol(1.0));
    let mut o = vec![];
    if let Some(m) = a.geom.unit_frame().filter(|_| a.radial()) {
        // Dark dashes over a light line: readable on any art.
        let path = vectorcraft_geom::kurbo::Ellipse::from_affine(m).to_path(cx.tol(0.25));
        o.push(Overlay::Path { path: path.clone(), color: LIGHT, width: 1.0, dashed: false });
        o.push(Overlay::Path { path, color: BAR, width: 1.0, dashed: true });
        o.push(Overlay::Handle { p: a.aspect_point(), color: BAR });
        let ring = vectorcraft_geom::kurbo::Circle::new(s, cx.tol(CENTRE)).to_path(cx.tol(0.25));
        o.push(Overlay::Path { path: ring, color: BAR, width: 1.5, dashed: false });
    }
    o.extend([
        Overlay::Line { a: s, b: e, color: BAR, dashed: false },
        Overlay::Line { a: s + shade, b: e + shade, color: LIGHT, dashed: false },
        Overlay::Handle { p: if a.radial() { a.geom.focal_point() } else { s }, color: BAR },
        Overlay::Anchor { p: e, color: BAR, filled: true, size: 7.0 },
    ]);
    let len = a.vector().hypot();
    if len < 1e-12 {
        return o;
    }
    let r = cx.tol(4.0);
    let (along, across) = (a.vector() / len * r, a.normal() * r);
    for i in 0..a.gradient.stops.len().saturating_sub(1) {
        let Some(c) = a.mid_point(cx, i) else { continue };
        let path = super::polygon(&[c - along, c - across, c + along, c + across], true);
        o.push(Overlay::Path { path, color: BAR, width: 1.0, dashed: false });
    }
    for (i, st) in a.gradient.stops.iter().enumerate() {
        let Some(p) = a.stop_point(cx, i) else { continue };
        o.push(Overlay::Line { a: a.at(st.offset as f64), b: p, color: BAR, dashed: false });
        o.push(Overlay::Swatch { p, color: st.color.to_rgba8(st.opacity), selected: cx.gradient_stop == Some(i) });
    }
    o
}

/// The vector a click applies to `id`: its current gradient's, else the default fit on its box.
fn click_vector(cx: &ToolContext, id: NodeId) -> Option<Vec2> {
    let stroke = !cx.fill_active;
    let n = cx.doc.node(id)?;
    if let Some((_, g)) = n.proxy_gradient(stroke, cx.appearance_item) {
        return Some(g.end - g.start);
    }
    let (to_doc, b) = match n.proxy_paint(stroke, cx.appearance_item) {
        Some((_, a, b)) => (a, b),
        None => (Affine::IDENTITY, n.geometric_bounds()?),
    };
    let mut g = GradientGeom::fit(GradientKind::Linear, b, 0.0);
    g.transform(to_doc, GradientKind::Linear);
    Some(g.end - g.start)
}

impl GradientTool {
    fn geom_params(cx: &ToolContext, start: Point, end: Point) -> Value {
        json!({ "start": [start.x, start.y], "end": [end.x, end.y], "stroke": !cx.fill_active })
    }
    /// The stops the tool sends carry the gradient's own colors or samples of it, so they keep
    /// their models (`keepModel`), in a CMYK document too.
    fn stops_params(cx: &ToolContext, stops: &[GradientStop]) -> Value {
        json!({ "stops": crate::params::stops_json(stops), "stroke": !cx.fill_active, "keepModel": true })
    }
    fn select_stop(i: usize) -> Action {
        Action::Exec("gradient.selectStop".into(), json!({ "index": i }))
    }

    fn press(&mut self, cx: &ToolContext, p: Point, mods: Mods) -> Vec<Action> {
        let annotator = Annotator::of(cx);
        let mut out = vec![];
        let grab = match annotator.as_ref().and_then(|a| a.hit(cx, p).map(|part| (a, part))) {
            Some((a, Part::Stop(index))) => {
                out.push(Self::select_stop(index));
                Grab::Stop { index, from: a.clone(), copy: mods.alt }
            }
            Some((a, Part::Mid(index))) => Grab::Mid { index, from: a.clone() },
            Some((a, Part::Start)) => Grab::Move { geom: a.geom, bar: None },
            Some((a, Part::Bar(t))) => Grab::Move { geom: a.geom, bar: Some(t) },
            Some((a, Part::End)) => Grab::End(a.geom),
            Some((a, Part::Rotate)) => Grab::Rotate { geom: a.geom, from: (p - a.geom.start).atan2() - a.vector().atan2() },
            Some((a, Part::Focal)) => Grab::Focal(a.geom),
            Some((a, Part::Aspect)) => Grab::Aspect(a.geom),
            None => {
                if cx.selection.is_empty() {
                    let Some(h) = hit_test(cx.doc, p, cx.hit_options()) else { return out };
                    out.push(Action::Exec("select.set".into(), json!({ "ids": [paint_owner(cx.doc, h.leaf).0] })));
                }
                Grab::Art
            }
        };
        // A vector drawn on the art starts on the snapped point.
        let at = if grab == Grab::Art { snap_draw(cx, p, &[]).0 } else { p };
        self.gesture = Some(Gesture { grab, at, began: false, vector: None, stop: None });
        out
    }

    fn drag(&mut self, cx: &ToolContext, p: Point, mods: Mods) -> Vec<Action> {
        let Some(g) = &mut self.gesture else { return vec![] };
        let mut out = vec![];
        if !g.began {
            if p.distance(g.at) < cx.tol(DRAG_START) {
                return out;
            }
            g.began = true;
            out.push(Action::Begin("Gradient".into()));
        }
        let constrain = |v: Vec2| constrain_angle_from(v, 45.0, cx.constrain_angle);
        // Where a handle dragged to `to` lands: Shift constrains the vector from `from`, else it
        // snaps (keeping the guides to show).
        let mut guides = vec![];
        let mut place = |from: Point, to: Point| {
            if mods.shift {
                from + constrain(to - from)
            } else {
                let (q, o) = snap_draw(cx, to, &[]);
                guides = o;
                q
            }
        };
        let (cmd, params) = match &g.grab {
            Grab::Art => {
                let end = place(g.at, p);
                g.vector = Some((g.at, end));
                ("paint.setGradientGeom", Self::geom_params(cx, g.at, end))
            }
            Grab::Move { geom, .. } => {
                // The start handle snaps; Shift constrains the move.
                let start = place(geom.start, geom.start + (p - g.at));
                ("paint.setGradientGeom", Self::geom_params(cx, start, start + (geom.end - geom.start)))
            }
            Grab::End(geom) => ("paint.setGradientGeom", Self::geom_params(cx, geom.start, place(geom.start, p))),
            Grab::Rotate { geom, from } => {
                let dir = (Affine::rotate(-from) * (p - geom.start).to_point()).to_vec2();
                let dir = if mods.shift { constrain(dir) } else { dir };
                let len = dir.hypot();
                let end = if len < 1e-12 { geom.end } else { geom.start + dir * (geom.length() / len) };
                ("paint.setGradientGeom", Self::geom_params(cx, geom.start, end))
            }
            Grab::Focal(geom) => {
                // Dropped back on the centre it centres.
                let f = geom.focal_point() + (p - g.at);
                let mut params = Self::geom_params(cx, geom.start, geom.end);
                params["focal"] = if f.distance(geom.start) <= cx.tol(FOCAL_HIT) { Value::Null } else { json!([f.x, f.y]) };
                ("paint.setGradientGeom", params)
            }
            Grab::Aspect(geom) => {
                // The pointer's distance across the bar, as a fraction of the radius.
                let across = Annotator::with(*geom, &[]).project(p).1.abs();
                let mut params = Self::geom_params(cx, geom.start, geom.end);
                params["aspect"] = json!(across / geom.length().max(1e-12) * 100.0);
                ("paint.setGradientGeom", params)
            }
            Grab::Stop { index, from, copy } => {
                let (t, d) = from.project(p);
                let t = t.clamp(0.0, 1.0) as f32;
                let stops = &from.gradient.stops;
                let removed = (!copy && d.abs() > cx.tol(OFF_BAR)).then(|| remove_stop(stops, *index)).flatten();
                let (stops, at) = match removed {
                    Some(v) => (v, None),
                    None => {
                        let (v, i) = if *copy { duplicate_stop(stops, *index, t) } else { move_stop(stops, *index, t) };
                        (v, Some(i))
                    }
                };
                g.stop = Some(at);
                ("paint.editGradient", Self::stops_params(cx, &stops))
            }
            Grab::Mid { index, from } => {
                let stops = &from.gradient.stops;
                let m = midpoint_from_pos(stops, *index, from.project(p).0.clamp(0.0, 1.0) as f32).unwrap_or(0.5);
                ("paint.editGradient", Self::stops_params(cx, &set_midpoint(stops, *index, m)))
            }
        };
        self.guides = guides;
        out.push(Action::Preview(cmd.into(), params));
        out
    }

    fn release(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        self.guides.clear();
        let Some(g) = self.gesture.take() else { return vec![] };
        if g.began {
            let mut out = vec![Action::Commit];
            match (g.grab, g.stop) {
                (Grab::Stop { .. }, Some(Some(i))) => out.push(Self::select_stop(i)),
                // Dragged off the bar: the stop that slid into its place is selected.
                (Grab::Stop { index, from, .. }, Some(None)) => out.push(Self::select_stop(index.min(from.gradient.stops.len() - 2))),
                _ => {}
            }
            return out;
        }
        match g.grab {
            // A click on the bar adds a stop there.
            Grab::Move { bar: Some(t), .. } => {
                let Some(a) = Annotator::of(cx) else { return vec![] };
                let (stops, i) = insert_stop(&a.gradient, t);
                vec![Action::Exec("paint.editGradient".into(), Self::stops_params(cx, &stops)), Self::select_stop(i)]
            }
            // A click inside selected art applies the gradient from that point.
            Grab::Art => {
                let Some(h) = hit_test(cx.doc, p, cx.hit_options()) else { return vec![] };
                let id = paint_owner(cx.doc, h.leaf);
                let Some(v) = cx.selection.objects.contains(&id).then(|| click_vector(cx, id)).flatten() else { return vec![] };
                let mut params = Self::geom_params(cx, p, p + v);
                params["ids"] = json!([id.0]);
                // With `ids` the engine edits the topmost fill or stroke unless an item is named.
                if let Some(i) = cx.doc.node(id).and_then(|n| n.appearance.item_of_kind(cx.appearance_item, cx.fill_active)) {
                    params["item"] = json!(i);
                }
                vec![Action::Exec("paint.setGradientGeom".into(), params)]
            }
            _ => vec![],
        }
    }
}

impl Tool for GradientTool {
    fn id(&self) -> &'static str {
        "gradient"
    }

    fn busy(&self) -> bool {
        self.gesture.as_ref().is_some_and(|g| g.began) || self.free.as_ref().is_some_and(freeform::Gesture::began)
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        // A freeform gradient: its points instead of the bar.
        match (ev.kind, &mut self.free) {
            (PointerKind::Down, _) => {
                if let Some(a) = FreeformAnnotator::of(cx) {
                    let (out, g) = freeform::press(cx, &a, ev.pos);
                    self.free = Some(g);
                    return out;
                }
            }
            (PointerKind::Drag, Some(g)) => return freeform::drag(cx, g, ev.pos),
            (PointerKind::Up, Some(_)) => {
                if let Some(g) = self.free.take() {
                    return freeform::release(cx, FreeformAnnotator::of(cx).as_ref(), g, &mut self.lines);
                }
            }
            (PointerKind::Move, None) => freeform::hover(cx, &mut self.lines, ev.pos),
            (PointerKind::DoubleClick, _) => {
                if let Some(a) = FreeformAnnotator::of(cx) {
                    return freeform::double_click(cx, &a, ev.pos);
                }
            }
            _ => {}
        }
        match ev.kind {
            PointerKind::Down => self.press(cx, ev.pos, ev.mods),
            PointerKind::Drag => self.drag(cx, ev.pos, ev.mods),
            PointerKind::Up => self.release(cx, ev.pos),
            PointerKind::DoubleClick => {
                let Some(a) = Annotator::of(cx) else { return vec![] };
                let Some(Part::Stop(i)) = a.hit(cx, ev.pos) else { return vec![] };
                let at = a.stop_point(cx, i).unwrap_or(ev.pos);
                vec![Self::select_stop(i), Action::Dialog("gradientStop".into(), json!({ "index": i, "x": at.x, "y": at.y }))]
            }
            PointerKind::Move => vec![],
        }
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        if let Some(a) = FreeformAnnotator::of(cx) {
            return freeform::claims_key(cx, &a, &self.lines, key);
        }
        matches!(key, ToolKey::Delete | ToolKey::Backspace | ToolKey::Left | ToolKey::Right)
            && cx.gradient_stop.is_some_and(|i| Annotator::of(cx).is_some_and(|a| i < a.gradient.stops.len()))
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.busy() {
            self.gesture = None;
            self.guides.clear();
            return self.free.take().map_or_else(|| vec![Action::Cancel], |g| freeform::cancel(&g));
        }
        if self.busy() || !self.claims_key(cx, key) {
            return vec![];
        }
        if let Some(a) = FreeformAnnotator::of(cx) {
            return freeform::key(cx, &a, &mut self.lines, key);
        }
        let (Some(a), Some(i)) = (Annotator::of(cx), cx.gradient_stop) else { return vec![] };
        let stops = &a.gradient.stops;
        let (new, sel) = match key {
            // Never below two stops: Delete then does nothing.
            ToolKey::Delete | ToolKey::Backspace => match remove_stop(stops, i) {
                Some(v) => {
                    let sel = i.min(v.len() - 1);
                    (v, sel)
                }
                None => return vec![],
            },
            _ => {
                let step = if mods.shift { NUDGE_BIG } else { NUDGE };
                move_stop(stops, i, stops[i].offset + if key == ToolKey::Left { -step } else { step })
            }
        };
        vec![Action::Exec("paint.editGradient".into(), Self::stops_params(cx, &new)), Self::select_stop(sel)]
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if let Some(a) = FreeformAnnotator::of(cx) {
            return a.overlays(cx, &self.lines);
        }
        let mut o = match (Annotator::of(cx), self.gesture.as_ref().and_then(|g| g.vector)) {
            (Some(a), _) => annotator_overlays(cx, &a),
            // A vector drawn on a paint that isn't a gradient yet.
            (None, Some((s, e))) => {
                annotator_overlays(cx, &Annotator::with(GradientGeom { start: s, end: e, aspect: 1.0, focal: None }, &Gradient::default().stops))
            }
            _ => vec![],
        };
        o.extend(self.guides.iter().cloned());
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if let Some(a) = FreeformAnnotator::of(cx) {
            return freeform::cursor(cx, &a, self.free.as_ref(), p);
        }
        if let Some(g) = &self.gesture {
            return match (&g.grab, g.stop) {
                (Grab::Stop { .. }, Some(None)) => Cursor::RemoveStop,
                (Grab::Rotate { .. }, _) => Cursor::Rotate,
                (Grab::Art, _) => Cursor::Crosshair,
                _ => Cursor::Move,
            };
        }
        match Annotator::of(cx).and_then(|a| a.hit(cx, p)) {
            Some(Part::Bar(_)) => Cursor::AddStop,
            Some(Part::Rotate) => Cursor::Rotate,
            Some(_) => Cursor::Move,
            None => Cursor::Crosshair,
        }
    }

    fn options(&self) -> Value {
        Value::Null
    }
}

#[cfg(test)]
mod tests;
