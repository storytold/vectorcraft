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

// PLAN: assumed the app depends on craft-behavior, craft-ui-widgets and craft-ui-patterns (hub-w2 owns Cargo.toml).
use craft::behavior::{CursorIcon, HitLevel, HitPriority, HitShape, Metrics, NavMove, PartKey, PointerButton, PressPhase, Sense};
use craft::ui::buttons::Button;
use craft::ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DisabledLook, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Outcome, PaintUi,
    PartsCx, Pointer as _, Props as _, Remember as _, Role, Show as _,
};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields};
use craft::ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft::ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Stroke, Vec2, vec2};
use serde_json::{Value, json};
use vectorcraft_geom::Point;
use vectorcraft_three_d::{DEFAULT_LIGHT, Revolve};

use super::revolve::{c3, explicit_lights, light_record, set_light_value};
use super::revolve_gizmo::{snap_height, wrap};

/// The sphere's largest radius, in points.
pub(crate) const RADIUS: f32 = 56.0;
/// The radius of a light's sun.
const SUN: f32 = 7.0;
/// The arrow keys' steps in degrees, without and with Shift.
const STEP: f64 = 1.0;
const SHIFT_STEP: f64 = 15.0;
/// One light of the sphere: `LIGHT.at(i)` with `i` its place in the list of lights.
const LIGHT: PartKey = PartKey::new("light");

/// The lights the sphere shows, in the order of the list of lights.
pub(crate) type Lights = Vec<Light>;

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

    /// The Rotation and Height after one press of `key` (Back and Forward turn the light around the
    /// object counterclockwise and clockwise, Up and Down raise and lower it). Only the setting the
    /// key moves changes, rounded to 0.1°. `None` for another key.
    fn stepped(self, key: NavMove, step: f64) -> Option<(f64, f64)> {
        let turn = |by: f64| round(wrap(self.rotation + by, false));
        let raise = |by: f64| round(snap_height(self.height + by, false));
        match key {
            NavMove::Back => Some((turn(step), self.height)),
            NavMove::Forward => Some((turn(-step), self.height)),
            NavMove::Up => Some((self.rotation, raise(step))),
            NavMove::Down => Some((self.rotation, raise(-step))),
            _ => None,
        }
    }

    /// The name screen readers give the light at `index`: its number, Rotation, Height and side.
    fn name(self, index: usize) -> String {
        let side = if self.behind { format!(", {}", tl!("Light behind the object")) } else { String::new() };
        let light = crate::i18n::colon(&format!("{} {}", c3("Light"), index + 1));
        format!("{light} {} {:.1}°, {} {:.1}°{side}", tl!("Rotation"), self.rotation, tl!("Height"), self.height)
    }
}

fn round(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}

/// The effect's lights: each light of its list, or its single light, with the effect's defaults for
/// the settings a light leaves out or the selected effects hold with different values (null). A 3D
/// (Classic) effect's lights are white whatever color a list light carries, since it shades
/// without light color.
pub(crate) fn lights(fields: &DialogFields) -> Lights {
    let classic = vectorcraft_effects::is_classic_three_d(fields.text("__effect").unwrap_or_default());
    lights_for(fields, classic)
}

/// [`lights`] for a light editor that already knows whether it edits a 3D (Classic) effect.
fn lights_for(fields: &DialogFields, classic: bool) -> Lights {
    let defaults = vectorcraft_effects::default_params(fields.text("__effect").unwrap_or_default()).unwrap_or(json!({}));
    let list = explicit_lights(fields).unwrap_or_else(|| vec![light_record(fields)]);
    list.iter()
        .map(|light| {
            let value =
                |key: &str, field: &str| light.get(key).filter(|v| !v.is_null()).or_else(|| defaults.get(field)).cloned().unwrap_or(Value::Null);
            let color = if classic { Value::Null } else { value("color", "lightColor") };
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
fn selected(fields: &DialogFields, count: usize) -> usize {
    fields.int("__light").and_then(|v| usize::try_from(v).ok()).unwrap_or(0).min(count.saturating_sub(1))
}

/// Makes light `index` the selected one, as a choice in the list of lights does.
fn select(fields: &mut DialogFields, index: usize) {
    fields.set("__light", index as i64);
}

/// Writes the settings of `to` that differ from `from` into the selected light, selecting the light that moved. Only a setting
/// that moved is written: a press that leaves a light where it is, or a key at a limit, changes nothing, and a key leaves the
/// other setting as each selected effect has it.
pub(super) fn write_moved(fields: &mut DialogFields, from: &[Light], to: &[Light]) {
    for (index, (was, now)) in from.iter().zip(to).enumerate() {
        for (key, value, old) in [("lightRotation", now.rotation, was.rotation), ("lightHeight", now.height, was.height)] {
            if (value - old).abs() > 1e-9 {
                select(fields, index);
                set_light_value(fields, key, json!(value));
            }
        }
    }
}

/// The style of a part-based dialog row control (the light sphere, the track cube): the well it is drawn in and the
/// colours of its marks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct RowStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The well the control is drawn in, and its edge.
    pub(super) well: StyleColor,
    pub(super) edge: StyleColor,
    /// A light behind the object, and the centre mark.
    pub(super) panel: StyleColor,
    /// Guides and the cube's edges at rest.
    pub(super) muted: StyleColor,
    /// The lights' rims and rays.
    pub(super) light: StyleColor,
    /// What has the keyboard focus.
    pub(super) accent: StyleColor,
    /// The X, Y and Z axes.
    pub(super) axis_x: StyleColor,
    pub(super) axis_y: StyleColor,
    pub(super) axis_z: StyleColor,
}

impl RowStyle {
    /// The style from the theme.
    pub(super) fn of(cx: &StyleCx<'_>) -> Self {
        let colors = &cx.tokens.color;
        RowStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            well: cx.color(colors.surface.sunken),
            edge: cx.color(colors.line.control_edge),
            panel: cx.color(colors.surface.panel),
            muted: cx.color(colors.text.muted),
            light: cx.color(colors.status.warn.solid),
            accent: cx.color(colors.accent.solid),
            axis_x: cx.color(colors.data.red),
            axis_y: cx.color(colors.data.green),
            axis_z: cx.color(colors.data.blue),
        }
    }
}

/// The radius of the sphere in a row `width` points wide: it stays inside a narrow panel.
fn radius_in(width: f32) -> f32 {
    RADIUS.min((width * 0.5 - SUN - 8.0).max(24.0))
}

/// Where the sphere's centre and radius are in `rect`.
fn geometry(rect: Rect) -> (Pos2, f32) {
    (rect.center(), radius_in(rect.width()))
}

/// What a press on a light holds while it runs: the light and the offset from the press to the light's centre, kept for the
/// drag.
#[derive(Clone, Copy, Default)]
struct Hold {
    index: usize,
    grip: Vec2,
}

/// The sphere of the lights: one slider part per light that a drag moves on its side of the object and the arrow keys step.
/// The value is the list of lights; a drag proposes it with the dragged light moved and Escape restores the start.
///
/// - Pointer: a press selects the light and takes the focus; the light follows the pointer once it has left the press point,
///   keeping the offset between the press and its centre; Shift snaps to 15°. The selected light is above the others.
/// - Keyboard: each light is a Tab stop; Left and Right turn the light, Up and Down raise and lower it, 1° a press and 15°
///   with Shift, each press one step.
/// - AccessKit: a `Slider` per light named by its number, Rotation, Height and side.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(content_painter)]
pub(crate) struct LightSphere<'a> {
    #[common]
    common: Common,
    lights: Bind<'a, Lights>,
    /// The selected light, drawn larger and ringed.
    #[prop]
    selected: usize,
    /// The light a press selected this pass.
    picked: Option<usize>,
    /// The light the pointer is over or holds.
    hot: Option<usize>,
}

impl<'a> LightSphere<'a> {
    /// A sphere named `label` showing `lights`.
    pub(crate) fn new(label: impl Into<craft::ui::runtime::env::Label>, lights: Bind<'a, Lights>) -> Self {
        LightSphere { common: Common::new(), lights, selected: 0, picked: None, hot: None }.label(label)
    }
}

/// What a [`LightSphere`] returns.
#[derive(Debug)]
pub(crate) struct LightSphereOutput {
    /// The edit of the lights.
    pub edit: EditResponse<Lights>,
    /// The light a press or the keyboard focus took this pass.
    pub picked: Option<usize>,
}

impl Outcome for LightSphereOutput {
    type Value = Lights;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

impl Component for LightSphere<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const SINGLE_VALUE: bool = true;
    const SHOWS_LABEL: bool = false;
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither).with_moves();
    const OVERLAYS: bool = false;
    type Style = RowStyle;
    type Output = LightSphereOutput;

    fn style(cx: &StyleCx<'_>, _: LightSphereStyleKey) -> RowStyle {
        RowStyle::of(cx)
    }

    /// The row's width and the height of the sphere that fits it, with its suns.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = cx.picture_width((24.0 + SUN + 8.0) * 2.0, f32::INFINITY);
        Vec2::new(width, (radius_in(width) + SUN + 6.0) * 2.0)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (center, radius) = geometry(rect);
        let lights: Lights = cx.value().cloned().unwrap_or_default();
        for (index, light) in lights.iter().enumerate() {
            // The selected light wins an overlap, as it is drawn last.
            let level = u8::from(index == self.selected);
            cx.part(LIGHT.at(index as u64), HitShape::circle(center + light.offset() * radius, SUN + 4.0))
                .sense(Sense::DRAG | Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Handle, level))
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .tooltip(tl!("Drag a light to move it. The arrow keys move the focused light; Shift steps 15°."))
                .focusable();
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Lights>> {
        Some(self.lights.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (center, radius) = geometry(cx.rect);
        let lights: Lights = cx.value().cloned().unwrap_or_default();
        let mut held = *cx.press_slot::<Hold>();
        for (index, light) in lights.iter().enumerate() {
            let part = LIGHT.at(index as u64);
            let Some(result) = cx.part_if_declared(part) else { continue };
            let (press, hovered) = (result.press, result.hovered);
            if hovered {
                self.hot = Some(index);
            }
            // The light goes where the pointer carries it, by the offset the press had from its centre.
            let moved = |hold: &Hold, pos: Pos2| {
                let to = light.moved_to((pos + hold.grip - center) / radius, press.mods.shift);
                let mut next = lights.clone();
                if let Some(l) = next.get_mut(index) {
                    (l.rotation, l.height) = to;
                }
                next
            };
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    cx.focus_part(part);
                    self.picked = Some(index);
                    held = Some(Hold { index, grip: center + light.offset() * radius - start.origin });
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let Some(hold) = held.filter(|h| h.index == index) else { continue };
                    if matches!(press.phase, PressPhase::DragStart { .. }) {
                        cx.begin();
                    }
                    self.hot = Some(index);
                    cx.propose(moved(&hold, pos));
                }
                PressPhase::Release { pos, .. } => {
                    let Some(hold) = held.filter(|h| h.index == index) else { continue };
                    // A click that never left the press point moves nothing: no gesture ran.
                    if cx.gesture().is_some() {
                        cx.propose(moved(&hold, pos));
                        cx.commit();
                    }
                    held = None;
                }
                PressPhase::Cancel { .. } => held = None,
                _ => {}
            }
        }
        *cx.press_slot::<Hold>() = held;
        if let Some(arrived) = cx.focus_arrived().filter(|part| part.key == LIGHT) {
            self.picked = arrived.index.and_then(|i| usize::try_from(i).ok());
        }
        let focused = cx.active_part().filter(|part| part.key == LIGHT).and_then(|part| part.index).and_then(|i| usize::try_from(i).ok());
        let mut next = lights.clone();
        while let Some((key, mods)) = cx.intents().take_move() {
            // Only a plain or Shift press steps; every press of the pass moves the light in turn.
            if mods.lead || mods.fine || cx.gesture().is_some() {
                continue;
            }
            let Some(index) = focused else { continue };
            let step = if mods.extend { SHIFT_STEP } else { STEP };
            if let Some(light) = next.get_mut(index)
                && let Some((rotation, height)) = light.stepped(key, step)
            {
                (light.rotation, light.height) = (rotation, height);
                cx.set(next.clone());
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        for (index, light) in cx.value().map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
            access.node(LIGHT.at(index as u64), Role::SLIDER).name = Name::Text(light.name(index));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (center, radius) = geometry(cx.rect);
        let content = ui.content_painter(cx);
        let painter = &content.painter;
        let s = cx.style;
        let edge = s.edge.egui();
        // The sphere: its rim is Height 0, its center Height 90°, and the ring between them 45°.
        painter.circle(center, radius, s.well.egui(), Stroke::new(1.0, edge));
        painter.circle_stroke(center, radius * std::f32::consts::FRAC_1_SQRT_2, Stroke::new(1.0, edge.gamma_multiply(0.5)));
        for axis in [vec2(radius, 0.0), vec2(0.0, radius)] {
            painter.line_segment([center - axis, center + axis], Stroke::new(1.0, edge.gamma_multiply(0.5)));
        }
        painter.circle(center, 4.0, s.panel.egui(), Stroke::new(1.0, s.muted.egui()));
        let lights: &[Light] = cx.value().map(Vec::as_slice).unwrap_or_default();
        // The selected light last, so it is drawn above the others.
        let order = (0..lights.len()).filter(|&i| i != self.selected).chain([self.selected]);
        for index in order {
            let Some(&light) = lights.get(index) else { continue };
            let focused = cx.is_focused() && cx.active_part() == Some(LIGHT.at(index as u64));
            paint_light(painter, s, center + light.offset() * radius, light, index == self.selected, focused, self.hot == Some(index));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> LightSphereOutput {
        LightSphereOutput { edit: cx.edit_response(), picked: self.picked }
    }
}

/// Draws the sphere with the lights and the button that moves the selected light to the back or
/// the front, editing `fields`. `classic` is whether the effect is a 3D (Classic) one, whose lights are white. The control's
/// events go to the rows through `cx`, so the panel and the dialog's preview follow a drag.
pub(crate) fn sphere(ui: &mut egui::Ui, fields: &mut DialogFields, cx: &mut CustomContext, classic: bool) -> Response {
    let list = lights_for(fields, classic);
    let current = selected(fields, list.len());
    let out = LightSphere::new(tl!("Lights"), Bind::given(list.clone())).selected(current).show(ui);
    cx.gesture(&out.edit.events);
    if let Some(index) = out.picked {
        select(fields, index);
    }
    // What each event leaves in the lights, a cancel's restored start included, each from where the one before left
    // them (Down then Up in one pass lands back where it began).
    let mut from = list.clone();
    for lights in out.edit.events.iter().filter_map(|event| event.landed()) {
        write_moved(fields, &from, lights);
        from.clone_from(lights);
    }
    let current = selected(fields, list.len());
    let behind = list.get(current).is_some_and(|l| l.behind);
    let text = if behind { tl!("Move Light to Front") } else { tl!("Move Light to Back") };
    if Button::new(text).show(ui).clicks > 0 {
        select(fields, current);
        set_light_value(fields, "lightBehind", json!(!behind));
        // The rows hear of a custom row's change through its events: the move is one step.
        let mut moved = craft::behavior::EditEvents::new();
        moved.push(craft::behavior::EditEvent::set(craft::behavior::Mixed::Same(behind), !behind, text.into(), None));
        cx.gesture(&moved);
    }
    ui.add_space(4.0);
    out.edit.response
}

/// A light's sun: filled with its color in front of the object, hollow behind it, larger and
/// ringed when it is the selected light, with a second ring while it has the keyboard focus.
fn paint_light(painter: &egui::Painter, s: &RowStyle, at: Pos2, light: Light, selected: bool, focused: bool, hot: bool) {
    let r = if selected { SUN } else { SUN - 2.0 };
    let rim = s.light.egui();
    let outline = Stroke::new(if hot { 2.5 } else { 1.5 }, if selected { rim } else { rim.gamma_multiply(0.7) });
    if light.behind {
        painter.circle(at, r, s.panel.egui(), outline);
        painter.circle_stroke(at, r * 0.5, Stroke::new(1.5, light.color));
    } else {
        painter.circle(at, r, light.color, outline);
    }
    if selected {
        for i in 0..8 {
            let angle = i as f32 * std::f32::consts::TAU / 8.0;
            let direction = vec2(angle.cos(), angle.sin());
            painter.line_segment([at + direction * (r + 2.0), at + direction * (r + 4.0)], Stroke::new(1.25, rim));
        }
    }
    if focused {
        painter.circle_stroke(at, r + 6.0, Stroke::new(1.0, s.accent.egui()));
    }
}
