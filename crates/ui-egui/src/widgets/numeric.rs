use super::labels::{PreviousLabel, label_field, name_field, take_previous_label};
use super::panel_chrome::chevron_cell;
use super::text::{after_shift_enter, recessed_text, select_all, select_all_on_focus, shift_enter_keeps_focus, take_dialog_focus};
use super::*;

/// Preferences › Units › Numbers Without Units Are Points (#394; on until set): the app sets it
/// when the preference changes (`prefs_dialog::apply_runtime`), the length fields read it.
pub(crate) fn set_bare_numbers_are_points(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("bare_numbers_are_points"), on));
}

/// The unit a number typed with no unit into a field in `unit` is read in: `unit`, or for a field
/// in picas points with Numbers Without Units Are Points on (where `12` could be either). A typed
/// unit (`12 mm`, `2p6`, `1in`) always wins.
pub(crate) fn typed_unit(ctx: &egui::Context, unit: Unit) -> Unit {
    let bare_points = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("bare_numbers_are_points"))).unwrap_or(true);
    if bare_points && unit == Unit::Picas { Unit::Points } else { unit }
}

/// `text` typed in a numeric field, with a decimal comma read as a point: `0,1 mm` is `0.1 mm`
/// (#3837). Text with a point keeps its commas, so `1,000.5` is still refused rather than read
/// with two decimal separators, and so does a number that reads as thousands ([`thousands`]).
pub(super) fn decimal_point(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains(',') && !text.contains('.') && !thousands(text) { text.replace(',', ".").into() } else { text.into() }
}

/// Is `text` a number written with a thousands comma, as `1,000` or `12,500 mm`: one to three
/// digits (not a lone 0), a comma, exactly three digits, then at most a unit? Read as a decimal it
/// would be a thousand times too small, so the field refuses it instead (`0,125` and `1,25` are
/// decimals).
fn thousands(text: &str) -> bool {
    let number = text.trim().trim_start_matches('-');
    let Some((whole, rest)) = number.split_once(',') else { return false };
    let (part, unit) = rest.split_at(rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len()));
    (1..=3).contains(&whole.len())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && !whole.starts_with('0')
        && part.len() == 3
        && unit.trim().chars().all(char::is_alphabetic)
}

/// A recessed numeric field showing `value` (points) in `unit`. Returns the new value (points)
/// when the user commits (Enter / focus loss / Shift+Enter, which keeps the focus outside dialogs).
/// Supports unit suffixes and arithmetic; a number without a unit is in `unit` ([`typed_unit`]: in
/// points in a picas field with Numbers Without Units Are Points on).
pub fn num_field(ui: &mut Ui, id: impl std::hash::Hash, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    num_field_named(ui, id, None, value, unit, width)
}

/// [`num_field`] for a row whose visible label is not an adjacent label widget (a grid cell with
/// its label painted separately): screen readers name the field `name` (already translated).
pub fn num_field_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    let label = take_previous_label(ui);
    unit_field(ui, id, label, name, value, unit, width, false, ANY)
}

/// Any number: the bounds of a field that has none.
const ANY: RangeInclusive<f64> = f64::NEG_INFINITY..=f64::INFINITY;

/// `v` within `bounds` (bounds the wrong way round or not numbers don't panic, as `f64::clamp` would).
fn within(v: f64, bounds: &RangeInclusive<f64>) -> f64 {
    v.max(*bounds.start()).min(*bounds.end())
}

/// [`num_field`]; with `share_of_value` a percentage typed alone is that share of `value`
/// ([`share_of`]). ↑/↓, the wheel and a scrub stop at `bounds` (points), and the field shows where
/// they stopped.
#[allow(clippy::too_many_arguments)]
fn unit_field(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    label: Option<PreviousLabel>,
    name: Option<&str>,
    value: Option<f64>,
    unit: Unit,
    width: f32,
    share_of_value: bool,
    bounds: RangeInclusive<f64>,
) -> Option<f64> {
    let id = id_with(ui.scope_id(), id);
    let shown = value.map(|v| unit.format(v)).unwrap_or_default();
    take_dialog_focus(ui, id, &shown);
    let (mut buf, commit, resp, rect) = recessed_text(ui, id, &shown, width, 1, label, name);
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step in the field's unit; a drag on its label scrubs it.
    let typed = typed_unit(ui.ctx(), unit);
    let read = |b: &str| typed.parse(&decimal_point(b));
    let bounds = unit.from_pt(*bounds.start())..=unit.from_pt(*bounds.end());
    let stepped = step_value(ui, &resp, &mut buf, &bounds, (|b: &str| Some(unit.from_pt(read(b)?)), |v| unit.format(unit.to_pt(v))), None);
    let scrubbed = scrub::field(ui, id, rect, &buf, value.map(|v| unit.from_pt(v)), STEP_DECIMALS, &bounds);
    let commit = commit && buf != shown;
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    if commit {
        read(&buf).or_else(|| if share_of_value { share_of(&decimal_point(&buf), value?) } else { None })
    } else {
        stepped.or(scrubbed).map(|v| unit.to_pt(v))
    }
}

/// A percentage typed alone (`50%`, `150 %`, `100/3%`) as that share of `value`; `None` for any
/// other text. `Unit::parse` reads a bare percentage as nothing, having no value to take it of.
pub(super) fn share_of(text: &str, value: f64) -> Option<f64> {
    let percent = vectorcraft_doc::parse_number(text.trim().strip_suffix('%')?)?;
    Some(value * percent / 100.0)
}

/// The places a stepped value in a unit field keeps (no 12.300000001).
pub(super) const STEP_DECIMALS: i32 = 3;

/// `v` rounded to `decimals` places.
pub(crate) fn round_to(v: f64, decimals: i32) -> f64 {
    let scale = 10f64.powi(decimals.clamp(0, 6));
    (v * scale).round() / scale
}

/// One step of a numeric field with each modifier, most specific first (a plain pattern would also
/// match Shift): ten with Shift, a tenth with Ctrl/Cmd, else one. A Ctrl/Cmd or plain step goes to
/// the next multiple of its size ([`step_to_multiple`]); a Shift step adds or takes ten.
const STEPS: [(egui::Modifiers, f64); 3] = [(egui::Modifiers::SHIFT, 10.0), (egui::Modifiers::COMMAND, 0.1), (egui::Modifiers::NONE, 1.0)];

/// `v` moved `n` steps of `size` (down when `n` is negative): the first step goes to the next
/// multiple of `size` in its direction and each further step adds or takes one `size`. By 1, 5.1
/// goes up to 6 and down to 5, and 5 goes up to 6 and down to 4. A value within a millionth of a
/// step of a multiple counts as that multiple.
pub(super) fn step_to_multiple(v: f64, size: f64, n: f64) -> f64 {
    if n == 0.0 {
        return v;
    }
    let q = v / size;
    let from = if n > 0.0 { (q + 1e-6).floor() } else { (q - 1e-6).ceil() };
    (from + n) * size
}

/// ↑/↓ in focused numeric field `resp`, or the mouse wheel turned over it, step the number `buf`
/// shows (`read` parses it, `show` formats it) per press or wheel notch ([`STEPS`]): a plain step
/// goes to the next whole number in its direction (5.1 up to 6, down to 5), a Ctrl/Cmd step goes to
/// the next tenth, and a Shift step adds or takes ten. Steps stop at the field's `bounds` (#3370:
/// a weight stops at 0, a miter limit at 1). The new value is applied at once, as in
/// Illustrator's panels and dialogs, and the new text replaces `buf`, all selected so typing
/// replaces it. Returns the new number when a step was taken. While the pointer is over the focused
/// field the wheel is the field's (the panel under it doesn't scroll); an unfocused field leaves the
/// wheel to the panel and the canvas.
fn step_value(
    ui: &Ui,
    resp: &Response,
    buf: &mut String,
    bounds: &RangeInclusive<f64>,
    (read, show): (impl Fn(&str) -> Option<f64>, impl Fn(f64) -> String),
    step: Option<fn(f64, f64) -> f64>,
) -> Option<f64> {
    // Memory focus, as the field's highlight and its typing use: `Response::has_focus` also needs the
    // window to report keyboard focus.
    if !ui.memory(|m| m.has_focus(resp.id)) {
        return None;
    }
    use egui::{Event, Key, MouseWheelUnit};
    let wheel = resp.contains_pointer();
    // Wheel turns in notches: a line each, or `line` points of a trackpad or a fine wheel; what is
    // left of a notch carries over to the next turn.
    let (line, notch_id) = (ui.ctx().options(|o| o.input_options.line_scroll_speed), resp.id.with("wheel"));
    let mut notches: f32 = if wheel { ui.data(|d| d.get_temp(notch_id)).unwrap_or(0.0) } else { 0.0 };
    // Each press and each turn of whole notches in the order they came: its modifiers and size from
    // [`STEPS`], and how many steps up (down when negative).
    let mut steps: Vec<(egui::Modifiers, f64, f64)> = Vec::new();
    ui.input_mut(|i| {
        i.events.retain(|e| {
            let (Event::Key { modifiers, .. } | Event::MouseWheel { modifiers, .. }) = e else { return true };
            let Some(&(mods, size)) = STEPS.iter().find(|(m, _)| modifiers.matches_logically(*m)) else { return true };
            let n = match e {
                Event::Key { key: Key::ArrowUp, pressed: true, .. } => 1.0,
                Event::Key { key: Key::ArrowDown, pressed: true, .. } => -1.0,
                Event::MouseWheel { unit, delta, .. } if wheel => {
                    // Shift may turn the wheel sideways (the Mac does).
                    let d = if modifiers.shift { delta.x + delta.y } else { delta.y };
                    notches += match unit {
                        MouseWheelUnit::Point => d / line.max(1.0),
                        MouseWheelUnit::Line | MouseWheelUnit::Page => d,
                    };
                    let whole = notches.trunc();
                    notches -= whole;
                    f64::from(whole)
                }
                _ => return true,
            };
            if n != 0.0 {
                steps.push((mods, size, n));
            }
            false
        });
        if wheel {
            // The rest of a turn egui spreads over the next frames scrolls nothing either.
            i.smooth_scroll_delta = egui::Vec2::ZERO;
        }
    });
    if wheel {
        ui.data_mut(|d| d.insert_temp(notch_id, notches));
    }
    if steps.is_empty() {
        return None;
    }
    let mut v = read(buf)?;
    for (mods, size, n) in steps {
        v = match step {
            // The field's own steps: ten at once with Shift, one with or without Ctrl/Cmd.
            Some(step) => step(v, if mods == egui::Modifiers::SHIFT { 10.0 * n } else { n }),
            None if mods == egui::Modifiers::SHIFT => v + size * n,
            None => step_to_multiple(v, size, n),
        };
        // Each step stops at the bounds, so a step back in the same frame starts from there.
        v = within(v, bounds);
    }
    // Rounded as fields show it, within the field's bounds.
    let v = within(round_to(v, STEP_DECIMALS), bounds);
    *buf = show(v);
    ui.data_mut(|d| d.insert_temp(resp.id, buf.clone()));
    select_all(ui, resp.id, buf);
    Some(v)
}

/// A [`num_field`] for a width or height (Transform panel, Properties panel, Control bar): Ctrl/Cmd+Enter
/// also commits it and ends its editing, as Enter does, and [`enter_modifiers`] reports Command. A
/// percentage typed alone (`50%`) is that share of the size shown, as in Illustrator (#2401).
pub fn size_field(ui: &mut Ui, id: impl std::hash::Hash, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    size_field_named(ui, id, None, value, unit, width)
}

/// [`size_field`] for a row named explicitly instead of by an adjacent label widget.
pub fn size_field_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    use egui::{Event, Key};
    let label = take_previous_label(ui);
    // The text box does not end its editing on an Enter with Ctrl/Cmd: a plain Enter goes in right
    // after it, so the box first takes the text typed before it in this frame.
    let field = id_with(ui.scope_id(), &id);
    if ui.memory(|m| m.has_focus(field)) {
        ui.input_mut(|i| {
            let cmd_enter = |e: &Event| matches!(e, Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.command);
            if let Some(at) = i.events.iter().position(cmd_enter) {
                let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
                i.events.insert(at + 1, enter);
            }
        });
    }
    unit_field(ui, id, label, name, value, unit, width, true, ANY)
}

/// [`plain_field`] for a number kept within `range` (the values of dialogs, preferences and panels
/// such as the Stroke panel's Limit); `None` shows a blank field, as [`mixed_field`] does. What is
/// typed, stepped or scrubbed is clamped into it and rounded to `decimals` places (0: a count), and
/// ↑/↓, the wheel and a scrub stop at its ends in the field too (#3370).
pub fn range_field(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    value: impl Into<Option<f64>>,
    range: RangeInclusive<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    range_field_named(ui, id, None, value, range, suffix, decimals, width)
}

/// [`range_field`] for a row named explicitly instead of by an adjacent label widget.
#[allow(clippy::too_many_arguments)]
pub fn range_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    value: impl Into<Option<f64>>,
    range: RangeInclusive<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let places = decimals.min(6) as i32;
    let value = value.into().map(|v| within(v, &range));
    let label = take_previous_label(ui);
    bounded_field(ui, id, label, name, value, suffix, decimals, width, &range, None).map(|v| within(round_to(v, places), &range))
}

/// [`range_field`] whose ↑/↓ and wheel take `step`'s steps instead of whole numbers (see
/// [`SliderValue::step`]), for a number whose useful values are fractions, such as the Pencil's
/// Fidelity.
#[allow(clippy::too_many_arguments)]
pub fn range_field_stepped(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    value: impl Into<Option<f64>>,
    range: RangeInclusive<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
    step: fn(f64, f64) -> f64,
) -> Option<f64> {
    let places = decimals.min(6) as i32;
    let value = value.into().map(|v| within(v, &range));
    let label = take_previous_label(ui);
    bounded_field(ui, id, label, None, value, suffix, decimals, width, &range, Some(step)).map(|v| within(round_to(v, places), &range))
}

/// [`plain_field`] for a value the selected objects may not share: `None` shows a blank field.
pub fn mixed_field(ui: &mut Ui, id: impl std::hash::Hash, value: Option<f64>, suffix: &str, decimals: usize, width: f32) -> Option<f64> {
    mixed_field_named(ui, id, None, value, suffix, decimals, width)
}

/// [`mixed_field`] for a row named explicitly instead of by an adjacent label widget.
pub fn mixed_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    value: Option<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let label = take_previous_label(ui);
    bounded_field(ui, id, label, name, value, suffix, decimals, width, &ANY, None)
}

/// [`mixed_field`] whose ↑/↓, wheel and scrub stop at `bounds`; ↑/↓ and the wheel take `step`'s steps
/// when it is given (see [`SliderValue::step`]).
#[allow(clippy::too_many_arguments)]
fn bounded_field(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    label: Option<PreviousLabel>,
    name: Option<&str>,
    value: Option<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
    bounds: &RangeInclusive<f64>,
    step: Option<fn(f64, f64) -> f64>,
) -> Option<f64> {
    let id = id_with(ui.scope_id(), id);
    let show = |value: f64| field_text(value, suffix, decimals);
    // The suffix (and %, °) may follow any operand: `45*2°`, `50% / 2`.
    let read = |buf: &str| {
        let suffix = suffix.trim();
        let buf = decimal_point(buf);
        let bare = if suffix.is_empty() { buf.to_string() } else { buf.replace(suffix, "") };
        vectorcraft_doc::parse_number(&bare.replace(['%', '°'], ""))
    };
    let shown = value.map(show).unwrap_or_default();
    take_dialog_focus(ui, id, &shown);
    let (mut buf, commit, resp, rect) = recessed_text(ui, id, &shown, width, 1, label, name);
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step at the field's precision (a count ignores Ctrl/Cmd's tenth); a drag on
    // its label scrubs it.
    let decimals = decimals.min(6) as i32;
    let stepped = step_value(ui, &resp, &mut buf, bounds, (read, show), step).map(|v| round_to(v, decimals));
    let scrubbed = scrub::field(ui, id, rect, &buf, value, decimals, bounds);
    if commit && buf != shown { read(&buf) } else { stepped.or(scrubbed).filter(|&v| Some(v) != value) }
}

/// The text [`plain_field`] shows for `value`: at most `decimals` places, without trailing zeros,
/// then `suffix`.
pub(super) fn field_text(value: f64, suffix: &str, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, value);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    format!("{s}{suffix}")
}

/// A slider whose rail shows, filled up to the handle: the theme's widget fill matches the panel
/// and dialog background, which leaves only the handle visible (#1483).
pub fn slider(ui: &mut Ui, slider: egui::Slider<'_>) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        // The buttons' border shows against the panel in every brightness; the fields' border
        // all but vanished on Medium Dark's panel (#1662).
        v.widgets.inactive.bg_fill = t.button_border;
        v.selection.bg_fill = t.accent;
        ui.add(slider.trailing_fill(true))
    })
    .inner
}

/// Numeric field with an up/down spinner on the left and a preset dropdown on the right
/// (Stroke weight, font size…): an editable combo box whose field keeps typing, ↑/↓ stepping and
/// label scrubbing. `value` (points) shows in `unit`, blank when `None` (the selection's values
/// differ); `presets` (points) are listed in `unit` too, and empty = no dropdown. The spinner, ↑/↓,
/// the wheel and a scrub stop at `min` (#3370), and a typed value below it applies `min`. Returns
/// the committed, stepped or picked value.
#[allow(clippy::too_many_arguments)]
pub fn spin_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    value: Option<f64>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    spin_field_named(ui, id, None, value, unit, width, step, min, presets)
}

/// [`spin_field`] for a row named explicitly instead of by an adjacent label widget.
#[allow(clippy::too_many_arguments)]
pub fn spin_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    name: Option<&str>,
    value: Option<f64>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    spin_field_auto_named(ui, id, name, value, None, unit, width, step, min, presets).and_then(SpinPick::value)
}

/// What a [`spin_field_auto`] returns: a value (points), or its Auto entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpinPick {
    Value(f64),
    Auto,
}

impl SpinPick {
    /// The value, `None` for Auto.
    pub fn value(self) -> Option<f64> {
        match self {
            SpinPick::Value(v) => Some(v),
            SpinPick::Auto => None,
        }
    }
}

/// [`spin_field`] whose dropdown starts with an Auto entry (Leading): `auto` is `Some(on)`, with
/// `on` checking it (no preset is then checked), or `None` for no Auto entry.
#[allow(clippy::too_many_arguments)]
pub fn spin_field_auto(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    value: Option<f64>,
    auto: Option<bool>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<SpinPick> {
    spin_field_auto_named(ui, id, None, value, auto, unit, width, step, min, presets)
}

/// [`spin_field_auto`] for a row named explicitly instead of by an adjacent label widget.
#[allow(clippy::too_many_arguments)]
pub fn spin_field_auto_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    name: Option<&str>,
    value: Option<f64>,
    auto: Option<bool>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<SpinPick> {
    let label = take_previous_label(ui);
    spin_generic(ui, label, value, auto, width, step, min, presets, &|v| unit.format(v), &mut |ui, fw, label| {
        unit_field(ui, id, label, name, value, unit, fw, false, min..=f64::INFINITY)
    })
}

/// [`spin_field`] for unitless values (percent, degrees, 1/1000 em) with a display suffix; a
/// `None` value (the selection's values differ) shows blank.
#[allow(clippy::too_many_arguments)]
pub fn spin_plain(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    value: impl Into<Option<f64>>,
    suffix: &str,
    decimals: usize,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    spin_plain_named(ui, id, None, value, suffix, decimals, width, step, min, presets)
}

/// [`spin_plain`] for a row named explicitly instead of by an adjacent label widget.
#[allow(clippy::too_many_arguments)]
pub fn spin_plain_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    name: Option<&str>,
    value: impl Into<Option<f64>>,
    suffix: &str,
    decimals: usize,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    let label = take_previous_label(ui);
    let value = value.into();
    let fmt = |v: f64| format!("{v}{suffix}");
    spin_generic(ui, label, value, None, width, step, min, presets, &fmt, &mut |ui, fw, label| {
        bounded_field(ui, id, label, name, value, suffix, decimals, fw, &(min..=f64::INFINITY), None)
    })
    .and_then(SpinPick::value)
}

type SpinField<'a> = dyn FnMut(&mut Ui, f32, Option<PreviousLabel>) -> Option<f64> + 'a;

#[allow(clippy::too_many_arguments)]
fn spin_generic(
    ui: &mut Ui,
    label: Option<PreviousLabel>,
    value: Option<f64>,
    auto: Option<bool>,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
    fmt: &dyn Fn(f64) -> String,
    field: &mut SpinField<'_>,
) -> Option<SpinPick> {
    let t = Tokens::get(ui.ctx());
    let mut out = None;
    let h = 26.0;
    let enabled = ui.is_enabled();
    ui.scope(|ui| {
        // The spinner, the field and the chevron share their borders: one box in three parts.
        ui.spacing_mut().item_spacing.x = -1.0;
        let (sr, sresp) = ui.allocate_exact_size(vec2(16.0, h), Sense::click());
        ui.painter().rect_filled(sr, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, t.input);
        ui.painter().rect_stroke(sr, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
        let up = Rect::from_min_max(sr.min, pos2(sr.right(), sr.center().y));
        let down = Rect::from_min_max(pos2(sr.left(), sr.center().y), sr.max);
        let hover = sresp.hover_pos();
        for (r, is_up) in [(up, true), (down, false)] {
            let c = if !enabled {
                t.text_disabled
            } else if hover.is_some_and(|p| r.contains(p)) {
                t.text_strong
            } else {
                t.icon
            };
            let m = r.center();
            let d = if is_up { -1.5 } else { 1.5 };
            ui.painter().line_segment([m + vec2(-3.0, -d), m + vec2(0.0, d)], Stroke::new(1.2, c));
            ui.painter().line_segment([m + vec2(0.0, d), m + vec2(3.0, -d)], Stroke::new(1.2, c));
        }
        sresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tl!("Step up or down")));
        // A blank field (values that differ) has nothing to step from.
        if sresp.clicked()
            && let Some(p) = sresp.interact_pointer_pos()
            && let Some(v) = value
        {
            let modifiers = ui.input(|i| i.modifiers);
            out = Some(SpinPick::Value(spinner_step(v, step, min, up.contains(p), modifiers)));
        }
        let fw = if presets.is_empty() { width - 15.0 } else { width - 34.0 };
        if let Some(v) = field(ui, fw, label) {
            out = Some(SpinPick::Value(v.max(min)));
        }
        if !presets.is_empty() {
            let dresp = chevron_cell(ui, h);
            egui::Popup::menu(&dresp).show(|ui| {
                ui.set_min_width(width - 10.0);
                if let Some(on) = auto
                    && ui.selectable_label(on, tl!("Auto")).clicked()
                {
                    out = Some(SpinPick::Auto);
                }
                let current = value.filter(|_| auto != Some(true));
                for p in presets {
                    if ui.selectable_label(current.is_some_and(|v| (v - p).abs() < 1e-6), fmt(*p)).clicked() {
                        out = Some(SpinPick::Value(*p));
                    }
                }
            });
        }
    });
    out
}

/// Spinner buttons follow the same modifier sizes as ↑/↓ in a focused number field:
/// Shift ×10, Ctrl/Cmd ÷10, otherwise the caller's normal step.
pub(super) fn spinner_step(value: f64, step: f64, min: f64, up: bool, modifiers: egui::Modifiers) -> f64 {
    let multiplier = STEPS.iter().find(|(m, _)| modifiers.matches_logically(*m)).map_or(1.0, |(_, factor)| *factor);
    let delta = step * multiplier;
    (value + if up { delta } else { -delta }).max(min)
}

/// A number field that may be empty (Stroke dash/gap): `value` (points) shows in `unit` without
/// its suffix, to fit narrow fields. Returns `Some(new)` (points) on commit (Enter, Shift+Enter or
/// focus loss, as in [`num_field`]), where `new` is `None` when the field was cleared; typed units
/// and arithmetic work as in [`num_field`]. The value is a length: ↑/↓, the wheel and a scrub stop
/// at 0.
pub fn opt_field(ui: &mut Ui, id: impl std::hash::Hash, value: Option<f64>, unit: Unit, width: f32) -> Option<Option<f64>> {
    opt_field_named(ui, id, None, value, unit, width)
}

/// [`opt_field`] for a field named explicitly instead of by an adjacent label widget (the Stroke
/// panel's dash and gap cells have their labels in a row below them).
pub fn opt_field_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, value: Option<f64>, unit: Unit, width: f32) -> Option<Option<f64>> {
    let t = Tokens::get(ui.ctx());
    let label = take_previous_label(ui);
    let id = id_with(ui.scope_id(), id);
    let shown = value.map(|v| unit.number(v)).unwrap_or_default();
    let editing = ui.memory(|m| m.has_focus(id));
    // As in `hex_field`: until the user types into it, the field shows `shown` as it is now
    // (#1692). When `shown` changes while the field has the focus and nothing has been typed into
    // it, all of the new text is selected, as on keyboard focus.
    let edited_id = id.with("edited");
    let edited = editing && ui.data_mut(|d| d.get_temp::<bool>(edited_id)).unwrap_or(false);
    let before = ui.data_mut(|d| d.get_temp::<String>(id));
    let mut buf: String = if edited { before.clone().unwrap_or_else(|| shown.clone()) } else { shown.clone() };
    let reshown = after_shift_enter(ui, id);
    if reshown || editing && !edited && before.is_some_and(|b| b != buf) {
        select_all(ui, id, &buf);
    }
    let kept = shift_enter_keeps_focus(ui, id);
    let enabled = ui.is_enabled();
    let framed = egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, if editing { t.accent } else { t.input_border }))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(4, 4))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut buf)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(width - 10.0)
                    .font(egui::FontId::proportional(12.5))
                    .text_color(if enabled { t.text_strong } else { t.text_disabled }),
            )
        });
    let resp = framed.inner;
    let resp = name_field(label_field(resp, label), name);
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step a value (an empty field stays empty); a drag on its label scrubs it.
    let lengths = 0.0..=f64::INFINITY;
    let stepped = step_value(
        ui,
        &resp,
        &mut buf,
        &lengths,
        (|b: &str| Some(unit.from_pt(unit.parse(&decimal_point(b))?)), |v| unit.number(unit.to_pt(v))),
        None,
    );
    let scrubbed = scrub::field(ui, id, framed.response.rect, &buf, value.map(|v| unit.from_pt(v)), STEP_DECIMALS, &lengths);
    let focused = ui.memory(|m| m.has_focus(id));
    ui.data_mut(|d| {
        d.insert_temp(id, buf.clone());
        d.insert_temp(edited_id, (edited || resp.changed()) && focused && !kept);
    });
    if (resp.lost_focus() || kept) && buf != shown {
        let s = buf.trim();
        if s.is_empty() { Some(None) } else { typed_unit(ui.ctx(), unit).parse(&decimal_point(s)).map(Some) }
    } else {
        stepped.or(scrubbed).map(|v| Some(unit.to_pt(v)))
    }
}
