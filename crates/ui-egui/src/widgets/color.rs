use super::labels::{label_field, name_field, take_previous_label};
use super::numeric::{STEP_DECIMALS, STEPS, field_text, round_to, step_to_multiple};
use super::text::{after_shift_enter, select_all, shift_enter_keeps_focus};
use super::{Color, Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, id_with, pos2, vec2};

/// Drag phase of a live slider/handle: previews while dragging, commits on release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Live {
    Idle,
    Dragging,
    Released,
}

/// A colour slider with a gradient track (`track(t)` gives the colour at 0..1) and a triangular
/// thumb below it, like Illustrator's Color panel. Returns the new normalized value and the phase.
pub fn color_slider(ui: &mut Ui, id: impl std::hash::Hash, value: f32, width: f32, track: &dyn Fn(f32) -> Color32) -> (Option<f32>, Live) {
    color_slider_named(ui, id, "", value, width, track)
}

/// [`color_slider`] that screen readers get as a slider named `name` (already translated) with no
/// value (none when `name` is empty).
pub fn color_slider_named(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: &str,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    color_slider_valued(ui, id, name, None, value, width, track)
}

/// The value a slider gives screen readers: the number the field beside it shows, the range of the
/// slider's rail and the field's text.
#[derive(Clone, Debug)]
pub struct SliderValue {
    pub value: f64,
    pub range: std::ops::RangeInclusive<f64>,
    /// The field's text, such as "50%"; `None` gives the number alone.
    pub text: Option<String>,
    /// How the keys and screen readers step the number ([`color_slider_stepped`]): the number `n`
    /// steps from `value` (±1 for an arrow, ±10 with Shift or Page Up/Down). `None` steps to whole
    /// numbers.
    pub step: Option<fn(f64, f64) -> f64>,
}

/// Two values are the same when they show the same number, range and text, whatever their keys' steps
/// (function pointers don't compare reliably).
impl PartialEq for SliderValue {
    fn eq(&self, other: &Self) -> bool {
        (self.value, &self.range, &self.text) == (other.value, &other.range, &other.text)
    }
}

impl SliderValue {
    /// `value` on a rail over `range`, without a text.
    pub fn number(value: f64, range: std::ops::RangeInclusive<f64>) -> Self {
        Self { value, range, text: None, step: None }
    }

    /// `value` on a rail over `range` as [`plain_field`] shows it with `suffix` and at most
    /// `decimals` places: the number it shows (`value` rounded) and its text.
    pub fn field(value: f64, range: std::ops::RangeInclusive<f64>, suffix: &str, decimals: usize) -> Self {
        let number = field_text(value, "", decimals);
        Self { value: number.parse().unwrap_or(value), range, text: Some(format!("{number}{suffix}")), step: None }
    }

    /// `value` on a rail over `range`, with `text`, for a field that writes its own text (a length
    /// in the document's unit, an egui `DragValue`).
    pub fn text(value: f64, range: std::ops::RangeInclusive<f64>, text: String) -> Self {
        Self { value, range, text: Some(text), step: None }
    }

    /// This value, stepped by the keys with `step` instead of to whole numbers (see
    /// [`SliderValue::step`]), for a rail whose useful numbers are fractions.
    pub fn with_step(self, step: fn(f64, f64) -> f64) -> Self {
        Self { step: Some(step), ..self }
    }
}

/// Give `resp` a slider's screen-reader node named `name` (nothing when `name` is empty), with
/// `shown`'s number, text and range: the rail's range, extended to the number when the field holds
/// a number past the rail's end.
fn slider_info(ui: &Ui, resp: &Response, name: &str, shown: Option<SliderValue>) {
    if name.is_empty() {
        return;
    }
    let enabled = ui.is_enabled();
    let Some(s) = shown else {
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, enabled, name));
        return;
    };
    resp.widget_info(|| egui::WidgetInfo { current_text_value: s.text.clone(), ..egui::WidgetInfo::slider(enabled, s.value, name) });
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_min_numeric_value(s.range.start().min(s.value));
        node.set_max_numeric_value(s.range.end().max(s.value));
    });
}

/// [`color_slider_named`] that screen readers also get `shown` from (see [`SliderValue`]). With
/// `None`, as for a field left blank for a mixed value, it reports no value.
pub fn color_slider_valued(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: &str,
    shown: Option<SliderValue>,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    let id = id_with(ui.scope_id(), id);
    let (_, picked, phase) = color_rail(ui, id, name, shown, value, width, track);
    (picked, phase)
}

/// [`color_slider_valued`] for a dialog's rail beside a number field, which also responds to the
/// keyboard. A click or a drag gives the rail the keyboard focus; an outline around it shows the
/// focus once a key has stepped it, or when the focus came from the keyboard (Tab).
/// While it has the focus, → and ↑ raise the number `shown` holds to the next whole number and ←
/// and ↓ lower it to the next whole number, Shift with an arrow, Page Up and Page Down add or take
/// ten, and Home and End set the start and the end of `shown`'s range. Screen readers' increment
/// and decrement actions step the number as → and ← do. A number past the range steps from the
/// range's nearer end, and no step goes past either end. With `shown` `None` (a blank field), or
/// in a disabled `ui`, the keys and actions leave the rail as it is. Returns the new value from 0
/// to 1 that a click or a drag gives, its phase, and the number in `shown`'s units that a key or
/// an action gives.
pub fn color_slider_stepped(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: &str,
    shown: Option<SliderValue>,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live, Option<f64>) {
    let id = id_with(ui.scope_id(), id);
    let (resp, picked, phase) = color_rail(ui, id, name, shown.clone(), value, width, track);
    // Focus a click or a drag gave shows no outline until a key steps the rail: the outline tells
    // a keyboard user where the focus is, and after a drag it only framed the rail.
    let by_pointer = id.with("focused-by-pointer");
    if resp.drag_started() || resp.clicked() {
        resp.request_focus();
        ui.data_mut(|d| d.insert_temp(by_pointer, true));
    }
    let focused = ui.memory(|m| m.has_focus(id));
    if !focused {
        ui.data_mut(|d| d.remove_temp::<bool>(by_pointer));
    }
    let stepped = shown.filter(|_| ui.is_enabled()).and_then(|s| {
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_numeric_value_step(1.0);
            node.add_action(egui::accesskit::Action::Increment);
            node.add_action(egui::accesskit::Action::Decrement);
        });
        rail_step(ui, id, focused, &s)
    });
    if stepped.is_some() {
        ui.data_mut(|d| d.remove_temp::<bool>(by_pointer));
    }
    if focused && !ui.data(|d| d.get_temp::<bool>(by_pointer)).unwrap_or(false) {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(resp.rect, 2.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    (picked, phase, stepped.filter(|_| picked.is_none()))
}

/// The number of slider `id` (`s`) after this frame's keys while it is `focused`, and after screen
/// readers' actions on it ([`color_slider_stepped`]); `None` when they leave it as it was.
fn rail_step(ui: &Ui, id: egui::Id, focused: bool, s: &SliderValue) -> Option<f64> {
    use egui::accesskit::Action;
    use egui::{Event, Key};
    let (lo, hi) = (*s.range.start(), *s.range.end());
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return None;
    }
    let from = s.value.clamp(lo, hi);
    let mut v = from;
    // `n` steps from `v`: the rail's own steps, or to whole numbers (ten at a time with Shift).
    let by = |v: f64, n: f64| match s.step {
        Some(step) => step(v, n),
        None if n.abs() >= 10.0 => v + n,
        None => step_to_multiple(v, 1.0, n),
    };
    // Home or End: an end of the range, which also replaces a number past it.
    let mut end = false;
    if focused {
        // The arrows step the rail and do not move the focus. egui keeps the filter only when the
        // rail also had the focus in the frame before, and applies it from the next frame; until
        // then, an arrow the rail takes cancels the focus move that egui set at the start of the frame.
        let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() };
        ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
        let mut arrows = false;
        ui.input_mut(|i| {
            i.events.retain(|e| {
                let Event::Key { key, pressed: true, modifiers, .. } = e else { return true };
                let up = matches!(key, Key::ArrowRight | Key::ArrowUp | Key::PageUp);
                let n = if up { 1.0 } else { -1.0 };
                match key {
                    Key::Home | Key::End if modifiers.is_none() => {
                        v = if *key == Key::Home { lo } else { hi };
                        end = true;
                    }
                    Key::PageUp | Key::PageDown if modifiers.is_none() => v = by(v, 10.0 * n).clamp(lo, hi),
                    Key::ArrowRight | Key::ArrowUp | Key::ArrowLeft | Key::ArrowDown if modifiers.shift_only() => {
                        v = by(v, 10.0 * n).clamp(lo, hi);
                    }
                    Key::ArrowRight | Key::ArrowUp | Key::ArrowLeft | Key::ArrowDown if modifiers.is_none() => {
                        v = by(v, n).clamp(lo, hi);
                        arrows = true;
                    }
                    _ => return true,
                }
                false
            });
        });
        if arrows {
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
    }
    ui.input(|i| {
        let n = i.num_accesskit_action_requests(id, Action::Increment) as f64 - i.num_accesskit_action_requests(id, Action::Decrement) as f64;
        v = by(v, n).clamp(lo, hi);
    });
    (v != from || (end && v != s.value)).then_some(round_to(v, STEP_DECIMALS).clamp(lo, hi))
}

/// The rail of [`color_slider_valued`] with the id `id`: its response, the new value from 0 to 1
/// that a click or drag gives and the phase.
fn color_rail(
    ui: &mut Ui,
    id: egui::Id,
    name: &str,
    shown: Option<SliderValue>,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Response, Option<f32>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let bar = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 4.0), vec2(width - 8.0, 7.0));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    gradient_strip(ui, bar, (bar.width() / 2.0).ceil().max(2.0) as usize, false, track);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let v = value.clamp(0.0, 1.0);
    let x = bar.left() + v * bar.width();
    let tip = pos2(x, bar.bottom() - 1.0);
    let thumb = vec![tip, pos2(x + 5.5, tip.y + 6.0), pos2(x + 5.5, tip.y + 10.0), pos2(x - 5.5, tip.y + 10.0), pos2(x - 5.5, tip.y + 6.0)];
    let fill = if resp.dragged() || resp.hovered() { Color32::WHITE } else { t.icon };
    ui.painter().add(egui::Shape::convex_polygon(thumb, fill, Stroke::new(1.0, t.border)));
    slider_info(ui, &resp, name, shown);
    let (picked, phase) = match pointer_phase(&resp) {
        Some((p, phase)) => (Some(((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0)), phase),
        None => (None, Live::Idle),
    };
    (resp, picked, phase)
}

/// The pointer of a click or drag on `resp` and its phase (dragging, or released on click/drop).
pub(crate) fn pointer_phase(resp: &Response) -> Option<(Pos2, Live)> {
    if !(resp.dragged() || resp.clicked() || resp.drag_stopped()) {
        return None;
    }
    let p = resp.interact_pointer_pos()?;
    Some((p, if resp.dragged() && !resp.drag_stopped() { Live::Dragging } else { Live::Released }))
}

/// A gradient strip into `rect`: `n` bands along x (or up y when `vertical`), each filled with
/// `track(t)` at its centre.
fn gradient_strip(ui: &Ui, rect: Rect, n: usize, vertical: bool, track: &dyn Fn(f32) -> Color32) {
    let len = if vertical { rect.height() } else { rect.width() };
    let seg = len / n as f32;
    for i in 0..n {
        let r = if vertical {
            Rect::from_min_size(pos2(rect.left(), rect.bottom() - (i + 1) as f32 * seg), vec2(rect.width(), seg + 0.6))
        } else {
            Rect::from_min_size(pos2(rect.left() + i as f32 * seg, rect.top()), vec2(seg + 0.6, rect.height()))
        };
        ui.painter().rect_filled(r, 0.0, track((i as f32 + 0.5) / n as f32));
    }
}

/// One axis of a [`color_field`] as keys and screen readers see it: the component's name, how many
/// of its units the axis spans (an arrow press moves one unit) and the units' suffix.
#[derive(Clone, Copy, Debug)]
pub struct FieldAxis<'a> {
    pub name: &'a str,
    pub units: f32,
    pub suffix: &'a str,
}

/// A press on `resp`, or a click whose press and release came in one frame, gives it the keyboard
/// focus, so the arrow keys reach it. Escape or a click elsewhere takes the focus away again.
fn focus_on_press(resp: &Response) {
    if resp.is_pointer_button_down_on() || resp.clicked() {
        resp.request_focus();
    }
}

/// The arrow keys pressed for control `id` this frame while it has the keyboard focus, in units
/// ([`STEPS`]: ten with Shift, a tenth with Ctrl/Cmd, else one): (→ minus ←, ↑ minus ↓). The control
/// takes the arrows of each axis it uses (`horizontal`, `vertical`), which then do not move the focus
/// to the next widget.
fn arrow_steps(ui: &Ui, id: egui::Id, horizontal: bool, vertical: bool) -> (f32, f32) {
    if !ui.memory(|m| m.has_focus(id)) {
        return (0.0, 0.0);
    }
    use egui::Key;
    let filter = egui::EventFilter { horizontal_arrows: horizontal, vertical_arrows: vertical, ..Default::default() };
    ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
    let (mut x, mut y) = (0.0, 0.0);
    ui.input_mut(|i| {
        for (mods, size) in STEPS {
            let mut count = |key| i.count_and_consume_key(mods, key) as f64 * size;
            if horizontal {
                x += count(Key::ArrowRight) - count(Key::ArrowLeft);
            }
            if vertical {
                y += count(Key::ArrowUp) - count(Key::ArrowDown);
            }
        }
    });
    (x as f32, y as f32)
}

/// The focus outline of a focused color control.
fn focus_outline(ui: &Ui, resp: &Response, rect: Rect) {
    if ui.memory(|m| m.has_focus(resp.id)) {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(rect.expand(2.0), 0.0, Stroke::new(1.0, t.accent), StrokeKind::Outside);
    }
}

/// A 2D color field (the Color Picker's, the Color Wheel panel's square): `color_at(x, y)` gives
/// the color at a point (0..1 each, y down) and, when `marked`, a ring marks `pos`. A press gives
/// it the keyboard focus. While it has the focus, → and ← move `pos` across by one unit of
/// `axes[0]` and ↑ and ↓ move it up and down by one unit of `axes[1]`, marked or not. Screen
/// readers read `name` and, when `marked`, both components. They read it as a slider over
/// `axes[0]`'s units (0 when not `marked`), which their increment and decrement actions move as →
/// and ← do and their set-value action sets, as for a [`hue_ring`]. Returns the clicked or dragged
/// point, or the point the keys or actions moved to, and the phase. When `marked`, a step past an
/// edge returns no point.
pub fn color_field(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    (name, axes): (&str, [FieldAxis<'_>; 2]),
    size: Vec2,
    pos: (f32, f32),
    marked: bool,
    color_at: &dyn Fn(f32, f32) -> Color32,
) -> (Option<(f32, f32)>, Live) {
    use egui::accesskit::{Action, ActionData};
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, id_with(ui.scope_id(), id), Sense::click_and_drag());
    focus_on_press(&resp);
    let painter = ui.painter_at(rect.expand(6.0));
    painter.add(color_mesh(rect, (32, 32), color_at));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    focus_outline(ui, &resp, rect);
    let (x, y) = pos;
    if marked {
        let c = pos2(rect.left() + x.clamp(0.0, 1.0) * rect.width(), rect.top() + y.clamp(0.0, 1.0) * rect.height());
        painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::BLACK));
        painter.circle_stroke(c, 4.0, Stroke::new(1.0, Color32::WHITE));
    }
    let value = |v: f32, a: &FieldAxis| format!("{}{}", (v.clamp(0.0, 1.0) * a.units).round(), a.suffix);
    let text = marked.then(|| {
        crate::i18n::fmt(
            tl!("{name1} {value1}, {name2} {value2}"),
            &[("name1", axes[0].name), ("value1", &value(x, &axes[0])), ("name2", axes[1].name), ("value2", &value(1.0 - y, &axes[1]))],
        )
    });
    let shown = if marked { f64::from(x.clamp(0.0, 1.0) * axes[0].units) } else { 0.0 };
    resp.widget_info(|| egui::WidgetInfo { current_text_value: text.clone(), ..egui::WidgetInfo::slider(ui.is_enabled(), shown, name) });
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(f64::from(axes[0].units));
        node.set_numeric_value_step(1.0);
        for action in [Action::SetValue, Action::Increment, Action::Decrement] {
            node.add_action(action);
        }
    });
    let (dx, dy) = arrow_steps(ui, resp.id, true, true);
    let (steps, set) = ui.input(|i| {
        let steps =
            i.num_accesskit_action_requests(resp.id, Action::Increment) as f32 - i.num_accesskit_action_requests(resp.id, Action::Decrement) as f32;
        let set = i.accesskit_action_requests(resp.id, Action::SetValue).find_map(|r| match r.data {
            Some(ActionData::NumericValue(v)) if v.is_finite() => Some(v as f32),
            _ => None,
        });
        (steps, set)
    });
    if let Some((p, phase)) = pointer_phase(&resp) {
        let f = |v: f32, lo: f32, len: f32| ((v - lo) / len).clamp(0.0, 1.0);
        return (Some((f(p.x, rect.left(), rect.width()), f(p.y, rect.top(), rect.height()))), phase);
    }
    let across = set.map_or(x + (dx + steps) / axes[0].units, |v| v / axes[0].units);
    let moved = (across.clamp(0.0, 1.0), (y - dy / axes[1].units).clamp(0.0, 1.0));
    // A step past an edge leaves the point where it is. Without a marker there is no color yet, so
    // a key or an action applies even when the point stays.
    let asked = set.is_some() || dx + steps != 0.0 || dy != 0.0;
    if moved != (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)) || (asked && !marked) {
        return (Some(moved), Live::Released);
    }
    (None, Live::Idle)
}

/// A smoothly shaded colour area: a `cols` × `rows` grid over `rect` with `color_at(x, y)` (0..1
/// each, y down) at its vertices.
pub fn color_mesh(rect: Rect, (cols, rows): (u32, u32), color_at: &dyn Fn(f32, f32) -> Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    let row = cols + 1;
    mesh.reserve_vertices((row * (rows + 1)) as usize);
    mesh.reserve_triangles((cols * rows * 2) as usize);
    for j in 0..=rows {
        for i in 0..=cols {
            let (x, y) = (i as f32 / cols as f32, j as f32 / rows as f32);
            mesh.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), color_at(x, y));
        }
    }
    for j in 0..rows {
        for i in 0..cols {
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row);
            mesh.add_triangle(a + 1, a + row + 1, a + row);
        }
    }
    egui::Shape::mesh(mesh)
}

/// The Color Picker's vertical channel slider: `track(t)` gives the colour at 0..1 (bottom to top)
/// and arrows on both sides mark `value`. Returns the new value and the phase.
pub fn channel_slider(ui: &mut Ui, id: impl std::hash::Hash, value: f32, size: Vec2, track: &dyn Fn(f32) -> Color32) -> (Option<f32>, Live) {
    channel_slider_named(ui, id, "", value, size, track)
}

/// [`channel_slider`] that screen readers get as a slider named `name` (already translated) with
/// no value, as for [`color_slider_named`] (none when `name` is empty).
pub fn channel_slider_named(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: &str,
    value: f32,
    size: Vec2,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    channel_slider_valued(ui, id, name, None, value, size, track)
}

/// [`channel_slider_named`] that screen readers also get `shown` from, as for
/// [`color_slider_valued`].
pub fn channel_slider_valued(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: &str,
    shown: Option<SliderValue>,
    value: f32,
    size: Vec2,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, id_with(ui.scope_id(), id), Sense::click_and_drag());
    let bar = rect.shrink2(vec2(6.0, 0.0));
    gradient_strip(ui, bar, (bar.height() / 2.0).ceil().max(2.0) as usize, true, track);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let y = bar.bottom() - value.clamp(0.0, 1.0) * bar.height();
    let fill = if resp.dragged() || resp.hovered() { t.text_strong } else { t.icon };
    for (x, d) in [(rect.left(), 1.0), (rect.right(), -1.0)] {
        let tip = pos2(x + d * 5.0, y);
        ui.painter().add(egui::Shape::convex_polygon(vec![tip, pos2(x, y + 4.0 * d), pos2(x, y - 4.0 * d)], fill, Stroke::NONE));
    }
    slider_info(ui, &resp, name, shown);
    match pointer_phase(&resp) {
        Some((p, phase)) => (Some(((bar.bottom() - p.y) / bar.height()).clamp(0.0, 1.0)), phase),
        None => (None, Live::Idle),
    }
}

/// A hex colour field (`E67828`, no `#`) after a `#` label. Returns the text on commit when it
/// was typed into the field and differs from `hex`. Enter, Shift+Enter (which keeps the focus
/// outside dialogs) and focus loss commit it.
pub fn hex_field(ui: &mut Ui, id: impl std::hash::Hash, hex: &str) -> Option<String> {
    hex_field_named(ui, id, None, hex)
}

/// [`hex_field`] for a colour field named explicitly instead of by an adjacent label widget.
pub fn hex_field_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, hex: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let label = take_previous_label(ui);
    ui.label(egui::RichText::new("#").size(15.0).color(t.text));
    let id = id_with(ui.scope_id(), id);
    let editing = ui.memory(|m| m.has_focus(id));
    // Until the user types into it, the field shows `hex` as it is now, also while it has the
    // focus. Tab can give it the focus in the frame another field commits an edit, before the edit
    // changes `hex`; the text from before that edit is not committed when the focus moves on
    // (#1692).
    let edited_id = id.with("edited");
    let edited = editing && ui.data_mut(|d| d.get_temp::<bool>(edited_id)).unwrap_or(false);
    let mut buf: String = if edited { ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| hex.to_string()) } else { hex.to_string() };
    if after_shift_enter(ui, id) {
        select_all(ui, id, &buf);
    }
    let kept = shift_enter_keeps_focus(ui, id);
    let resp = egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            // As tall as the other fields (26 points with the margin), whatever the font's line
            // height: egui 0.37 gives a TextEdit the theme's interact height, so lower it here too.
            ui.spacing_mut().interact_size.y = 18.0;
            ui.set_height(18.0);
            ui.add(
                egui::TextEdit::singleline(&mut buf)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(62.0)
                    .char_limit(7)
                    .font(egui::FontId::proportional(12.5))
                    .text_color(t.text_strong),
            )
        })
        .inner;
    let resp = name_field(label_field(resp, label), name);
    let commit = (resp.lost_focus() || kept) && buf != hex;
    let focused = ui.memory(|m| m.has_focus(id));
    ui.data_mut(|d| {
        d.insert_temp(id, buf.clone());
        d.insert_temp(edited_id, (edited || resp.changed()) && focused && !kept);
    });
    commit.then_some(buf)
}

/// Position of hue `h` (degrees) and saturation `s` on a wheel of radius `r` around `c`.
fn wheel_pos(c: Pos2, r: f32, h: f32, s: f32) -> Pos2 {
    let a = h.to_radians();
    c + vec2(a.cos(), -a.sin()) * r * s
}

/// The hue and saturation wheel at brightness `v`: hue around, saturation outwards.
fn wheel_shape(c: Pos2, r: f32, v: f32) -> egui::Shape {
    const SEG: u32 = 72;
    const RINGS: u32 = 8;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, crate::panels::c32(&Color::from_hsb(0.0, 0.0, v)));
    for ring in 1..=RINGS {
        let s = ring as f32 / RINGS as f32;
        for k in 0..SEG {
            let h = k as f32 * 360.0 / SEG as f32;
            mesh.colored_vertex(wheel_pos(c, r, h, s), crate::panels::c32(&Color::from_hsb(h, s, v)));
        }
    }
    let at = |ring: u32, k: u32| if ring == 0 { 0 } else { 1 + (ring - 1) * SEG + k % SEG };
    for ring in 0..RINGS {
        for k in 0..SEG {
            if ring == 0 {
                mesh.add_triangle(0, at(1, k), at(1, k + 1));
            } else {
                mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
                mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
            }
        }
    }
    egui::Shape::mesh(mesh)
}

/// The width of a [`hue_ring`]'s band.
const RING_WIDTH: f32 = 14.0;
/// How far inside its rect a [`hue_ring`]'s band is drawn: a press up to this far outside the band
/// still picks a hue.
const RING_MARGIN: f32 = 4.0;

/// The display sRGB of HSB `[h°, s, v]` as an egui color, with no color conversion.
pub fn hsb32(hsb: [f32; 3]) -> Color32 {
    let [r, g, b] = vectorcraft_color::hsb_to_rgb(hsb).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// The square a [`hue_ring`] drawn in `rect` holds: the largest one inside the band's inner edge,
/// 2 points clear of it.
pub fn ring_square(rect: Rect) -> Rect {
    let inner = rect.width().min(rect.height()) / 2.0 - RING_MARGIN - RING_WIDTH - 2.0;
    Rect::from_center_size(rect.center(), Vec2::splat(inner.max(0.0) * std::f32::consts::SQRT_2))
}

/// The part of a [`hue_ring`] a press started on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RingPart {
    Band,
    Inner,
    Corner,
}

/// What a [`hue_ring`] did this frame.
pub struct RingResponse {
    /// The rect the ring is drawn in.
    pub rect: Rect,
    /// A new hue (degrees, 0..360) from a press or drag on the band, an arrow key or a screen
    /// reader, and its phase.
    pub hue: Option<(f32, Live)>,
    /// The pointer of a press or drag that started inside the band's inner edge, and its phase.
    pub inner: Option<(Pos2, Live)>,
}

/// A hue ring `size` across: a band of hues drawn [`RING_MARGIN`] inside its rect, the hue
/// increasing counterclockwise from 3 o'clock as on [`harmony_wheel`], with a marker at `hue` when
/// `marked`. A press on the band, or up to [`RING_MARGIN`] outside it, picks the hue under the
/// pointer (with Shift, in 45° steps) and gives the ring the keyboard focus. A press inside the
/// band's inner edge comes back as `inner`, and a press in the rect's corners does nothing. The part
/// a press starts on keeps the whole drag. While the ring has the focus, → and ↑ raise `hue` by
/// 1° and ← and ↓ lower it ([`STEPS`]), marked or not, and 360° wraps to 0°. Screen readers read it
/// as a slider named `name` from 0 to 360 (0 when not `marked`), which their increment, decrement
/// and set-value actions move.
pub fn hue_ring(ui: &mut Ui, id: impl std::hash::Hash, name: &str, size: f32, hue: f32, marked: bool) -> RingResponse {
    use egui::accesskit::{Action, ActionData};
    const SEG: u32 = 72;
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let resp = ui.interact(rect, id_with(ui.scope_id(), id), Sense::click_and_drag());
    let c = rect.center();
    let outer = size / 2.0 - RING_MARGIN;
    let inner = outer - RING_WIDTH;
    let mut mesh = egui::Mesh::default();
    for k in 0..=SEG {
        let h = k as f32 * 360.0 / SEG as f32;
        let col = hsb32([h, 1.0, 1.0]);
        mesh.colored_vertex(wheel_pos(c, inner, h, 1.0), col);
        mesh.colored_vertex(wheel_pos(c, outer, h, 1.0), col);
    }
    for k in 0..SEG {
        let a = 2 * k;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(c, outer, Stroke::new(1.0, t.border));
    painter.circle_stroke(c, inner, Stroke::new(1.0, t.border));
    if ui.memory(|m| m.has_focus(resp.id)) {
        painter.circle_stroke(c, outer + 2.0, Stroke::new(1.0, t.accent));
    }
    if marked {
        let m = wheel_pos(c, (inner + outer) / 2.0, hue, 1.0);
        painter.circle_stroke(m, RING_WIDTH / 2.0 - 1.0, Stroke::new(1.5, Color32::WHITE));
        painter.circle_stroke(m, RING_WIDTH / 2.0, Stroke::new(1.0, Color32::BLACK));
    }
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), if marked { f64::from(hue) } else { 0.0 }, name));
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(360.0);
        node.set_numeric_value_step(1.0);
        for action in [Action::SetValue, Action::Increment, Action::Decrement] {
            node.add_action(action);
        }
    });
    let part_at = |p: Pos2| match p.distance(c) {
        d if d < inner => RingPart::Inner,
        d if d <= outer + RING_MARGIN => RingPart::Band,
        _ => RingPart::Corner,
    };
    let part_id = resp.id.with("part");
    if resp.is_pointer_button_down_on()
        && ui.input(|i| i.pointer.primary_pressed())
        && let Some(p) = resp.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(part_id, part_at(p)));
    }
    let pointer = pointer_phase(&resp);
    // A click takes the part under it; a drag keeps the part its press started on.
    let part = match pointer {
        Some((p, _)) if resp.clicked() => Some(part_at(p)),
        Some((p, _)) => Some(ui.data(|d| d.get_temp::<RingPart>(part_id)).unwrap_or_else(|| part_at(p))),
        None => ui.data(|d| d.get_temp::<RingPart>(part_id)),
    };
    if resp.clicked() || resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<RingPart>(part_id));
    }
    if part == Some(RingPart::Band) {
        focus_on_press(&resp);
    }
    let (dx, dy) = arrow_steps(ui, resp.id, true, true);
    let (steps, set) = ui.input(|i| {
        let steps =
            i.num_accesskit_action_requests(resp.id, Action::Increment) as f32 - i.num_accesskit_action_requests(resp.id, Action::Decrement) as f32;
        let set = i.accesskit_action_requests(resp.id, Action::SetValue).find_map(|r| match r.data {
            Some(ActionData::NumericValue(v)) if v.is_finite() => Some(v.rem_euclid(360.0) as f32),
            _ => None,
        });
        (steps + dx + dy, set)
    });
    let mut out = RingResponse { rect, hue: None, inner: None };
    if let Some((p, phase)) = pointer {
        match part.unwrap_or_else(|| part_at(p)) {
            RingPart::Band => {
                let v = p - c;
                let h = (-v.y).atan2(v.x).to_degrees();
                let h = if ui.input(|i| i.modifiers.shift) { (h / 45.0).round() * 45.0 } else { h };
                out.hue = Some((h.rem_euclid(360.0), phase));
            }
            RingPart::Inner => out.inner = Some((p, phase)),
            RingPart::Corner => {}
        }
    } else {
        let new = set.unwrap_or(hue + steps);
        // Without a marker there is no color yet, so a set value applies even when it equals `hue`.
        if new != hue || (set.is_some() && !marked) {
            out.hue = Some((new.rem_euclid(360.0), Live::Released));
        }
    }
    out
}

/// What a [`harmony_wheel`] did this frame.
#[derive(Default)]
pub struct WheelResponse {
    /// The colour whose marker was pressed (it becomes the selected one).
    pub pressed: Option<usize>,
    /// A colour moved by dragging its marker (a new hue and saturation) or, the selected colour,
    /// the brightness slider (a new brightness): `[h°, s, v]`, `None` where the control kept it.
    pub moved: Option<(usize, [Option<f32>; 3])>,
}

/// The harmony wheel of Recolor Artwork's Edit tab and the Color Themes panel: a hue and saturation
/// wheel `size` across (hue around, saturation outwards) at the selected colour's brightness, a
/// marker per colour joined to the centre (the selected colour, `base`, larger) and a Brightness
/// slider for the selected colour below it.
pub fn harmony_wheel(ui: &mut Ui, id: &str, size: f32, colors: &[Color], base: Option<usize>) -> WheelResponse {
    let t = Tokens::get(ui.ctx());
    let mut out = WheelResponse::default();
    let base_hsb = base.and_then(|b| colors.get(b)).map(Color::to_hsb);
    let v = base_hsb.map_or(1.0, |c| c[2]);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size) / 2.0).max(0.0));
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
        let (c, r) = (rect.center(), size / 2.0 - 10.0);
        let painter = ui.painter_at(rect);
        painter.add(wheel_shape(c, r, v.max(0.15)));
        painter.circle_stroke(c, r, Stroke::new(1.0, t.border));
        let at = |col: &Color| {
            let [h, s, _] = col.to_hsb();
            wheel_pos(c, r, h, s)
        };
        for (i, col) in colors.iter().enumerate() {
            let (p, big) = (at(col), Some(i) == base);
            painter.line_segment([c, p], Stroke::new(1.0, Color32::from_white_alpha(160)));
            painter.circle(p, if big { 8.0 } else { 5.5 }, crate::panels::c32(col), Stroke::new(1.5, Color32::WHITE));
            painter.circle_stroke(p, if big { 9.0 } else { 6.5 }, Stroke::new(1.0, Color32::BLACK));
        }
        // Drag a marker: the nearest one within reach of the press.
        let drag = ui.scope_id().with((id, "drag"));
        if (resp.drag_started() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let near = colors.iter().map(at).enumerate().map(|(i, m)| (i, m.distance(p))).filter(|x| x.1 <= 14.0).min_by(|a, b| a.1.total_cmp(&b.1));
            out.pressed = near.map(|x| x.0);
            ui.data_mut(|m| m.insert_temp(drag, out.pressed));
        }
        if resp.dragged()
            && let (Some(i), Some(p)) = (ui.data(|m| m.get_temp::<Option<usize>>(drag)).flatten(), resp.interact_pointer_pos())
            && i < colors.len()
        {
            let dv = p - c;
            let h = (-dv.y).atan2(dv.x).to_degrees().rem_euclid(360.0);
            out.moved = Some((i, [Some(h), Some((dv.length() / r).clamp(0.0, 1.0)), None]));
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, ui.is_enabled(), tl!("Color wheel")));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - size - 76.0) / 2.0).max(0.0));
        ui.add_sized([70.0, 22.0], egui::Label::new(egui::RichText::new(tl!("Brightness")).color(t.text_dim)));
        // Screen readers get the brightness in percent, as the Color panel's HSB B field shows it.
        let shown = SliderValue::field((f64::from(v) * 100.0).round(), 0.0..=100.0, "%", 0);
        if let (Some(b), Some([h, s, _])) = (base, base_hsb)
            && let (Some(nv), _) = color_slider_valued(ui, (id, "brightness"), tl!("Brightness"), Some(shown), v, size, &|x| {
                crate::panels::c32(&Color::from_hsb(h, s, x))
            })
        {
            out.moved = Some((b, [None, None, Some(nv)]));
        }
    });
    out
}

/// `region` of `doc` rendered on white, its longest side `px` pixels, as a texture named `name`
/// (artboard and page thumbnails).
pub fn region_texture(
    ctx: &egui::Context,
    renderer: &mut vectorcraft_render::Renderer,
    name: &str,
    doc: &vectorcraft_doc::Document,
    region: vectorcraft_geom::Rect,
    px: f64,
) -> egui::TextureHandle {
    let scale = px / region.width().max(region.height()).max(1.0);
    let img = renderer.render_region(doc, region, scale, true);
    let color = crate::canvas::color_image(&img);
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}
