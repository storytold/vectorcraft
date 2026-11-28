//! Artboard rotation and light controls for the open 3D effect dialog (Extrude & Bevel, Revolve, Inflate, Rotate and
//! the classic 3D effects). [`RevolveGizmo`] is a canvas control over the rotation and light values: a drag is one edit
//! gesture whose values [`show`] writes into the dialog's fields, and the dialog's preview applies them, owns OK,
//! Cancel and undo. Escape during a drag is the gesture's cancel: the values go back to where the press found them.
//!
//! - Rotation page: three rings (X, Y, Z) turn one axis each, a press at the centre or anywhere else in the gizmo's
//!   square turns X and Y together. Shift snaps to 15 degrees.
//! - Lighting page: the light's handle, its two rings and their labels move the selected light; Shift snaps to 15
//!   degrees. Nothing shows with Preview off or while the lighting page has no light to move ([`lit`]).
//! - A press that never drags changes nothing: the gesture begins at the first drag.
use craft::behavior::{
    CancelReason, CanvasView, CursorIcon, CursorPriority, HitLevel, HitPriority, HitShape, Metrics, PartKey, PartRef, PointerButton, PressPhase,
    Sense,
};
// PLAN: the `craft` facade crate needs to be a dependency in crates/ui-egui/Cargo.toml (owner: hub-w1/hub-w2).
use craft::ui::canvas::{CanvasSpace, canvas_space};
use craft::ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, Show as _,
};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
use craft::ui::primitives::{ContentPainterCx, TextRun};
use craft::ui::runtime::focus::KeyUse;
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx, TypeRole};
use egui::{Pos2, Rect, Response, Stroke, Ui, Vec2, vec2};
use serde_json::json;
use vectorcraft_effects::{CLASSIC_REVOLVE, REVOLVE};
use vectorcraft_geom::Point;
use vectorcraft_three_d::Revolve;

use super::revolve::{light_options, set_light_value};
use super::three_d;
use crate::VectorcraftApp;

const KEYS: [&str; 3] = ["rotationX", "rotationY", "rotationZ"];
const SAMPLES: usize = 96;
/// How far from a ring a press still takes it, in points.
const RING_REACH: f32 = 9.0;
/// How far from the centre a press turns X and Y together, in points.
const ORBIT_REACH: f32 = 14.0;
/// How far from the light's handle a press takes it, in points.
const LIGHT_REACH: f32 = 18.0;
/// The least radius of the plate under an axis letter, in points; a taller letter gets a plate as tall as its line.
const AXIS_PLATE: f32 = 9.0;

const RING: PartKey = PartKey::new("ring");
const CENTRE: PartKey = PartKey::new("centre");
const LIGHT: PartKey = PartKey::new("light");
const LIGHT_RING: PartKey = PartKey::new("lightRing");
const LIGHT_LABEL: PartKey = PartKey::new("lightLabel");
const AREA: PartKey = PartKey::new("area");

/// The keyboard path of every part: the dialog's own rotation and light fields.
const BY_FIELDS: &str = "the rotation and light fields of the dialog";

/// Does the effect take light: Shading on, or for the classic 3D effects a Diffuse or Plastic surface?
pub(crate) fn lit(fields: &DialogFields) -> bool {
    // PLAN: assumes the dialog's fields keep `__effect` (ef.C13 lists only `__page` and `__light`).
    let id = fields.text("__effect").unwrap_or_default();
    if vectorcraft_effects::is_classic_three_d(id) {
        matches!(fields.text("surface"), Some("diffuse" | "plastic"))
    } else {
        fields.bool("shade").unwrap_or(false)
    }
}

/// Is the dialog on its Lighting page?
fn lighting(fields: &DialogFields) -> bool {
    fields.text("__page") == Some("lighting")
}

/// Does the gizmo show for these values: on the Lighting page only while the effect takes light. Preview is the
/// dialog's own state, which [`three_d::gizmo_shown`] reads.
fn shown(fields: &DialogFields) -> bool {
    !lighting(fields) || lit(fields)
}

/// The values the gizmo edits: the three rotations and the selected light's rotation and height, in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GizmoValues {
    /// The rotations about X, Y and Z, in [`KEYS`] order.
    pub rotation: [f64; 3],
    /// The selected light's rotation.
    pub light_rotation: f64,
    /// The selected light's height.
    pub light_height: f64,
}

impl GizmoValues {
    /// The values the dialog holds; a missing rotation reads 0.
    fn of(fields: &DialogFields) -> Self {
        let options = light_options(fields);
        GizmoValues {
            rotation: KEYS.map(|key| fields.number(key).unwrap_or(0.0)),
            light_rotation: options.light_rotation,
            light_height: options.light_height,
        }
    }
}

/// What the gizmo works on: the dialog's values and the bounds of the art it turns.
struct Subject {
    fields: DialogFields,
    bounds: vectorcraft_geom::Rect,
}

#[derive(Clone, Copy, Debug, Default)]
struct Ring {
    center: Vec2,
    u: Vec2,
    v: Vec2,
    depth: [f32; 2],
}

impl Ring {
    fn point(self, angle: f32) -> Vec2 {
        self.center + self.u * angle.cos() + self.v * angle.sin()
    }
    fn tangent(self, angle: f32) -> Vec2 {
        -self.u * angle.sin() + self.v * angle.cos()
    }
    fn phase(self, p: Vec2) -> Option<f32> {
        let p = p - self.center;
        let det = self.u.x * self.v.y - self.u.y * self.v.x;
        // Near edge-on, an ellipse's inverse magnifies tiny pointer movements.
        if det.abs() < self.u.length().max(self.v.length()).powi(2) * 0.15 {
            return None;
        }
        let x = (p.x * self.v.y - p.y * self.v.x) / det;
        let y = (p.y * self.u.x - p.x * self.u.y) / det;
        (x.hypot(y) > 0.1).then(|| y.atan2(x))
    }
}

/// The sine and cosine of the view's turn: document directions on screen.
fn turn(view: &CanvasView) -> (f32, f32) {
    (view.rotation.to_radians() as f32).sin_cos()
}

#[derive(Clone, Copy, Debug, Default)]
struct Geometry {
    center: Pos2,
    radius: f32,
    rings: [Ring; 3],
    turn: (f32, f32),
}

impl Geometry {
    /// The gizmo of the art in `bounds` under `view`; `None` when its centre is off the canvas.
    fn new(view: &CanvasView, fields: &DialogFields, bounds: vectorcraft_geom::Rect) -> Option<Self> {
        let options = light_options(fields);
        // Both Revolves turn about their axis; the others about the centre of the art's bounds.
        let id = fields.text("__effect").unwrap_or_default();
        let origin = if id == REVOLVE || id == CLASSIC_REVOLVE { options.rotation_origin(bounds) } else { bounds.center() };
        let center = view.to_screen(origin);
        if !center.x.is_finite() || !center.y.is_finite() || !view.rect.contains(center) {
            return None;
        }
        let radius = (bounds.height().abs() * view.zoom * 0.24).clamp(76.0, 116.0) as f32;
        let (s, c) = turn(view);
        let screen = |p: [f64; 3], scale: f32| vec2(c * p[0] as f32 - s * p[1] as f32, s * p[0] as f32 + c * p[1] as f32) * radius * scale;
        let rings = std::array::from_fn(|i| {
            let [u, v] = options.rotation_planes()[i];
            let scale = if i == 2 { 1.12 } else { 1.0 };
            Ring { center: Vec2::ZERO, u: screen(u, scale), v: screen(v, scale), depth: [u[2] as f32, v[2] as f32] }
        });
        Some(Self { center, radius, rings, turn: (s, c) })
    }

    fn hit(self, p: Pos2) -> Option<(usize, f32)> {
        let mut best = None;
        let mut distance = RING_REACH;
        for (axis, ring) in self.rings.iter().enumerate() {
            for i in 0..SAMPLES {
                let a = i as f32 * std::f32::consts::TAU / SAMPLES as f32;
                let b = a + std::f32::consts::TAU / SAMPLES as f32;
                let (start, end) = (self.center + ring.point(a), self.center + ring.point(b));
                let line = end - start;
                let t = ((p - start).dot(line) / line.length_sq().max(1e-6)).clamp(0.0, 1.0);
                let d = p.distance(start + line * t);
                if d < distance {
                    distance = d;
                    best = Some((axis, a + (b - a) * t));
                }
            }
        }
        best
    }

    fn light_rings(self, options: Revolve) -> [Ring; 2] {
        let (s, c) = self.turn;
        let screen = |p: [f64; 3]| vec2(c * p[0] as f32 - s * p[1] as f32, s * p[0] as f32 + c * p[1] as f32) * self.radius * 1.4;
        options.light_orbits().map(|[center, u, v]| Ring { center: screen(center), u: screen(u), v: screen(v), depth: [u[2] as f32, v[2] as f32] })
    }

    fn light_labels(self) -> [Rect; 2] {
        [
            Rect::from_center_size(self.center + vec2(0.0, self.radius * 1.4 + 24.0), vec2(88.0, 26.0)),
            Rect::from_center_size(self.center - vec2(self.radius * 1.4 + 32.0, 0.0), vec2(78.0, 26.0)),
        ]
    }

    fn light_hit(self, p: Pos2, options: Revolve) -> Option<usize> {
        if let Some(axis) = self.light_labels().iter().position(|r| r.contains(p)) {
            return Some(axis);
        }
        let mut best = None;
        let mut distance = RING_REACH;
        for (axis, ring) in self.light_rings(options).iter().enumerate() {
            let span = light_span(axis);
            for i in 0..SAMPLES {
                let a = i as f32 * span / SAMPLES as f32;
                let (from, to) = (self.center + ring.point(a), self.center + ring.point(a + span / SAMPLES as f32));
                let line = to - from;
                let t = ((p - from).dot(line) / line.length_sq().max(1e-6)).clamp(0.0, 1.0);
                let d = p.distance(from + line * t);
                if d < distance {
                    distance = d;
                    best = Some(axis);
                }
            }
        }
        best
    }

    /// The points of ring `ring` over `span` radians from its start, for a hit shape.
    fn arc(self, ring: Ring, span: f32) -> Vec<Pos2> {
        (0..=SAMPLES).map(|i| self.center + ring.point(i as f32 * span / SAMPLES as f32)).collect()
    }

    /// The square the gizmo takes presses in; the light page adds its labels.
    fn area(self, light_mode: bool) -> Rect {
        let mut rect = Rect::from_center_size(self.center, Vec2::splat(self.radius * 3.0 + 48.0));
        if light_mode {
            for label in self.light_labels() {
                rect = rect.union(label.expand(2.0));
            }
        }
        rect
    }
}

/// The angle a light ring spans: the Rotation ring is whole, the Height ring a quarter.
fn light_span(axis: usize) -> f32 {
    if axis == 0 { std::f32::consts::TAU } else { std::f32::consts::FRAC_PI_2 }
}

/// What a drag turns, fixed at the press.
#[derive(Clone, Copy, Debug, Default)]
enum Mode {
    Ring {
        axis: usize,
        ring: Ring,
        phase: f32,
        degrees: f64,
    },
    #[default]
    Orbit,
    Light,
    LightRing {
        axis: usize,
    },
}

/// The press the gizmo is dragging, kept from the press to its release. The ring and the geometry stay as the press
/// found them, so a ring does not slide under the pointer.
#[derive(Clone, Debug, Default)]
struct Grip {
    mode: Mode,
    geometry: Geometry,
    /// The values at the press.
    start: GizmoValues,
    /// The dialog's values at the press: the light moves from the options they give.
    fields: DialogFields,
    origin: Pos2,
    /// The pointer where the last step left it: a pass that sees the same position steps by nothing.
    last: Pos2,
    /// The values the last step proposed.
    latest: GizmoValues,
    began: bool,
}

pub(super) fn wrap(n: f64, snap: bool) -> f64 {
    let n = if snap { (n / 15.0).round() * 15.0 } else { n };
    (n + 180.0).rem_euclid(360.0) - 180.0
}

/// A Height, 0° to 90°, on 15° steps with Shift.
pub(super) fn snap_height(height: f64, snap: bool) -> f64 {
    (if snap { (height / 15.0).round() * 15.0 } else { height }).clamp(0.0, 90.0)
}

/// What a press at `origin` turns.
fn mode_at(g: Geometry, options: Revolve, light_mode: bool, origin: Pos2) -> Mode {
    if light_mode {
        if origin.distance(light_position(g, options)) <= LIGHT_REACH {
            Mode::Light
        } else if let Some(axis) = g.light_hit(origin, options) {
            Mode::LightRing { axis }
        } else {
            Mode::Light
        }
    } else if origin.distance(g.center) <= ORBIT_REACH {
        Mode::Orbit
    } else if let Some((axis, phase)) = g.hit(origin) {
        Mode::Ring { axis, ring: g.rings[axis], phase, degrees: 0.0 }
    } else {
        Mode::Orbit
    }
}

impl Grip {
    /// The values the pointer at `pos` gives, with Shift as `snap`; moves the ring's phase on.
    fn follow(&mut self, view: &CanvasView, pos: Pos2, snap: bool) -> GizmoValues {
        let g = self.geometry;
        let mut next = self.start;
        match &mut self.mode {
            Mode::Ring { axis, ring, phase, degrees } => {
                let step = if let Some(now) = ring.phase(pos - g.center) {
                    let step = (now - *phase + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                    *phase = now;
                    step
                } else {
                    let tangent = ring.tangent(*phase);
                    let step = (pos - self.last).dot(tangent) / tangent.length_sq().max(144.0);
                    *phase += step;
                    step
                };
                *degrees += f64::from(step.to_degrees());
                let start = self.start.rotation.get(*axis).copied().unwrap_or(0.0);
                if let Some(slot) = next.rotation.get_mut(*axis) {
                    *slot = wrap(start + *degrees, snap);
                }
            }
            Mode::Orbit => {
                let delta = view.unmap_vec(pos - self.origin) * view.zoom;
                let [x, y, z] = self.start.rotation;
                next.rotation = [wrap(x + delta.y * 0.5, snap), wrap(y + delta.x * 0.5, snap), z];
            }
            Mode::Light => {
                let start = light_options(&self.fields);
                let projected = light_position(g, start) + (pos - self.origin);
                let point = view.unmap_vec(projected - g.center) * (view.zoom / f64::from(g.radius * 1.4));
                let [rotation, height] = start.light_at_on_side(Point::new(point.x, point.y));
                next.light_rotation = wrap(rotation, snap);
                next.light_height = snap_height(height, snap);
            }
            Mode::LightRing { axis } => {
                let delta = view.unmap_vec(pos - self.origin) * view.zoom;
                let start = light_options(&self.fields);
                if *axis == 0 {
                    // Right turns the light counterclockwise, as along the ring's foot.
                    next.light_rotation = wrap(start.light_rotation + delta.x * 0.6, snap);
                } else {
                    next.light_height = snap_height(start.light_height - delta.y * 0.6, snap);
                }
            }
        }
        self.last = pos;
        self.latest = next;
        next
    }
}

/// The style of the gizmo: the axis colours, the hot tone and the plates under its labels.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct RevolveGizmoStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The X, Y and Z rings.
    axis_x: StyleColor,
    axis_y: StyleColor,
    axis_z: StyleColor,
    /// The hovered or dragged ring, the light and its rings.
    hot: StyleColor,
    /// The plate under a letter or a label, and the fill of a back light.
    plate: StyleColor,
    /// A hovered label's plate.
    plate_hover: StyleColor,
    /// The outline under the rings, so they read on any art.
    halo: StyleColor,
    /// Guides and the centre mark.
    muted: StyleColor,
    /// The readout's text.
    ink: StyleColor,
}

impl RevolveGizmoStyle {
    fn axis(&self, axis: usize) -> StyleColor {
        [self.axis_x, self.axis_y, self.axis_z].get(axis).copied().unwrap_or(self.axis_z)
    }
}

/// What the gizmo reports: the edit of the values.
#[derive(Debug)]
pub(crate) struct RevolveGizmoOutput {
    /// A drag as `Begin` (at the first drag), `Update`s and `Commit` at the release; Escape cancels it, as does a
    /// press that ends where it began.
    pub edit: EditResponse<GizmoValues>,
}

impl Outcome for RevolveGizmoOutput {
    type Value = GizmoValues;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

#[derive(Default)]
struct Report {
    /// The ring or light ring the pointer is over.
    active: Option<usize>,
    /// What the open press turns.
    held: Option<Mode>,
}

/// The rotation rings and the light of a 3D effect's dialog, over the canvas.
///
/// - Pointer: the grab hand over the rings, the centre and the light, closed during a drag. The parts are pointer
///   only: the dialog's fields are the keyboard path.
/// - A press that is not on a ring turns X and Y together (the rotation page) or moves the light (the lighting page).
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub(crate) struct RevolveGizmo<'a> {
    #[common]
    common: Common,
    values: Bind<'a, GizmoValues>,
    subject: &'a Subject,
    report: Report,
}

impl<'a> RevolveGizmo<'a> {
    fn new(label: &'static str, values: Bind<'a, GizmoValues>, subject: &'a Subject) -> Self {
        RevolveGizmo { common: Common::new(), values, subject, report: Report::default() }.label(label)
    }

    fn light_mode(&self) -> bool {
        lighting(&self.subject.fields)
    }

    /// The parts of the page in `light_mode`, each with the ring it highlights.
    fn part_list(light_mode: bool) -> Vec<(PartRef, Option<usize>)> {
        let mut parts = vec![(AREA.at(0), None)];
        if light_mode {
            parts.push((LIGHT.at(0), None));
            parts.extend((0..2).flat_map(|axis| [(LIGHT_RING.at(axis as u64), Some(axis)), (LIGHT_LABEL.at(axis as u64), Some(axis))]));
        } else {
            parts.push((CENTRE.at(0), None));
            parts.extend((0..3).map(|axis| (RING.at(axis as u64), Some(axis))));
        }
        parts
    }

    fn declare(cx: &mut PartsCx<'_, Self>, part: PartRef, shape: HitShape, level: HitLevel) {
        cx.part(part, shape).sense(Sense::DRAG | Sense::CLICK).priority(HitPriority::new(level, 0)).cursor(CursorIcon::Grab).pointer_only(BY_FIELDS);
    }
}

impl Component for RevolveGizmo<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS;
    const OVERLAYS: bool = false;
    type Style = RevolveGizmoStyle;
    type Output = RevolveGizmoOutput;

    fn style(cx: &StyleCx<'_>, _: RevolveGizmoStyleKey) -> RevolveGizmoStyle {
        let colors = &cx.tokens.color;
        RevolveGizmoStyle {
            metrics: Metrics::new(),
            axis_x: cx.color(colors.data.red),
            axis_y: cx.color(colors.data.green),
            axis_z: cx.color(colors.data.blue),
            hot: cx.color(colors.status.warn.solid),
            plate: cx.color(colors.surface.raised),
            plate_hover: cx.color(colors.state.hover),
            halo: cx.color(colors.overlay.halo),
            muted: cx.color(colors.text.muted),
            ink: cx.color(colors.text.normal),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, GizmoValues>> {
        Some(self.values.reborrow())
    }

    /// The rings, the centre and the light as the page has them, over the gizmo's square, which takes every other
    /// press of it.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(g) = Geometry::new(&space.view, &self.subject.fields, self.subject.bounds) else { return };
        let light_mode = self.light_mode();
        Self::declare(cx, AREA.at(0), HitShape::Rect(g.area(light_mode).intersect(rect)), HitLevel::Body);
        if light_mode {
            let options = light_options(&self.subject.fields);
            Self::declare(cx, LIGHT.at(0), HitShape::circle(light_position(g, options), LIGHT_REACH), HitLevel::Handle);
            for (axis, (ring, label)) in g.light_rings(options).into_iter().zip(g.light_labels()).enumerate() {
                let points = g.arc(ring, light_span(axis));
                Self::declare(cx, LIGHT_RING.at(axis as u64), HitShape::polyline_within(points, axis == 0, RING_REACH), HitLevel::Handle);
                Self::declare(cx, LIGHT_LABEL.at(axis as u64), HitShape::Rect(label), HitLevel::Handle);
            }
        } else {
            Self::declare(cx, CENTRE.at(0), HitShape::circle(g.center, ORBIT_REACH), HitLevel::Handle);
            for (axis, ring) in g.rings.into_iter().enumerate() {
                let points = g.arc(ring, std::f32::consts::TAU);
                Self::declare(cx, RING.at(axis as u64), HitShape::polyline_within(points, true, RING_REACH), HitLevel::Handle);
            }
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let Some(g) = Geometry::new(&space.view, &self.subject.fields, self.subject.bounds) else { return };
        let (light_mode, options) = (self.light_mode(), light_options(&self.subject.fields));
        let mut held: Option<Grip> = cx.press_slot::<Grip>().clone();
        self.report.active = None;
        for (reference, ring) in Self::part_list(light_mode) {
            let Some(result) = cx.part_if_declared(reference) else { continue };
            let press = result.press;
            if result.hovered && held.is_none() && self.report.active.is_none() {
                self.report.active = ring;
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    held = Some(Grip {
                        mode: mode_at(g, options, light_mode, start.origin),
                        geometry: g,
                        start: shown,
                        fields: self.subject.fields.clone(),
                        origin: start.origin,
                        last: start.origin,
                        latest: shown,
                        began: false,
                    });
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let Some(mut grip) = held.take() else { continue };
                    if matches!(press.phase, PressPhase::DragStart { .. }) && !grip.began {
                        cx.begin();
                        grip.began = true;
                    }
                    if grip.began {
                        cx.propose(grip.follow(&space.view, pos, press.mods.shift));
                        cx.cursor(CursorIcon::Grabbing, CursorPriority::Drag);
                    }
                    held = Some(grip);
                }
                PressPhase::Release { .. } => {
                    let Some(grip) = held.take() else { continue };
                    if grip.began && grip.latest == grip.start {
                        // A drag that ended where it began moved nothing: no step.
                        cx.cancel(CancelReason::Unchanged);
                    } else if grip.began {
                        cx.commit();
                    }
                }
                PressPhase::Cancel { .. } => held = None,
                _ => {}
            }
        }
        self.report.held = held.as_ref().map(|grip| grip.mode);
        *cx.press_slot::<Grip>() = held;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(g) = Geometry::new(&space.view, &self.subject.fields, self.subject.bounds) else { return };
        let content = ui.content_painter(cx);
        let paint = Paint {
            content: &content,
            style: cx.style,
            view: &space.view,
            subject: self.subject,
            values: cx.value().copied().unwrap_or_default(),
            g,
            active: self
                .report
                .held
                .and_then(|mode| match mode {
                    Mode::Ring { axis, .. } | Mode::LightRing { axis } => Some(axis),
                    _ => None,
                })
                .or(self.report.active),
            dragging: self.report.held,
        };
        if self.light_mode() {
            paint.light();
        } else {
            paint.rotation();
        }
    }

    fn access(&self, _: &mut Access, _: &Cx<'_, Self>) {}

    fn finish(self, cx: Cx<'_, Self>) -> RevolveGizmoOutput {
        RevolveGizmoOutput { edit: cx.edit_response() }
    }
}

fn light_position(g: Geometry, options: Revolve) -> Pos2 {
    let [x, y, _] = options.light_direction();
    let (s, c) = g.turn;
    g.center + vec2(c * x as f32 - s * y as f32, s * x as f32 + c * y as f32) * g.radius * 1.4
}

/// What a pass draws with.
struct Paint<'a> {
    /// The content painter: its `painter` draws the rings and plates, and its `Draw` lays out and paints the labels.
    content: &'a ContentPainterCx<'a>,
    style: &'a RevolveGizmoStyle,
    view: &'a CanvasView,
    subject: &'a Subject,
    values: GizmoValues,
    g: Geometry,
    /// The ring or light ring shown hot.
    active: Option<usize>,
    /// What the open drag turns.
    dragging: Option<Mode>,
}

impl Paint<'_> {
    fn rotation(&self) {
        let (g, s, painter) = (self.g, self.style, &self.content.painter);
        for (axis, ring) in g.rings.iter().enumerate() {
            let hot = self.active == Some(axis);
            let ink = if hot { s.hot } else { s.axis(axis) };
            let color = ink.egui();
            for i in 0..SAMPLES {
                let angle = i as f32 * std::f32::consts::TAU / SAMPLES as f32;
                let behind = ring.depth[0] * angle.cos() + ring.depth[1] * angle.sin() < -0.05;
                if behind && i % 3 == 0 {
                    continue;
                }
                let points = [g.center + ring.point(angle), g.center + ring.point(angle + std::f32::consts::TAU / SAMPLES as f32)];
                painter.line_segment(points, Stroke::new(4.0, s.halo.egui().gamma_multiply(0.55)));
                painter.line_segment(points, Stroke::new(if hot { 2.5 } else { 1.75 }, color.gamma_multiply(if behind { 0.55 } else { 1.0 })));
            }
            // Letters make the axes distinguishable without relying on colour alone.
            let phase = match axis {
                0 => 0.75,
                1 => 2.25,
                _ => 4.7,
            };
            let p = g.center + ring.point(phase);
            self.text(["X", "Y", "Z"][axis], ink, |size| {
                painter.circle_filled(p, (size.max_elem() * 0.5).max(AXIS_PLATE), s.plate.egui());
                p - size * 0.5
            });
        }
        painter.circle(g.center, 8.0, s.plate.egui().gamma_multiply(0.8), Stroke::new(1.5, s.muted.egui()));
        painter.line_segment([g.center - vec2(4.0, 0.0), g.center + vec2(4.0, 0.0)], Stroke::new(1.0, s.muted.egui()));
        painter.line_segment([g.center - vec2(0.0, 4.0), g.center + vec2(0.0, 4.0)], Stroke::new(1.0, s.muted.egui()));
        if self.subject.fields.bool("__showAxis").unwrap_or(false) {
            let options = light_options(&self.subject.fields);
            painter.line_segment(options.projected_axis(self.subject.bounds).map(|p| self.view.to_screen(p)), Stroke::new(1.0, s.hot.egui()));
        }
        if let Some(Mode::Ring { axis, .. }) = self.dragging {
            let degrees = self.values.rotation.get(axis).copied().unwrap_or(0.0);
            let text = format!("{}  {degrees:.1}°", ["X", "Y", "Z"].get(axis).unwrap_or(&""));
            self.readout(&text, g.center + vec2(0.0, g.radius * 1.12 + 18.0));
        }
    }

    fn light(&self) {
        let (g, s, painter) = (self.g, self.style, &self.content.painter);
        let options = light_options(&self.subject.fields);
        let radius = g.radius * 1.4;
        painter.circle_stroke(g.center, radius, Stroke::new(1.0, s.muted.egui().gamma_multiply(0.3)));
        let labels = g.light_labels();
        for (axis, ring) in g.light_rings(options).iter().enumerate() {
            let span = light_span(axis);
            for i in 0..SAMPLES {
                let a = i as f32 * span / SAMPLES as f32;
                let behind = ring.depth[0] * a.cos() + ring.depth[1] * a.sin() < -0.05;
                if behind && i % 3 == 0 {
                    continue;
                }
                let points = [g.center + ring.point(a), g.center + ring.point(a + span / SAMPLES as f32)];
                painter.line_segment(points, Stroke::new(4.0, s.plate.egui().gamma_multiply(0.4)));
                painter.line_segment(
                    points,
                    Stroke::new(if self.active == Some(axis) { 2.5 } else { 1.5 }, s.hot.egui().gamma_multiply(if behind { 0.45 } else { 0.85 })),
                );
            }
            let label = labels[axis];
            painter.rect_filled(label, 4.0, if self.active == Some(axis) { s.plate_hover.egui() } else { s.plate.egui() });
            let text = if axis == 0 { tl!("Rotation") } else { tl!("Height") };
            self.text(text, s.hot, |size| label.center() - size * 0.5);
        }
        let p = light_position(g, options);
        let delta = g.center - p;
        if delta.length() > 24.0 {
            let direction = delta.normalized();
            let normal = vec2(-direction.y, direction.x);
            let tip = g.center - direction * 12.0;
            painter.line_segment([p, tip], Stroke::new(1.25, s.hot.egui().gamma_multiply(0.6)));
            painter.line_segment([tip, tip - direction * 8.0 + normal * 4.0], Stroke::new(1.25, s.hot.egui()));
            painter.line_segment([tip, tip - direction * 8.0 - normal * 4.0], Stroke::new(1.25, s.hot.egui()));
        }
        painter.circle(g.center, 4.0, s.plate.egui(), Stroke::new(1.0, s.muted.egui()));
        // An original sun handle. A back light is hollow; the guide's back half is dashed.
        let front = options.light_direction()[2] >= 0.0;
        let [r, green, b] = options.light_color.map(|c| (c * 255.0).round() as u8);
        let color = egui::Color32::from_rgb(r, green, b);
        // Keep the themed outline and rays readable even with a dark light color.
        painter.circle(p, 9.0, if front { color } else { s.plate.egui() }, Stroke::new(2.0, s.hot.egui()));
        if !front {
            painter.circle_stroke(p, 5.0, Stroke::new(2.0, color));
        }
        for i in 0..8 {
            let angle = i as f32 * std::f32::consts::TAU / 8.0;
            let direction = vec2(angle.cos(), angle.sin());
            painter.line_segment([p + direction * 13.0, p + direction * 17.0], Stroke::new(1.5, s.hot.egui()));
        }
        let name = crate::i18n::tr_ctx(crate::i18n::current(), "lighting", "Light");
        self.text(name, s.hot, |size| {
            let name_pos = p - vec2(size.x * 0.5, 25.0 + size.y);
            painter.rect_filled(Rect::from_min_size(name_pos - vec2(4.0, 2.0), size + vec2(8.0, 4.0)), 3.0, s.plate.egui());
            name_pos
        });
        if self.dragging.is_some() {
            let text = format!("{} {:.1}°   {} {:.1}°", tl!("Rotation"), self.values.light_rotation, tl!("Height"), self.values.light_height);
            self.readout(&text, g.center + vec2(0.0, radius + 56.0));
        }
    }

    /// `text` on a plate centred at `at`.
    fn readout(&self, text: &str, at: Pos2) {
        let (s, painter) = (self.style, &self.content.painter);
        self.text(text, s.ink, |size| {
            painter.rect_filled(Rect::from_center_size(at, size + vec2(14.0, 8.0)), 4.0, s.plate.egui());
            at - size * 0.5
        });
    }

    /// Paints `text` in the label type role and `color` at the top left `place` gives for its size; `place` may paint
    /// what goes under the text.
    fn text(&self, text: &str, color: StyleColor, place: impl FnOnce(Vec2) -> Pos2) {
        self.content.with_draw(|draw| {
            let layout = draw.layout(&TextRun::new(text).role(TypeRole::Label).color(color), f32::INFINITY);
            let at = place(layout.size());
            draw.galley(at, &layout);
        });
    }
}

/// The bounds of the first selected object, which the gizmo turns.
fn selected_bounds(app: &VectorcraftApp) -> Option<vectorcraft_geom::Rect> {
    let state = app.session.active()?;
    state.doc.node(*state.selection.objects.first()?)?.geometric_bounds()
}

/// Writes the values `landed` that differ from `before` into the dialog; keys that did not change are left alone.
fn write(dialog: &mut HostedParamDialog, before: GizmoValues, landed: GizmoValues) {
    for (axis, key) in KEYS.iter().enumerate() {
        if landed.rotation[axis] != before.rotation[axis] {
            dialog.set_field(*key, FieldValue::Number(landed.rotation[axis]));
        }
    }
    // The selected light's keys are the light editor's rule (`set_light_value`); only the keys it changed are written.
    let mut fields = dialog.fields().clone();
    for (key, was, now) in
        [("lightRotation", before.light_rotation, landed.light_rotation), ("lightHeight", before.light_height, landed.light_height)]
    {
        if was != now {
            set_light_value(&mut fields, key, json!(now));
        }
    }
    for key in ["lights", "lightRotation", "lightHeight"] {
        if let Some(value) = fields.get(key)
            && dialog.fields().get(key) != Some(value)
        {
            dialog.set_field(key, value.clone());
        }
    }
}

/// Puts the keys the gizmo writes back as `start` held them, value for value: Escape restores the fields as they were,
/// not only their numbers.
fn restore(dialog: &mut HostedParamDialog, start: &DialogFields) {
    for key in KEYS.iter().copied().chain(["lights", "lightRotation", "lightHeight"]) {
        match start.get(key) {
            Some(value) if dialog.fields().get(key) != Some(value) => dialog.set_field(key, value.clone()),
            Some(_) => {}
            None => dialog.clear_field(key),
        }
    }
}

/// The gizmo over the canvas while the 3D effect's dialog shows it, then the Map Art outline: each event's values go
/// to the dialog's fields, which its preview applies.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) {
    if three_d::gizmo_shown(app)
        && let Some(bounds) = selected_bounds(app)
        && let Some(fields) = three_d::with_fields(app, |dialog| dialog.fields().clone())
        && shown(&fields)
    {
        // The dialog is modal and keeps the canvas's view open: the gizmo's presses pass its layer as a canvas tool's do.
        if let Some(canvas) = crate::doc_windows::current(app).map(crate::canvas::canvas_id) {
            craft::ui::runtime::sampler::pass_tool_gesture(ui.ctx(), canvas, craft::ui::contract::CanvasGesture::PRIMARY);
        }
        let before = GizmoValues::of(&fields);
        let label = if lighting(&fields) { tl!("Lighting") } else { tl!("Rotation") };
        let subject = Subject { fields, bounds };
        let output = RevolveGizmo::new(label, Bind::given(before), &subject).show_at(ui, space.view.rect);
        // The fields as the gesture found them, for its cancel.
        let start = egui::Id::unique("revolve-gizmo-start");
        if output.edit.events.began() {
            ui.data_mut(|d| d.insert_temp(start, subject.fields.clone()));
        }
        let cancelled = output.edit.events.cancel_reason().is_some().then(|| ui.data_mut(|d| d.remove_temp::<DialogFields>(start))).flatten();
        if let Some(start) = cancelled {
            three_d::with_fields(app, |dialog| restore(dialog, &start));
        } else if let Some(landed) = output.edit.events.landed().copied() {
            three_d::with_fields(app, |dialog| write(dialog, before, landed));
        }
    }
    if three_d::editing_current_drawing(app) {
        super::three_d_classic::show_outline(app, ui, space);
    }
}

#[cfg(test)]
#[path = "tests_revolve_gizmo.rs"]
mod tests;
