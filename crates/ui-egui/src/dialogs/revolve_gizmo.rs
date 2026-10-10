//! Artboard rotation and light controls for the active Revolve dialog. Gestures edit fields;
//! the existing effect preview applies commands and owns OK, Cancel and undo.
use egui::{Align2, FontId, Id, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use serde_json::json;
use vectorcraft_effects::{REVOLVE, revolve_options};
use vectorcraft_geom::Point;
use vectorcraft_three_d::Revolve;

use crate::{VectorcraftApp, canvas::Xf, state::Dialog, theme::Tokens};

const KEYS: [&str; 3] = ["rotationX", "rotationY", "rotationZ"];
const SAMPLES: usize = 96;

pub(crate) fn active(d: &Dialog) -> bool {
    d.kind == "effect" && d.str("__effect") == REVOLVE
}

pub(crate) fn lighting(d: &Dialog) -> bool {
    d.str("__gizmo") == "light"
}

pub(crate) fn hint(d: &Dialog) -> &'static str {
    if !d.bool("preview") || (lighting(d) && !d.bool("shade")) {
        tl!("Drag values or labels. Double-click a value to type.")
    } else if lighting(d) {
        tl!("Drag the light, or its rings. Shift snaps to 15°.")
    } else {
        tl!("Drag rings to rotate. Shift snaps to 15°.")
    }
}

fn drag_id() -> Id {
    Id::new("revolve-gizmo-drag")
}

pub(crate) fn dragging(ctx: &egui::Context) -> bool {
    ctx.data(|m| m.get_temp::<Drag>(drag_id())).is_some()
}

pub(super) fn clear(ctx: &egui::Context) {
    ctx.data_mut(|m| m.remove::<Drag>(drag_id()));
}

#[derive(Clone, Copy)]
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

#[derive(Clone, Copy)]
struct Geometry {
    center: Pos2,
    radius: f32,
    rings: [Ring; 3],
    xf: Xf,
    bounds: vectorcraft_geom::Rect,
}

impl Geometry {
    fn new(app: &VectorcraftApp, d: &Dialog) -> Option<Self> {
        let state = app.session.active()?;
        let node = state.doc.node(*state.selection.objects.first()?)?;
        let bounds = node.geometric_bounds()?;
        let xf = Xf::new(app.canvas_rect?, app.view()?);
        let options = revolve_options(&super::form::params(d));
        let center = xf.to_screen(options.rotation_origin(bounds));
        if !center.x.is_finite() || !center.y.is_finite() || !xf.rect.contains(center) {
            return None;
        }
        let radius = (bounds.height().abs() * xf.zoom * 0.24).clamp(76.0, 116.0) as f32;
        let (s, c) = (xf.rot as f32).sin_cos();
        let screen = |p: [f64; 3], scale: f32| vec2(c * p[0] as f32 - s * p[1] as f32, s * p[0] as f32 + c * p[1] as f32) * radius * scale;
        let rings = std::array::from_fn(|i| {
            let [u, v] = options.rotation_planes()[i];
            let scale = if i == 2 { 1.12 } else { 1.0 };
            Ring { center: Vec2::ZERO, u: screen(u, scale), v: screen(v, scale), depth: [u[2] as f32, v[2] as f32] }
        });
        Some(Self { center, radius, rings, xf, bounds })
    }

    fn hit(self, p: Pos2) -> Option<(usize, f32)> {
        let mut best = None;
        let mut distance = 9.0;
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
        let (s, c) = (self.xf.rot as f32).sin_cos();
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
        let mut distance = 9.0;
        for (axis, ring) in self.light_rings(options).iter().enumerate() {
            let (start, span) = if axis == 0 { (0.0, std::f32::consts::TAU) } else { (-std::f32::consts::FRAC_PI_2, std::f32::consts::PI) };
            for i in 0..SAMPLES {
                let a = start + i as f32 * span / SAMPLES as f32;
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
}

#[derive(Clone, Copy)]
enum Mode {
    Ring { axis: usize, ring: Ring, phase: f32, degrees: f64 },
    Orbit,
    Light,
    LightRing { axis: usize },
}

#[derive(Clone)]
struct Drag {
    mode: Mode,
    geometry: Geometry,
    last: Pos2,
    origin: Pos2,
    fields: serde_json::Map<String, serde_json::Value>,
}

fn wrap(n: f64, snap: bool) -> f64 {
    let n = if snap { (n / 15.0).round() * 15.0 } else { n };
    (n + 180.0).rem_euclid(360.0) - 180.0
}

pub(super) fn show(app: &VectorcraftApp, ctx: &egui::Context, d: &mut Dialog) {
    let light_mode = lighting(d);
    if !d.bool("preview") || (light_mode && !d.bool("shade")) {
        clear(ctx);
        return;
    }
    let Some(geometry) = Geometry::new(app, d) else { return };
    let geometry = ctx.data(|m| m.get_temp::<Drag>(drag_id())).map_or(geometry, |drag| drag.geometry);
    let mut rect = Rect::from_center_size(geometry.center, Vec2::splat(geometry.radius * 3.0 + 48.0));
    if light_mode {
        for label in geometry.light_labels() {
            rect = rect.union(label.expand(2.0));
        }
    }
    egui::Area::new(Id::new("revolve-gizmo-area")).order(egui::Order::Foreground).fixed_pos(rect.min).movable(false).show(ctx, |ui| {
        ui.set_clip_rect(geometry.xf.rect);
        let response = ui.allocate_rect(rect.intersect(geometry.xf.rect), Sense::drag());
        let options = revolve_options(&super::form::params(d));
        let light = light_position(geometry, options);
        let hovered = response
            .hovered()
            .then(|| response.hover_pos())
            .flatten()
            .and_then(|p| if light_mode { geometry.light_hit(p, options).map(|axis| (axis, 0.0)) } else { geometry.hit(p) });
        if response.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let mode = if light_mode {
                if origin.distance(light) <= 18.0 {
                    Mode::Light
                } else if let Some(axis) = geometry.light_hit(origin, options) {
                    Mode::LightRing { axis }
                } else {
                    Mode::Light
                }
            } else if origin.distance(geometry.center) <= 14.0 {
                Mode::Orbit
            } else if let Some((axis, phase)) = geometry.hit(origin) {
                Mode::Ring { axis, ring: geometry.rings[axis], phase, degrees: 0.0 }
            } else {
                Mode::Orbit
            };
            ui.ctx().memory_mut(|m| m.stop_text_input());
            ui.data_mut(|m| m.insert_temp(drag_id(), Drag { mode, geometry, last: origin, origin, fields: d.fields.clone() }));
        }
        let mut drag = ui.data(|m| m.get_temp::<Drag>(drag_id()));
        if let Some(current) = &mut drag {
            if ui.input_mut(|i| i.consume_key(i.modifiers, egui::Key::Escape)) {
                d.fields = current.fields.clone();
                d.fields.remove(super::form::PREVIEWED);
                ui.data_mut(|m| m.remove::<Drag>(drag_id()));
                drag = None;
            } else if let Some(p) = response.interact_pointer_pos() {
                let snap = ui.input(|i| i.modifiers.shift);
                match &mut current.mode {
                    Mode::Ring { axis, ring, phase, degrees } => {
                        let step = if let Some(next) = ring.phase(p - geometry.center) {
                            let step = (next - *phase + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                            *phase = next;
                            step
                        } else {
                            let tangent = ring.tangent(*phase);
                            let step = (p - current.last).dot(tangent) / tangent.length_sq().max(144.0);
                            *phase += step;
                            step
                        };
                        *degrees += f64::from(step.to_degrees());
                        if let Some(key) = KEYS.get(*axis) {
                            let start = current.fields.get(*key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            d.fields.insert((*key).into(), json!(wrap(start + *degrees, snap)));
                        }
                    }
                    Mode::Orbit => {
                        let delta = geometry.xf.delta_to_doc(p - current.origin) * geometry.xf.zoom;
                        for (key, delta) in [("rotationX", delta.y * 0.5), ("rotationY", delta.x * 0.5)] {
                            let start = current.fields.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            d.fields.insert(key.into(), json!(wrap(start + delta, snap)));
                        }
                    }
                    Mode::Light => {
                        let start = revolve_options(&serde_json::Value::Object(current.fields.clone()));
                        let projected = light_position(geometry, start) + (p - current.origin);
                        let point = geometry.xf.delta_to_doc(projected - geometry.center) * (geometry.xf.zoom / f64::from(geometry.radius * 1.4));
                        let [azimuth, elevation] = start.light_at_on_side(Point::new(point.x, point.y));
                        d.fields.insert("lightAzimuth".into(), json!(wrap(azimuth, snap)));
                        d.fields.insert(
                            "lightElevation".into(),
                            json!((if snap { (elevation / 15.0).round() * 15.0 } else { elevation }).clamp(-90.0, 90.0)),
                        );
                    }
                    Mode::LightRing { axis } => {
                        let delta = geometry.xf.delta_to_doc(p - current.origin) * geometry.xf.zoom;
                        let start = revolve_options(&serde_json::Value::Object(current.fields.clone()));
                        if *axis == 0 {
                            d.fields.insert("lightAzimuth".into(), json!(wrap(start.light_azimuth + delta.x * 0.6, snap)));
                        } else {
                            let elevation = start.light_elevation - delta.y * 0.6;
                            d.fields.insert(
                                "lightElevation".into(),
                                json!((if snap { (elevation / 15.0).round() * 15.0 } else { elevation }).clamp(-90.0, 90.0)),
                            );
                        }
                    }
                }
                current.last = p;
                ui.data_mut(|m| m.insert_temp(drag_id(), current.clone()));
            }
        }
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|m| m.remove::<Drag>(drag_id()));
        }
        if response.hovered() || drag.is_some() {
            ui.ctx().set_cursor_icon(if drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let active = match drag.as_ref().map(|d| d.mode) {
            Some(Mode::Ring { axis, .. }) => Some(axis),
            Some(Mode::LightRing { axis }) => Some(axis),
            _ => hovered.map(|h| h.0),
        };
        paint(ui, d, Geometry::new(app, d).unwrap_or(geometry), active, drag.as_ref());
        response.on_hover_text(hint(d));
    });
}

fn light_position(g: Geometry, options: Revolve) -> Pos2 {
    let [x, y, _] = options.light_direction();
    let (s, c) = (g.xf.rot as f32).sin_cos();
    g.center + vec2(c * x as f32 - s * y as f32, s * x as f32 + c * y as f32) * g.radius * 1.4
}

fn paint(ui: &egui::Ui, d: &Dialog, g: Geometry, active: Option<usize>, drag: Option<&Drag>) {
    if lighting(d) {
        paint_light(ui, d, g, active, drag);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let colors = [t.axis_x, t.axis_y, t.axis_z];
    for (axis, ring) in g.rings.iter().enumerate() {
        let color = if active == Some(axis) { t.warning } else { colors[axis] };
        for i in 0..SAMPLES {
            let angle = i as f32 * std::f32::consts::TAU / SAMPLES as f32;
            let behind = ring.depth[0] * angle.cos() + ring.depth[1] * angle.sin() < -0.05;
            if behind && i % 3 == 0 {
                continue;
            }
            let points = [g.center + ring.point(angle), g.center + ring.point(angle + std::f32::consts::TAU / SAMPLES as f32)];
            painter.line_segment(points, Stroke::new(4.0, t.input.gamma_multiply(0.55)));
            painter.line_segment(
                points,
                Stroke::new(if active == Some(axis) { 2.5 } else { 1.75 }, color.gamma_multiply(if behind { 0.55 } else { 1.0 })),
            );
        }
        // Letters make the axes distinguishable without relying on colour alone.
        let phase = match axis {
            0 => 0.75,
            1 => 2.25,
            _ => 4.7,
        };
        let p = g.center + ring.point(phase);
        painter.circle_filled(p, 9.0, t.panel);
        painter.text(p, Align2::CENTER_CENTER, ["X", "Y", "Z"][axis], FontId::proportional(11.0), color);
    }
    painter.circle(g.center, 8.0, t.panel.gamma_multiply(0.8), Stroke::new(1.5, t.text_dim));
    painter.line_segment([g.center - vec2(4.0, 0.0), g.center + vec2(4.0, 0.0)], Stroke::new(1.0, t.text_dim));
    painter.line_segment([g.center - vec2(0.0, 4.0), g.center + vec2(0.0, 4.0)], Stroke::new(1.0, t.text_dim));
    let options = revolve_options(&super::form::params(d));
    if d.bool("__showAxis") {
        painter.line_segment(options.projected_axis(g.bounds).map(|p| g.xf.to_screen(p)), Stroke::new(1.0, t.warning));
    }
    if let Some(Drag { mode: Mode::Ring { axis, .. }, .. }) = drag
        && let Some(key) = KEYS.get(*axis)
    {
        let text = format!("{}  {:.1}°", ["X", "Y", "Z"].get(*axis).unwrap_or(&""), d.f64(key, 0.0));
        let pos = g.center + vec2(0.0, g.radius * 1.12 + 18.0);
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), t.text_strong);
        painter.rect_filled(Rect::from_center_size(pos, galley.size() + vec2(14.0, 8.0)), 4.0, t.panel);
        painter.galley(pos - galley.size() * 0.5, galley, t.text_strong);
    }
}

fn paint_light(ui: &egui::Ui, d: &Dialog, g: Geometry, active: Option<usize>, drag: Option<&Drag>) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let options = revolve_options(&super::form::params(d));
    let radius = g.radius * 1.4;
    painter.circle_stroke(g.center, radius, Stroke::new(1.0, t.text_dim.gamma_multiply(0.3)));
    let labels = g.light_labels();
    for (axis, ring) in g.light_rings(options).iter().enumerate() {
        let (start, span) = if axis == 0 { (0.0, std::f32::consts::TAU) } else { (-std::f32::consts::FRAC_PI_2, std::f32::consts::PI) };
        for i in 0..SAMPLES {
            let a = start + i as f32 * span / SAMPLES as f32;
            let behind = ring.depth[0] * a.cos() + ring.depth[1] * a.sin() < -0.05;
            if behind && i % 3 == 0 {
                continue;
            }
            let points = [g.center + ring.point(a), g.center + ring.point(a + span / SAMPLES as f32)];
            painter.line_segment(points, Stroke::new(4.0, t.panel.gamma_multiply(0.4)));
            painter.line_segment(
                points,
                Stroke::new(if active == Some(axis) { 2.5 } else { 1.5 }, t.warning.gamma_multiply(if behind { 0.45 } else { 0.85 })),
            );
        }
        let label = labels[axis];
        painter.rect_filled(label, 4.0, if active == Some(axis) { t.hover } else { t.panel });
        let text = if axis == 0 { tl!("Direction") } else { tl!("Elevation") };
        painter.text(label.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.0), t.warning);
    }
    let p = light_position(g, options);
    let delta = g.center - p;
    if delta.length() > 24.0 {
        let direction = delta.normalized();
        let normal = vec2(-direction.y, direction.x);
        let tip = g.center - direction * 12.0;
        painter.line_segment([p, tip], Stroke::new(1.25, t.warning.gamma_multiply(0.6)));
        painter.line_segment([tip, tip - direction * 8.0 + normal * 4.0], Stroke::new(1.25, t.warning));
        painter.line_segment([tip, tip - direction * 8.0 - normal * 4.0], Stroke::new(1.25, t.warning));
    }
    painter.circle(g.center, 4.0, t.panel, Stroke::new(1.0, t.text_dim));
    // An original sun handle. A back light is hollow; the guide's back half is dashed.
    let front = options.light_direction()[2] >= 0.0;
    painter.circle(p, 9.0, if front { t.warning } else { t.panel }, Stroke::new(2.0, t.warning));
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::TAU / 8.0;
        let direction = vec2(angle.cos(), angle.sin());
        painter.line_segment([p + direction * 13.0, p + direction * 17.0], Stroke::new(1.5, t.warning));
    }
    let name = crate::i18n::tr_ctx(crate::i18n::current(), "lighting", "Light");
    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(11.0), t.warning);
    let name_pos = p - vec2(galley.size().x * 0.5, 25.0 + galley.size().y);
    painter.rect_filled(Rect::from_min_size(name_pos - vec2(4.0, 2.0), galley.size() + vec2(8.0, 4.0)), 3.0, t.panel);
    painter.galley(name_pos, galley, t.warning);
    if drag.is_some() {
        let text = format!("{} {:.1}°   {} {:.1}°", tl!("Direction"), d.f64("lightAzimuth", -45.0), tl!("Elevation"), d.f64("lightElevation", 45.0));
        let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), t.text_strong);
        let pos = g.center + vec2(0.0, radius + 56.0);
        painter.rect_filled(Rect::from_center_size(pos, galley.size() + vec2(12.0, 6.0)), 4.0, t.panel);
        painter.galley(pos - galley.size() * 0.5, galley, t.text_strong);
    }
}

#[cfg(test)]
#[path = "tests_revolve_gizmo.rs"]
mod tests;
