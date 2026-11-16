//! The lighting widget on the 3D and Materials panel's Lighting tab for Extrude & Bevel, Revolve,
//! Inflate and Rotate, and on the Lighting tab of the 3D (Classic) effects in both the panel and the
//! options dialog: a sphere that shows each of the effect's lights
//! at its Rotation and Height, a filled sun in front of the object and a hollow one behind it.
//! The 3D (Classic) effects' lights have no color, so their suns are white.
//! Dragging a light moves it on its side of the object, a press selects it in the list of lights,
//! and the button below the sphere moves the selected light to the back or the front of the object.
//! Each light takes the keyboard focus, where the arrow keys move it, and has a name for screen
//! readers. The widget edits the panel's fields as the light settings do, so the panel previews a
//! drag and records it as one undo step when the pointer is released.

use egui::{Id, Key, Modifiers, Rect, Sense, Stroke, Vec2, vec2};
use serde_json::{Value, json};
use vectorcraft_geom::Point;
use vectorcraft_three_d::{DEFAULT_LIGHT, Revolve};

use super::revolve::{c3, explicit_lights, light_record, set_light_value};
use super::revolve_gizmo::{snap_height, wrap};
use crate::state::Dialog;
use crate::theme::Tokens;

/// The sphere's largest radius, in points.
pub(crate) const RADIUS: f32 = 56.0;
/// The radius of a light's sun.
const SUN: f32 = 7.0;
/// The arrow keys' steps in degrees, without and with Shift.
const STEP: f64 = 1.0;
const SHIFT_STEP: f64 = 15.0;
/// The keys that move the focused light.
const ARROWS: [Key; 4] = [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown];

/// One light as the widget shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Light {
    pub rotation: f64,
    pub height: f64,
    pub behind: bool,
    pub color: egui::Color32,
}

impl Light {
    fn options(self) -> Revolve {
        Revolve { light_rotation: self.rotation, light_height: self.height, light_behind: self.behind, ..Default::default() }
    }

    /// Where the light sits on a sphere of radius 1 around the origin, y down: the direction the
    /// shading uses, seen from the front.
    pub(crate) fn offset(self) -> Vec2 {
        let [x, y, _] = self.options().light_direction();
        vec2(x as f32, y as f32)
    }

    /// The Rotation and Height, rounded to 0.1°, of the light moved to `offset` (radius 1, y down)
    /// on its side of the object: the farther out, the lower. Shift (`snap`) snaps both to 15°.
    pub(crate) fn moved_to(self, offset: Vec2, snap: bool) -> (f64, f64) {
        let [rotation, height] = self.options().light_at_on_side(Point::new(f64::from(offset.x), f64::from(offset.y)));
        (round(wrap(rotation, snap)), round(snap_height(height, snap)))
    }

    /// The Rotation and Height after one press of `key` (Left and Right turn the light around the
    /// object counterclockwise and clockwise, Up and Down raise and lower it). Only the setting the
    /// key moves changes, rounded to 0.1°. `None` for another key.
    fn stepped(self, key: Key, step: f64) -> Option<(f64, f64)> {
        let turn = |by: f64| round(wrap(self.rotation + by, false));
        let raise = |by: f64| round(snap_height(self.height + by, false));
        match key {
            Key::ArrowLeft => Some((turn(step), self.height)),
            Key::ArrowRight => Some((turn(-step), self.height)),
            Key::ArrowUp => Some((self.rotation, raise(step))),
            Key::ArrowDown => Some((self.rotation, raise(-step))),
            _ => None,
        }
    }

    /// The name screen readers give the light at `index`: its number, Rotation, Height and side.
    fn name(self, index: usize) -> String {
        let side = if self.behind { format!(", {}", tl!("Light behind the object")) } else { String::new() };
        format!("{} {}: {} {:.1}°, {} {:.1}°{side}", c3("Light"), index + 1, tl!("Rotation"), self.rotation, tl!("Height"), self.height)
    }
}

fn round(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}

/// The effect's lights: each light of its list, or its single light, with the effect's defaults for
/// the settings a light leaves out or the selected effects hold with different values (null). A 3D
/// (Classic) effect's lights are white whatever color a list light carries, since it shades
/// without light color.
pub(crate) fn lights(d: &Dialog) -> Vec<Light> {
    let effect = d.str("__effect");
    let colored = !vectorcraft_effects::is_classic_three_d(&effect);
    let defaults = vectorcraft_effects::default_params(&effect).unwrap_or(json!({}));
    let list = explicit_lights(d).cloned().unwrap_or_else(|| vec![light_record(d)]);
    list.iter()
        .map(|light| {
            let value =
                |key: &str, field: &str| light.get(key).filter(|v| !v.is_null()).or_else(|| defaults.get(field)).cloned().unwrap_or(Value::Null);
            let color = if colored { value("color", "lightColor") } else { Value::Null };
            let [r, g, b] = color
                .as_str()
                .filter(|s| s.len() == 7 && s.starts_with('#'))
                .and_then(vectorcraft_color::Color::from_hex)
                .unwrap_or(vectorcraft_color::Color::WHITE)
                .to_rgb_uncalibrated()
                .map(|c| (c * 255.0).round() as u8);
            Light {
                rotation: value("rotation", "lightRotation").as_f64().unwrap_or(DEFAULT_LIGHT.0),
                height: value("height", "lightHeight").as_f64().unwrap_or(DEFAULT_LIGHT.1),
                behind: value("behind", "lightBehind").as_bool().unwrap_or(false),
                color: egui::Color32::from_rgb(r, g, b),
            }
        })
        .collect()
}

/// The index of the selected light among `count` lights.
fn selected(d: &Dialog, count: usize) -> usize {
    d.fields.get("__light").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0).min(count.saturating_sub(1))
}

/// Makes light `index` the selected one, as a choice in the list of lights does.
fn select(ui: &egui::Ui, d: &mut Dialog, index: usize) {
    if d.fields.get("__light").and_then(Value::as_u64) != Some(index as u64) {
        ui.memory_mut(|m| m.stop_text_input());
        super::revolve_gizmo::clear(ui.ctx());
    }
    d.fields.insert("__light".into(), json!(index));
}

/// Draws the sphere with the lights and the button that moves the selected light to the back or
/// the front, editing `d.fields`.
pub(super) fn show(ui: &mut egui::Ui, d: &mut Dialog) {
    let lights = lights(d);
    let Some(&first) = lights.first() else { return };
    let current = selected(d, lights.len());
    let t = Tokens::get(ui.ctx());
    let radius = RADIUS.min((ui.available_width() * 0.5 - SUN - 8.0).max(24.0));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), (radius + SUN + 6.0) * 2.0), Sense::hover());
    let center = rect.center();
    let painter = ui.painter_at(rect);
    // The sphere: its rim is Height 0, its center Height 90°, and the ring between them 45°.
    painter.circle(center, radius, t.input, Stroke::new(1.0, t.input_border));
    painter.circle_stroke(center, radius * std::f32::consts::FRAC_1_SQRT_2, Stroke::new(1.0, t.input_border.gamma_multiply(0.5)));
    for axis in [vec2(radius, 0.0), vec2(0.0, radius)] {
        painter.line_segment([center - axis, center + axis], Stroke::new(1.0, t.input_border.gamma_multiply(0.5)));
    }
    painter.circle(center, 4.0, t.panel, Stroke::new(1.0, t.text_dim));
    // The selected light last, so it is drawn above the others and a press there takes it.
    let order = (0..lights.len()).filter(|&i| i != current).chain([current]);
    let mut moved = None;
    let enabled = ui.is_enabled();
    // The panel and a 3D (Classic) options dialog can both show the sphere at once.
    let panel = d.bool("__panel");
    for index in order {
        let light = lights.get(index).copied().unwrap_or(first);
        let at = center + light.offset() * radius;
        let id = Id::unique(("three-d-lighting-widget", panel, index));
        let response = ui.interact(Rect::from_center_size(at, Vec2::splat((SUN + 4.0) * 2.0)), id, Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, enabled, light.name(index)));
        // A light the pointer took, or that the keyboard focus reached, becomes the selected light and
        // keeps the focus, so the arrow keys move it next. Selecting another light stops the editor
        // that had the focus, which clears the focus, so the light asks for it after that.
        if response.drag_started() || response.clicked() || response.gained_focus() {
            select(ui, d, index);
            response.request_focus();
        }
        let origin = ui.input(|i| i.pointer.press_origin());
        if response.drag_started()
            && let Some(origin) = origin
        {
            // The offset from the press to the light's center, kept for the drag, and whether the
            // pointer has left the press yet.
            ui.data_mut(|m| m.insert_temp(id, (at - origin, false)));
        }
        // The frame that releases the pointer can also move it: the light goes where it was dropped,
        // the point of the press included once the pointer has left it.
        if (response.dragged() || response.drag_stopped())
            && let Some(p) = response.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()))
        {
            let (grip, left): (Vec2, bool) = ui.data(|m| m.get_temp(id)).unwrap_or_default();
            let left = left || origin != Some(p);
            ui.data_mut(|m| m.insert_temp(id, (grip, left)));
            if left {
                let snap = ui.input(|i| i.modifiers.shift);
                moved = Some((index, light.moved_to((p + grip - center) / radius, snap)));
            }
        }
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            // Every arrow-key press in the frame moves the light in turn, in the order it came.
            let presses = ui.input_mut(|i| {
                let mut presses = vec![];
                i.events.retain(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. }
                        if ARROWS.contains(key) && (modifiers.matches_exact(Modifiers::NONE) || modifiers.matches_exact(Modifiers::SHIFT)) =>
                    {
                        presses.push((*key, if modifiers.shift { SHIFT_STEP } else { STEP }));
                        false
                    }
                    _ => true,
                });
                presses
            });
            let mut next = light;
            for (key, step) in presses {
                if let Some((rotation, height)) = next.stepped(key, step) {
                    (next.rotation, next.height) = (rotation, height);
                }
            }
            if next != light {
                moved = Some((index, (next.rotation, next.height)));
            }
        }
        paint_light(ui, at, light, index == current, response.has_focus(), response.hovered() || response.dragged());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        response.on_hover_text(tl!("Drag a light to move it. The arrow keys move the focused light; Shift steps 15°."));
    }
    // Only a setting that moved is written: a press that leaves a light where it is, or a key at a
    // limit, changes nothing, and a key leaves the other setting as each selected effect has it.
    if let Some((index, (rotation, height))) = moved
        && let Some(light) = lights.get(index)
    {
        for (key, value, was) in [("lightRotation", rotation, light.rotation), ("lightHeight", height, light.height)] {
            if (value - was).abs() > 1e-9 {
                select(ui, d, index);
                set_light_value(d, key, json!(value));
            }
        }
    }
    let index = selected(d, lights.len());
    let behind = lights.get(index).is_some_and(|l| l.behind);
    ui.horizontal(|ui| {
        let text = if behind { tl!("Move Light to Front") } else { tl!("Move Light to Back") };
        if ui.button(text).clicked() {
            select(ui, d, index);
            set_light_value(d, "lightBehind", json!(!behind));
        }
    });
    ui.add_space(4.0);
}

/// A light's sun: filled with its color in front of the object, hollow behind it, larger and
/// ringed when it is the selected light, with a second ring while it has the keyboard focus.
fn paint_light(ui: &egui::Ui, at: egui::Pos2, light: Light, selected: bool, focused: bool, hot: bool) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let r = if selected { SUN } else { SUN - 2.0 };
    let outline = Stroke::new(if hot { 2.5 } else { 1.5 }, if selected { t.warning } else { t.warning.gamma_multiply(0.7) });
    if light.behind {
        painter.circle(at, r, t.panel, outline);
        painter.circle_stroke(at, r * 0.5, Stroke::new(1.5, light.color));
    } else {
        painter.circle(at, r, light.color, outline);
    }
    if selected {
        for i in 0..8 {
            let angle = i as f32 * std::f32::consts::TAU / 8.0;
            let direction = vec2(angle.cos(), angle.sin());
            painter.line_segment([at + direction * (r + 2.0), at + direction * (r + 4.0)], Stroke::new(1.25, t.warning));
        }
    }
    if focused {
        painter.circle_stroke(at, r + 6.0, Stroke::new(1.0, t.accent));
    }
}
