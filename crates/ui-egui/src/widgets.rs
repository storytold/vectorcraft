//! Shared widgets. Colours come from theme tokens.

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::Unit;

use crate::theme::{self, Tokens};
use crate::{icons, scrub};

/// Square icon button; `selected` draws the pressed well.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, selected: bool, size: f32) -> Response {
    let (_, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_icon_button(ui, resp, icon, tip, selected)
}

/// [`icon_button`] drawn for a response the caller made (its own id or sense).
pub fn paint_icon_button(ui: &mut Ui, resp: Response, icon: &str, tip: &str, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let rect = resp.rect;
    let size = rect.width();
    let bg = if selected {
        t.tool_active
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    let pad = (size * 0.2).round();
    icons::paint(ui, icon, rect.shrink(pad), if selected { t.text_strong } else { t.icon });
    resp.widget_info(|| icon_button_info(ui.is_enabled(), selected, tl!(tip)));
    if !tip.is_empty() { resp.on_hover_text(tl!(tip)) } else { resp }
}

/// Small flat text button (Quick Actions style).
pub fn flat_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.button);
    flat_fill(ui, rect, &resp);
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, tl!(text), egui::FontId::proportional(12.5), t.text_strong);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(text)));
    resp
}

/// Width of a [`split_button`]'s arrow.
pub const SPLIT_ARROW: f32 = 18.0;

/// A [`flat_button`] `width` wide whose right end is a menu arrow (Image Trace ▾) → the
/// responses of the button and of its arrow.
pub fn split_button(ui: &mut Ui, text: &str, width: f32) -> (Response, Response) {
    split_button_named(ui, text, "", width)
}

/// [`split_button`] whose arrow has the tool tip `arrow_tip`, which screen readers also call it
/// (none when empty).
pub fn split_button_named(ui: &mut Ui, text: &str, arrow_tip: &str, width: f32) -> (Response, Response) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    let split = rect.right() - SPLIT_ARROW;
    let main_rect = Rect::from_min_max(rect.min, pos2(split, rect.bottom()));
    let arrow_rect = Rect::from_min_max(pos2(split, rect.top()), rect.max);
    let main = ui.interact(main_rect, ui.id().with((text, "split")), Sense::click());
    let arrow = ui.interact(arrow_rect, ui.id().with((text, "split-arrow")), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.button);
    flat_fill(ui, main_rect, &main);
    flat_fill(ui, arrow_rect, &arrow);
    ui.painter().vline(split, rect.y_range().shrink(5.0), Stroke::new(1.0, t.panel));
    ui.painter().with_clip_rect(main_rect).text(
        main_rect.center(),
        egui::Align2::CENTER_CENTER,
        tl!(text),
        egui::FontId::proportional(12.5),
        t.text_strong,
    );
    icons::paint(ui, "chevron-down", Rect::from_center_size(arrow_rect.center(), vec2(12.0, 12.0)), t.text_dim);
    main.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(text)));
    if arrow_tip.is_empty() {
        return (main, arrow);
    }
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(arrow_tip)));
    (main, arrow.on_hover_text(tl!(arrow_tip)))
}

/// The pressed or hovered fill of a flat button part.
fn flat_fill(ui: &Ui, rect: Rect, resp: &Response) {
    let t = Tokens::get(ui.ctx());
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
}

/// The accent call-to-action button (dialog OK/Create).
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    primary_button_enabled(ui, text, true)
}

/// [`primary_button`], greyed out and not clickable unless `enabled`.
pub fn primary_button_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    primary_button_sized(ui, text, enabled, 72.0)
}

/// Primary dialog button with a minimum width, for aligned action columns.
pub(crate) fn primary_button_width(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    primary_button_sized(ui, text, true, min_width)
}

/// Draws a primary button at least `min_width` wide, growing to fit the translated label.
/// Returns its response with the enabled state and screen-reader name preserved.
fn primary_button_sized(ui: &mut Ui, text: &str, enabled: bool, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let fg = if enabled { Color32::WHITE } else { t.text_disabled };
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), theme::semibold(12.5), fg);
    let w = galley.size().x + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(min_width), 28.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = match (enabled, resp.hovered()) {
        (false, _) => t.input,
        (true, true) => t.accent,
        (true, false) => t.accent_strong,
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    if !enabled {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), egui::Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled && ui.is_enabled(), tl!(text)));
    resp
}

/// The quiet secondary button (dialog Cancel).
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    secondary_button_width(ui, text, 72.0)
}

/// Secondary dialog button with a minimum width, for aligned action columns.
pub(crate) fn secondary_button_width(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), theme::semibold(12.5), t.text);
    let w = galley.size().x + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(min_width), 28.0), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(6), if resp.hovered() { t.hover } else { t.button });
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(text)));
    resp
}

/// Bold section header (Properties panel).
pub fn section_header(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(tl!(text)).size(12.5).color(t.text_strong));
    ui.add_space(2.0);
}

/// A collapsible section's header across the panel: a chevron (down while `open`) and `text`;
/// a click anywhere on it toggles the section. Whether it was clicked.
pub fn section_toggle(ui: &mut Ui, text: &str, open: bool) -> bool {
    section_toggle_name(ui, tl!(text), open).clicked()
}

/// [`section_toggle`] for a name that is user data (a library group), shown as it is → its
/// response (clicked: toggle; it takes a context menu).
pub fn section_toggle_name(ui: &mut Ui, text: &str, open: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.hover);
    }
    let chevron = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(12.0, 12.0));
    icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, chevron, if resp.hovered() { t.text_strong } else { t.icon });
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, text, theme::semibold(12.5), t.text);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Full-width 1 px divider with vertical margins.
pub fn divider(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.divider);
    ui.add_space(6.0);
}

/// A label in the panels' text colour; before a numeric field it scrubs the field
/// ([`scrub`]).
pub fn dim_label(ui: &mut Ui, text: &str) -> Response {
    let resp = dim_name(ui, tl!(text));
    scrub::note_label(ui, resp.rect);
    resp
}

/// A field's label in other styles (`text` as given): before a numeric field it scrubs the field
/// ([`scrub`]).
pub fn field_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    let resp = ui.label(text);
    scrub::note_label(ui, resp.rect);
    resp
}

/// [`dim_label`] for a name that is user or file data (a swatch, style, artboard or font name):
/// shown as it is, never translated.
pub fn dim_name(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text).size(12.5))
}

/// Preferences › Units › Numbers Without Units Are Points (#394; on until set): the app sets it
/// when the preference changes (`prefs_dialog::apply_runtime`), the length fields read it.
pub(crate) fn set_bare_numbers_are_points(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("bare_numbers_are_points"), on));
}

/// The unit a number typed with no unit into a field in `unit` is read in: `unit`, or for a field
/// in picas points with Numbers Without Units Are Points on (where `12` could be either). A typed
/// unit (`12 mm`, `2p6`, `1in`) always wins.
pub(crate) fn typed_unit(ctx: &egui::Context, unit: Unit) -> Unit {
    let bare_points = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("bare_numbers_are_points"))).unwrap_or(true);
    if bare_points && unit == Unit::Picas { Unit::Points } else { unit }
}

/// A recessed numeric field showing `value` (points) in `unit`. Returns the new value (points)
/// when the user commits (Enter / focus loss / Shift+Enter, which keeps the focus outside dialogs).
/// Supports unit suffixes and arithmetic; a number without a unit is in `unit` ([`typed_unit`]: in
/// points in a picas field with Numbers Without Units Are Points on).
pub fn num_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    unit_field(ui, id, value, unit, width, false)
}

/// [`num_field`]; with `share_of_value` a percentage typed alone is that share of `value`
/// ([`share_of`]).
fn unit_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: Option<f64>,
    unit: Unit,
    width: f32,
    share_of_value: bool,
) -> Option<f64> {
    let id = ui.id().with(id);
    let shown = value.map(|v| unit.format(v)).unwrap_or_default();
    take_dialog_focus(ui, id, &shown);
    let (mut buf, commit, resp, rect) = recessed_text(ui, id, &shown, width, 1);
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step in the field's unit; a drag on its label scrubs it.
    let read = typed_unit(ui.ctx(), unit);
    let stepped = step_value(ui, &resp, &mut buf, |b| Some(unit.from_pt(read.parse(b)?)), |v| unit.format(unit.to_pt(v)));
    let scrubbed = scrub::field(ui, id, rect, &buf, value.map(|v| unit.from_pt(v)), STEP_DECIMALS);
    let commit = commit && buf != shown;
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    if commit {
        read.parse(&buf).or_else(|| if share_of_value { share_of(&buf, value?) } else { None })
    } else {
        stepped.or(scrubbed).map(|v| unit.to_pt(v))
    }
}

/// A percentage typed alone (`50%`, `150 %`, `100/3%`) as that share of `value`; `None` for any
/// other text. `Unit::parse` reads a bare percentage as nothing, having no value to take it of.
fn share_of(text: &str, value: f64) -> Option<f64> {
    let percent = vectorcraft_doc::parse_number(text.trim().strip_suffix('%')?)?;
    Some(value * percent / 100.0)
}

/// The places a stepped value in a unit field keeps (no 12.300000001).
const STEP_DECIMALS: i32 = 3;

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
fn step_to_multiple(v: f64, size: f64, n: f64) -> f64 {
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
/// the next tenth, and a Shift step adds or takes ten. The new value is applied at once, as in
/// Illustrator's panels and dialogs, and the new text replaces `buf`, all selected so typing
/// replaces it. Returns the new number when a step was taken. While the pointer is over the focused
/// field the wheel is the field's (the panel under it doesn't scroll); an unfocused field leaves the
/// wheel to the panel and the canvas.
fn step_value(ui: &Ui, resp: &Response, buf: &mut String, read: impl Fn(&str) -> Option<f64>, show: impl Fn(f64) -> String) -> Option<f64> {
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
        v = if mods == egui::Modifiers::SHIFT { v + size * n } else { step_to_multiple(v, size, n) };
    }
    // Rounded as fields show it.
    let v = round_to(v, STEP_DECIMALS);
    *buf = show(v);
    ui.data_mut(|d| d.insert_temp(resp.id, buf.clone()));
    select_all(ui, resp.id, buf);
    Some(v)
}

/// The flag `dialogs::show` raises while it draws the body of a dialog that has just opened; the
/// first enabled field drawn takes it ([`take_dialog_focus`]).
pub(crate) fn dialog_focus_flag() -> egui::Id {
    egui::Id::new("dialog-focus-first-field")
}

/// If field `id` (showing `text`) is the first enabled field of a dialog that has just opened, give
/// it the keyboard focus with all of `text` selected: typing replaces the value and Enter applies it.
pub(crate) fn take_dialog_focus(ui: &Ui, id: egui::Id, text: &str) {
    // A new window's first frame only measures its contents: nothing can take focus yet. Nor can a
    // greyed-out field (egui drops its focus): the flag stays for the first field that is enabled.
    if ui.is_sizing_pass() || !ui.is_enabled() || ui.data_mut(|d| d.remove_temp::<bool>(dialog_focus_flag())).is_none() {
        return;
    }
    focus_field(ui, id, text);
}

/// Give text field `id` the keyboard focus with all of `text` selected.
pub(crate) fn focus_field(ui: &Ui, id: egui::Id, text: &str) {
    select_all(ui, id, text);
    ui.memory_mut(|m| m.request_focus(id));
}

/// The modifiers held with an Enter press among this frame's events, `None` without one, as when a
/// field's editing ends with a click elsewhere.
pub(crate) fn enter_modifiers(ui: &Ui) -> Option<egui::Modifiers> {
    ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } => Some(*modifiers),
            _ => None,
        })
    })
}

/// A [`num_field`] for a width or height (Transform panel, Properties panel, Control bar): Ctrl/Cmd+Enter
/// also commits it and ends its editing, as Enter does, and [`enter_modifiers`] reports Command. A
/// percentage typed alone (`50%`) is that share of the size shown, as in Illustrator (#2401).
pub fn size_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<f64>, unit: Unit, width: f32) -> Option<f64> {
    use egui::{Event, Key};
    // The text box does not end its editing on an Enter with Ctrl/Cmd: a plain Enter goes in right
    // after it, so the box first takes the text typed before it in this frame.
    let field = ui.id().with(&id);
    if ui.memory(|m| m.has_focus(field)) {
        ui.input_mut(|i| {
            let cmd_enter = |e: &Event| matches!(e, Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.command);
            if let Some(at) = i.events.iter().position(cmd_enter) {
                let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
                i.events.insert(at + 1, enter);
            }
        });
    }
    unit_field(ui, id, value, unit, width, true)
}

/// Select all of `text` in text field `id`.
fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
    st.cursor.set_char_range(Some(all));
    st.store(ui.ctx(), id);
}

/// The flag that tells the fields a dialog is open in this frame ([`set_dialog_open`]).
fn dialog_open_flag() -> egui::Id {
    egui::Id::new("dialog-open")
}

/// The app sets this at the start of each frame: while a dialog is open, Shift+Enter in a field
/// applies its value and confirms the dialog as Enter does ([`shift_enter_keeps_focus`]).
pub(crate) fn set_dialog_open(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|d| d.insert_temp(dialog_open_flag(), open));
}

/// Shift+Enter (Shift+Return) in single-line field `id` while it has the keyboard focus and no
/// dialog is open: the press is taken from this frame's events before the field's text box reads
/// them, so the field keeps the focus. Returns true when it was pressed; the field then applies its
/// text as Enter does, and in the next frame shows its value with all of it selected
/// ([`after_shift_enter`]).
fn shift_enter_keeps_focus(ui: &Ui, id: egui::Id) -> bool {
    if !ui.memory(|m| m.has_focus(id)) || ui.data(|d| d.get_temp::<bool>(dialog_open_flag())).unwrap_or(false) {
        return false;
    }
    let pressed = ui.input_mut(|i| {
        let before = i.events.len();
        i.events.retain(|e| {
            !matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if modifiers.matches_exact(egui::Modifiers::SHIFT))
        });
        i.events.len() < before
    });
    if pressed {
        ui.data_mut(|d| d.insert_temp(id.with("shift-enter"), true));
    }
    pressed
}

/// Whether field `id`, still focused, took Shift+Enter in the last frame
/// ([`shift_enter_keeps_focus`]). Clears that record.
fn after_shift_enter(ui: &Ui, id: egui::Id) -> bool {
    ui.data_mut(|d| d.remove_temp::<bool>(id.with("shift-enter"))).unwrap_or(false) && ui.memory(|m| m.has_focus(id))
}

/// Handles Shift+Enter in a single-line panel field that draws its own text box `id` (Pattern
/// Options' Name, the inline renames in the Layers, Artboards and Asset Export panels). Call it
/// before the box is drawn, with `buf` the text the box will show and `shown` the value as it is
/// now. When the field took Shift+Enter in the last frame, `buf` becomes `shown`, all of it
/// selected. Returns whether Shift+Enter was pressed in this frame ([`shift_enter_keeps_focus`]):
/// the field then applies `buf` as Enter does and keeps the focus.
pub(crate) fn own_field_shift_enter(ui: &Ui, id: egui::Id, buf: &mut String, shown: &str) -> bool {
    if after_shift_enter(ui, id) {
        *buf = shown.to_string();
        select_all(ui, id, buf);
    }
    shift_enter_keeps_focus(ui, id)
}

/// Numeric fields select the whole text on keyboard focus, a completed focus click or a
/// double-click. A pointer press keeps its caret so dragging selects from the pressed position,
/// and a short drag the toolkit still counts as a click keeps it too (#1326): only a tap
/// releases into select-all.
fn select_all_on_focus(ui: &Ui, resp: &Response, text: &str) {
    let pending = resp.id.with("select-all-on-focus-click");
    let press_pos = resp.id.with("select-all-on-focus-press");
    let (pressed, released, at_press) = ui.input(|i| {
        (i.pointer.button_pressed(egui::PointerButton::Primary), i.pointer.button_released(egui::PointerButton::Primary), i.pointer.interact_pos())
    });
    if resp.gained_focus() {
        ui.data_mut(|d| {
            if pressed {
                d.insert_temp(pending, true);
                if let Some(p) = at_press {
                    d.insert_temp(press_pos, p);
                }
            } else {
                d.remove_temp::<bool>(pending);
                d.remove_temp::<egui::Pos2>(press_pos);
            }
        });
    }
    let clicked_focus = released && ui.data_mut(|d| d.remove_temp::<bool>(pending)).unwrap_or(false) && resp.clicked_by(egui::PointerButton::Primary);
    // A press released a few points away is a short drag, not a tap. egui still reports a
    // click, and a release inside one character leaves no text selection, so select-all on
    // release would replace the drag the user just made (#1326).
    let dragged = clicked_focus
        && ui
            .data_mut(|d| d.remove_temp::<egui::Pos2>(press_pos))
            .is_some_and(|p0| ui.input(|i| i.pointer.interact_pos()).is_some_and(|p1| (p1 - p0).length() > 3.0));
    if resp.lost_focus() {
        ui.data_mut(|d| {
            d.remove_temp::<bool>(pending);
            d.remove_temp::<egui::Pos2>(press_pos);
        });
    }
    if !(resp.gained_focus() && !pressed || resp.double_clicked() || clicked_focus && !dragged) {
        return;
    }
    if let Some(st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        // Preserve a text selection if the focus click already produced one.
        if clicked_focus && !resp.double_clicked() && st.cursor.char_range().is_some_and(|r| !r.as_sorted_char_range().is_empty()) {
            return;
        }
        select_all(ui, resp.id, text);
        ui.ctx().request_repaint();
    }
}

/// The recessed text box of the panel fields showing `shown`, `rows` lines tall (1: single line),
/// its text kept under `id` while it has focus. Returns the text (as edited), whether the field
/// applies it in this frame (it lost the focus, or a single-line field took Shift+Enter and kept
/// the focus: [`shift_enter_keeps_focus`]), the text edit's response and the box.
fn recessed_text(ui: &mut Ui, id: egui::Id, shown: &str, width: f32, rows: usize) -> (String, bool, Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let editing = ui.memory(|m| m.has_focus(id));
    // A press outside the field ends its editing in this frame, so the text typed is committed
    // before the press acts: the canvas, drawn after the panels, may select another object, and a
    // frame later the text would be committed to that one.
    if editing
        && let Some(at) = ui.input(|i| i.pointer.primary_pressed().then(|| i.pointer.interact_pos()).flatten())
        && ui.ctx().read_response(id).is_some_and(|r| !r.rect.contains(at))
    {
        ui.memory_mut(|m| m.surrender_focus(id));
    }
    // When the value changes under a focused field (the slider beside it, a preview, another
    // field's edit), its text follows: the text typed for the old value would otherwise go back in
    // on the next Enter, Tab or arrow key (#1457).
    let shown_id = id.with("shown");
    let followed = ui.data_mut(|d| d.get_temp::<String>(shown_id)).is_none_or(|before| before == shown);
    ui.data_mut(|d| d.insert_temp(shown_id, shown.to_string()));
    // A press elsewhere can end the focus outside this field's own drawing (a field drawn before it
    // takes it, the canvas ends text input), so `editing` is already false on the frame it reports
    // `lost_focus`: the text typed is still taken then, to be committed (#1637). Escape drops it.
    let focus_id = id.with("had-focus");
    let had_focus = ui.data_mut(|d| d.get_temp::<bool>(focus_id)).unwrap_or(false) && !ui.input(|i| i.key_pressed(egui::Key::Escape));
    // After Shift+Enter applied the text, the field shows the value as it is now, all selected.
    let reshown = after_shift_enter(ui, id);
    let typed = if (editing || had_focus) && followed && !reshown { ui.data_mut(|d| d.get_temp::<String>(id)) } else { None };
    let mut buf: String = typed.unwrap_or_else(|| shown.to_string());
    if reshown {
        select_all(ui, id, &buf);
    }
    let kept = rows == 1 && shift_enter_keeps_focus(ui, id);
    // The box is exactly as tall as the other controls of a row (26 points a line), its text
    // centred in it: a frame around the text came out a point taller and stood out next to
    // spinners and dropdowns (#900).
    let height = 26.0 + 16.0 * (rows.max(1) - 1) as f32;
    let layout = if rows > 1 { egui::Layout::top_down(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Center) };
    let (rect, resp) = ui
        .allocate_ui_with_layout(vec2(width, height), layout, |ui| {
            ui.set_min_size(vec2(width, height));
            let rect = ui.max_rect();
            ui.painter().rect(rect, 5.0, t.input, Stroke::new(1.0, if editing { t.accent } else { t.input_border }), StrokeKind::Inside);
            let margin = egui::Margin { left: 7, right: 7, top: if rows > 1 { 4 } else { 0 }, bottom: 0 };
            let resp = egui::Frame::NONE
                .inner_margin(margin)
                .show(ui, |ui| {
                    crate::theme::strong_selected_text(ui);
                    let edit = if rows > 1 { egui::TextEdit::multiline(&mut buf).desired_rows(rows) } else { egui::TextEdit::singleline(&mut buf) };
                    ui.add(
                        edit.id(id)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .desired_width(width - 14.0)
                            .font(egui::FontId::proportional(12.5))
                            .text_color(t.text_strong),
                    )
                })
                .inner;
            (rect, resp)
        })
        .inner;
    let focused = ui.memory(|m| m.has_focus(id));
    ui.data_mut(|d| {
        d.insert_temp(id, buf.clone());
        d.insert_temp(focus_id, focused);
    });
    let commit = resp.lost_focus() || kept;
    (buf, commit, resp, rect)
}

/// A recessed text field showing `value` (blank when `None`: the selection's values differ),
/// `rows` lines tall (1: single line, which Enter and Shift+Enter commit). Returns the new text,
/// trimmed, when a change is committed (Enter, Shift+Enter or focus loss).
pub fn text_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<&str>, width: f32, rows: usize) -> Option<String> {
    let id = ui.id().with(id);
    let shown = value.unwrap_or_default();
    let (buf, commit, _, _) = recessed_text(ui, id, shown, width, rows);
    (commit && buf.trim() != shown).then(|| buf.trim().to_string())
}

/// A one-line [`text_field`] that keeps the text exactly as typed, spaces included (a tab
/// leader such as ". "). Returns it when a change is committed (Enter, Shift+Enter or focus loss).
pub fn exact_text_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &str, width: f32) -> Option<String> {
    let id = ui.id().with(id);
    let (buf, commit, _, _) = recessed_text(ui, id, value, width, 1);
    (commit && buf != value).then_some(buf)
}

/// A form row: `label` in a column `label_width` wide, then what `add` draws.
pub fn label_row(ui: &mut Ui, label: &str, label_width: f32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(label_width, 24.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, tl!(label), egui::FontId::proportional(12.5), t.text);
        scrub::note_label(ui, r);
        add(ui);
    });
}

/// A plain number field (percent, degrees, counts) with optional suffix.
pub fn plain_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: f64, suffix: &str, decimals: usize, width: f32) -> Option<f64> {
    mixed_field(ui, id, Some(value), suffix, decimals, width)
}

/// [`plain_field`] for a number kept within `range` (the values of dialogs and preferences): what is
/// typed, stepped or scrubbed is clamped into it and rounded to `decimals` places (0: a count).
pub fn range_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let (lo, hi) = (*range.start(), *range.end());
    let places = decimals.min(6) as i32;
    plain_field(ui, id, value.clamp(lo, hi), suffix, decimals, width).map(|v| round_to(v, places).clamp(lo, hi))
}

/// [`plain_field`] for a value the selected objects may not share: `None` shows a blank field.
pub fn mixed_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: Option<f64>,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    let id = ui.id().with(id);
    let show = |value: f64| field_text(value, suffix, decimals);
    // The suffix (and %, °) may follow any operand: `45*2°`, `50% / 2`.
    let read = |buf: &str| {
        let suffix = suffix.trim();
        let bare = if suffix.is_empty() { buf.to_string() } else { buf.replace(suffix, "") };
        vectorcraft_doc::parse_number(&bare.replace(['%', '°'], ""))
    };
    let shown = value.map(show).unwrap_or_default();
    take_dialog_focus(ui, id, &shown);
    let (mut buf, commit, resp, rect) = recessed_text(ui, id, &shown, width, 1);
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step at the field's precision (a count ignores Ctrl/Cmd's tenth); a drag on
    // its label scrubs it.
    let decimals = decimals.min(6) as i32;
    let stepped = step_value(ui, &resp, &mut buf, read, show).map(|v| round_to(v, decimals));
    let scrubbed = scrub::field(ui, id, rect, &buf, value, decimals);
    if commit && buf != shown { read(&buf) } else { stepped.or(scrubbed).filter(|&v| Some(v) != value) }
}

/// The text [`plain_field`] shows for `value`: at most `decimals` places, without trailing zeros,
/// then `suffix`.
fn field_text(value: f64, suffix: &str, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, value);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    format!("{s}{suffix}")
}

/// Draw a paint preview (swatch chip) into `rect`.
pub fn paint_chip(ui: &Ui, rect: Rect, paint: &Paint) {
    let p = ui.painter();
    match paint {
        Paint::None => {
            p.rect_filled(rect, 0.0, Color32::WHITE);
            p.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.6, Color32::from_rgb(0xe0, 0x20, 0x20)));
        }
        Paint::Solid { color, .. } => {
            let [r, g, b, _] = color.to_rgba8(1.0);
            p.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
        }
        Paint::Gradient(gp) => gradient_chip(ui, rect, gp),
        Paint::Pattern { .. } => {
            p.rect_filled(rect, 0.0, Color32::from_gray(200));
            for i in 0..4 {
                let x = rect.left() + rect.width() * i as f32 / 4.0;
                p.line_segment([pos2(x, rect.bottom()), pos2(x + rect.width() / 4.0, rect.top())], Stroke::new(1.0, Color32::from_gray(90)));
            }
        }
    }
}

/// Draw a gradient preview into `rect`, laid out as the gradient would be on an object of that
/// shape: linear along its angle, radial from the centre, freeform from its colour points.
pub fn gradient_chip(ui: &Ui, rect: Rect, gradient: &vectorcraft_color::GradientPaint) {
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return;
    }
    // Freeform sampling is costly per spot, so it gets a coarse grid (the mesh interpolates).
    let freeform = gradient.gradient.kind == vectorcraft_color::GradientKind::Freeform;
    let max = if freeform { 16 } else { 64 };
    let cols = ((rect.width() / 3.0).ceil() as usize).clamp(1, max);
    let rows = ((rect.height() / 3.0).ceil() as usize).clamp(1, max);
    let colors = gradient_chip_colors(gradient, rect.width(), rect.height(), cols, rows);
    let mut mesh = egui::Mesh::default();
    for (k, color) in colors.into_iter().enumerate() {
        let (i, j) = (k % (cols + 1), k / (cols + 1));
        let at = pos2(rect.left() + rect.width() * i as f32 / cols as f32, rect.top() + rect.height() * j as f32 / rows as f32);
        mesh.colored_vertex(at, color);
    }
    for j in 0..rows as u32 {
        for i in 0..cols as u32 {
            let v = j * (cols as u32 + 1) + i;
            mesh.add_triangle(v, v + 1, v + cols as u32 + 1);
            mesh.add_triangle(v + 1, v + cols as u32 + 2, v + cols as u32 + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// The colours at the `(cols + 1) x (rows + 1)` grid points (row by row) of a `width` x `height`
/// chip showing `gp` as it would look on an object of that size.
fn gradient_chip_colors(gp: &vectorcraft_color::GradientPaint, width: f32, height: f32, cols: usize, rows: usize) -> Vec<Color32> {
    use vectorcraft_color::{GradientGeom, GradientKind};
    let b = vectorcraft_geom::kurbo::Rect::new(0.0, 0.0, width as f64, height as f64);
    let rgb = |c: vectorcraft_color::Color| {
        let [r, g, b, _] = c.to_rgba8(1.0);
        Color32::from_rgb(r, g, b)
    };
    let at = |k: usize| {
        let (i, j) = (k % (cols + 1), k / (cols + 1));
        vectorcraft_geom::kurbo::Point::new(b.width() * i as f64 / cols as f64, b.height() * j as f64 / rows as f64)
    };
    let n = (cols + 1) * (rows + 1);
    if gp.gradient.kind == GradientKind::Freeform {
        let Some(painted) = vectorcraft_color::freeform::painted_box(b) else { return vec![Color32::TRANSPARENT; n] };
        let field = gp.freeform_on(painted).field(vectorcraft_color::freeform::spread_scale(painted));
        return (0..n)
            .map(|k| {
                let ([r, g, b], _) = field.sample(at(k));
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                Color32::from_rgb(q(r), q(g), q(b))
            })
            .collect();
    }
    // A placed gradient keeps its direction; a fresh one is fitted to the chip at its angle.
    let angle = gp.geom.map_or(gp.angle, |g| g.angle_deg());
    let geom = GradientGeom::fit(gp.gradient.kind, b, if angle.is_finite() { angle } else { 0.0 });
    (0..n)
        .map(|k| {
            let t = geom.param_at(gp.gradient.kind, at(k)).clamp(0.0, 1.0) as f32;
            rgb(gp.gradient.sample(t).0)
        })
        .collect()
}

/// A Fill chip of `paint` into `rect` or, with `ring`, a Stroke chip: a frame `ring` wide round a
/// panel-coloured hole (returned; the whole chip for fills). A selection whose paints differ
/// (`mixed`) shows a white chip with a "?" (in the hole for strokes).
pub fn proxy_chip(ui: &Ui, rect: Rect, paint: &Paint, mixed: bool, ring: Option<f32>) -> Rect {
    let t = Tokens::get(ui.ctx());
    if mixed {
        ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    } else {
        paint_chip(ui, rect, paint);
    }
    let (hole, mark) = match ring {
        Some(w) => {
            let inner = rect.shrink(w);
            ui.painter().rect_filled(inner, 0.0, t.panel);
            (inner, t.text_strong)
        }
        None => (rect, Color32::BLACK),
    };
    if mixed {
        ui.painter().text(hole.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(hole.height() * 0.8), mark);
    }
    hole
}

/// A chip that opens something (the Control bar's and Properties' Fill/Stroke chips, the Control
/// bar's Style picker): `draw` paints the `size`-square chip, which gets a frame, and with
/// `chevron` a chevron follows it.
pub fn chip_button(ui: &mut Ui, size: f32, chevron: bool, draw: impl FnOnce(&Ui, Rect)) -> Response {
    chip_button_named(ui, size, chevron, "", draw)
}

/// [`chip_button`] with the tool tip `tip`, which screen readers also call it (none when empty).
pub fn chip_button_named(ui: &mut Ui, size: f32, chevron: bool, tip: &str, draw: impl FnOnce(&Ui, Rect)) -> Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(if chevron { size + 16.0 } else { size }, size), Sense::click());
    let chip = Rect::from_min_size(r.min, vec2(size, size)).shrink(2.0);
    draw(ui, chip);
    ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.input_border), StrokeKind::Outside);
    if chevron {
        icons::paint(ui, "chevron-down", Rect::from_center_size(pos2(chip.right() + 9.0, r.center().y), vec2(12.0, 12.0)), t.text_dim);
    }
    if tip.is_empty() {
        return resp;
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(tip)));
    resp.on_hover_text(tl!(tip))
}

/// What the user did on a [`fill_stroke_proxy`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxyClicks {
    pub fill: bool,
    pub stroke: bool,
    pub swap: bool,
    pub default: bool,
    /// Double-clicked a square: open the Color Picker for it (`true` = the stroke).
    pub pick: Option<bool>,
}

/// The Fill/Stroke proxy pair (two overlapping squares). `mixed` (fill, stroke) draws a "?" square
/// for a selection whose objects differ.
pub fn fill_stroke_proxy(ui: &mut Ui, fill: &Paint, stroke: &Paint, mixed: (bool, bool), fill_active: bool, size: f32) -> ProxyClicks {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let s = size * 0.62;
    let fill_r = Rect::from_min_size(rect.min + vec2(0.0, 0.0), Vec2::splat(s));
    let stroke_r = Rect::from_min_size(rect.max - Vec2::splat(s), Vec2::splat(s));
    let stroke_resp = ui.interact(stroke_r, ui.id().with("stroke-proxy"), Sense::click_and_drag());
    let fill_resp = ui.interact(fill_r, ui.id().with("fill-proxy"), Sense::click_and_drag());
    // A proxy drags its paint (onto the Swatches panel or art), unless it shows "?".
    for (resp, paint, mixed) in [(&fill_resp, fill, mixed.0), (&stroke_resp, stroke, mixed.1)] {
        if !mixed {
            drag_source(ui, resp, || PanelDrag::paint(paint.clone()));
        }
    }
    let draw_fill = |ui: &Ui| {
        proxy_chip(ui, fill_r, fill, mixed.0, None);
        ui.painter().rect_stroke(fill_r, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Inside);
        ui.painter().rect_stroke(fill_r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Outside);
    };
    let draw_stroke = |ui: &Ui| {
        let inner = proxy_chip(ui, stroke_r, stroke, mixed.1, Some(s * 0.28));
        ui.painter().rect_stroke(stroke_r, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Inside);
        ui.painter().rect_stroke(inner, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Outside);
        ui.painter().rect_stroke(stroke_r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Outside);
    };
    if fill_active {
        draw_stroke(ui);
        draw_fill(ui);
    } else {
        draw_fill(ui);
        draw_stroke(ui);
    }
    // Swap arrow (top-right) and default (bottom-left) mini buttons.
    let swap_r = Rect::from_min_size(pos2(rect.right() - size * 0.3, rect.top()), Vec2::splat(size * 0.3));
    let def_r = Rect::from_min_size(pos2(rect.left(), rect.bottom() - size * 0.3), Vec2::splat(size * 0.3));
    let swap = ui.interact(swap_r, ui.id().with("swap-proxy"), Sense::click());
    let def = ui.interact(def_r, ui.id().with("default-proxy"), Sense::click());
    icons::paint(ui, "dc-swap", swap_r.shrink(1.0), if swap.hovered() { t.text } else { t.icon });
    let a = Rect::from_min_size(def_r.min + vec2(1.0, 1.0), Vec2::splat(size * 0.14));
    let b = Rect::from_min_size(def_r.min + vec2(size * 0.1, size * 0.1), Vec2::splat(size * 0.14));
    ui.painter().rect_filled(b, 0.0, Color32::BLACK);
    ui.painter().rect_stroke(b, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    ui.painter().rect_filled(a, 0.0, Color32::WHITE);
    ui.painter().rect_stroke(a, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
    let pick = if fill_resp.double_clicked() {
        Some(false)
    } else if stroke_resp.double_clicked() {
        Some(true)
    } else {
        None
    };
    let on = ui.is_enabled();
    fill_resp
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, on, fill_active, tl!("Fill (X), double-click for the Color Picker")));
    stroke_resp
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, on, !fill_active, tl!("Stroke (X), double-click for the Color Picker")));
    swap.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tl!("Swap Fill and Stroke (Shift+X)")));
    def.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tl!("Default Fill and Stroke (D)")));
    ProxyClicks {
        fill: fill_resp.on_hover_text(tl!("Fill (X), double-click for the Color Picker")).clicked(),
        stroke: stroke_resp.on_hover_text(tl!("Stroke (X), double-click for the Color Picker")).clicked(),
        swap: swap.on_hover_text(tl!("Swap Fill and Stroke (Shift+X)")).clicked(),
        default: def.on_hover_text(tl!("Default Fill and Stroke (D)")).clicked(),
        pick,
    }
}

/// A compact dropdown of interface labels (shown in the UI language). Returns the chosen index.
pub fn dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[&str], width: f32) -> Option<usize> {
    dropdown_with(ui, id, current, options, width, |_| true)
}

/// [`dropdown`] of names that are user, file or system data (font styles, artboards, presets the
/// user saved, printers, profiles), shown as they are, never translated. For a list that mixes
/// built-in labels with such names, translate the built-in ones (`tl!`) before passing them.
pub fn dropdown_names(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[&str], width: f32) -> Option<usize> {
    combo(ui, id, current, width, false, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            if ui.add(egui::Button::selectable(*o == current, *o)).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// [`dropdown`] whose options can be disabled (`enabled(index)`: greyed and not choosable).
pub fn dropdown_with(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    options: &[&str],
    width: f32,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    combo(ui, id, tl!(current), width, false, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            if ui.add_enabled(enabled(i), egui::Button::selectable(*o == current, tl!(o))).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// The recessed combo box of [`dropdown`] showing `current`; `list` draws the options and returns
/// the chosen one. A `searchable` list (a search field over a list it scrolls itself) stays open
/// while it is clicked: it closes itself when an option is chosen.
pub(crate) fn combo<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    width: f32,
    searchable: bool,
    list: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R> {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(CornerRadius::same(3))
        .show(ui, |ui| {
            if searchable {
                return searchable_combo(ui, ui.id().with(id), current, width - 4.0, list);
            }
            let id = ui.id().with(id);
            let button_id = ui.make_persistent_id(egui::IdSalt::new(id));
            let enter_for_combo = ui.memory(|m| m.has_focus(button_id)) || egui::ComboBox::is_open(ui.ctx(), button_id);
            let response = egui::ComboBox::from_id_salt(id)
                .selected_text(egui::RichText::new(current).size(12.0))
                .width(width - 4.0)
                .height(ui.spacing().combo_height)
                .show_ui(ui, list);
            // The button or open list handles Enter before the modal can accept it.
            if enter_for_combo {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            }
            let chosen = response.inner.flatten();
            // egui closes the list on a click of the pointer only: an option picked with Space or
            // Enter closes it here, and the combo gives the keyboard back, so that Enter reaches the
            // dialog's OK (#2789).
            if chosen.is_some() {
                egui::Popup::close_all(ui.ctx());
                ui.memory_mut(|m| m.surrender_focus(button_id));
            }
            chosen
        })
        .inner
}

/// A dropdown whose list keeps its own search field and scrolling rows (the font menu): drawn as
/// egui's combo box, but in a popup without the scroll area egui puts around a combo's list, so
/// the field stays at the top and only the rows scroll (#555). A click outside closes it.
fn searchable_combo<R>(ui: &mut Ui, id: egui::Id, current: &str, width: f32, list: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    let popup = id.with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let pad = ui.spacing().button_padding;
    let icon = Vec2::splat(ui.spacing().icon_width);
    let galley = egui::WidgetText::from(egui::RichText::new(current).size(12.0)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width - 2.0 * pad.x - ui.spacing().icon_spacing - icon.x,
        egui::TextStyle::Button,
    );
    let height = (galley.size().y.max(icon.y) + 2.0 * pad.y).max(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    // Screen readers get what egui's combo box gives them: its role and the text it shows.
    resp.widget_info(|| egui::WidgetInfo {
        enabled: ui.is_enabled(),
        current_text_value: Some(current.to_owned()),
        ..egui::WidgetInfo::new(egui::WidgetType::ComboBox)
    });
    if ui.is_rect_visible(rect) {
        let v = if open { ui.visuals().widgets.open } else { *ui.style().interact(&resp) };
        ui.painter().rect(rect.expand(v.expansion), v.corner_radius, v.weak_bg_fill, v.bg_stroke, StrokeKind::Inside);
        let inner = rect.shrink2(pad);
        ui.painter().galley(egui::Align2::LEFT_CENTER.align_size_within_rect(galley.size(), inner).min, galley, v.text_color());
        // egui's combo arrow: a downward triangle.
        let arrow =
            Rect::from_center_size(egui::Align2::RIGHT_CENTER.align_size_within_rect(icon, inner).center(), vec2(icon.x * 0.7, icon.y * 0.45));
        ui.painter().add(egui::Shape::convex_polygon(
            vec![arrow.left_top(), arrow.right_top(), arrow.center_bottom()],
            v.fg_stroke.color,
            Stroke::NONE,
        ));
    }
    // It opens below the button, or above it when there is more room there, and the list gets
    // the room left (the popup's frame aside).
    const GAP: f32 = 12.0;
    let screen = ui.ctx().content_rect();
    let frame = ui.spacing().menu_margin.sum().y + GAP;
    let (below, above) = (screen.bottom() - rect.bottom() - frame, rect.top() - screen.top() - frame);
    let (align, room) = if below >= above { (egui::RectAlign::BOTTOM_START, below) } else { (egui::RectAlign::TOP_START, above) };
    egui::Popup::menu(&resp)
        .id(popup)
        .width(rect.width())
        .align(align)
        .align_alternatives(&[])
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(ui.available_width());
            ui.set_max_height(room);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            list(ui)
        })
        .and_then(|r| r.inner)
}

/// A popover anchored to `resp`, opened and closed by `toggle` (usually `resp.clicked()`), and
/// closed by Escape, a click outside it and outside any popup it opened, or a frame in which its
/// anchor isn't shown. Use it instead of `egui::Popup::menu` when the popover holds dropdowns or
/// popups of its own: egui remembers one open popup at a time, so opening a dropdown inside a
/// remembered popup would close the popup (and the dropdown with it). This one keeps its open
/// state itself: the last frame it was open in.
pub fn popover<R>(resp: &Response, toggle: bool, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let ctx = &resp.ctx;
    let id = resp.id.with("popover");
    let frame = ctx.cumulative_frame_nr();
    let was_open = ctx.data(|d| d.get_temp::<u64>(id)).is_some_and(|f| f + 1 >= frame);
    let mut open = if toggle {
        !was_open
    } else {
        // Popups (this one and those it opened) are on foreground layers; a click anywhere
        // else closes it.
        let clicked_elsewhere = ctx
            .input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()).flatten())
            .is_some_and(|pos| ctx.layer_id_at(pos).is_none_or(|l| l.order != egui::Order::Foreground));
        was_open && !clicked_elsewhere
    };
    let inner =
        egui::Popup::menu(resp).id(id).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(content).map(|r| r.inner);
    ctx.data_mut(|d| {
        if open {
            d.insert_temp(id, frame);
        } else {
            d.remove::<u64>(id);
        }
    });
    inner
}

/// Records where the bottom bars (status and hint bars) start this frame, so menus can stop above
/// them (#2086). Without bars it is the bottom of the window.
pub fn set_menu_floor(ctx: &egui::Context, y: f32) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("menu_floor"), y));
}

/// The height a menu list may take: from `top` down to `floor` less a small gap, at least 120 pt.
pub fn menu_room(floor: f32, top: f32) -> f32 {
    const BOTTOM_GAP: f32 = 12.0;
    const MIN_HEIGHT: f32 = 120.0;
    (floor - top - BOTTOM_GAP).max(MIN_HEIGHT)
}

/// The scroll bar of a menu's list: slim, in a column of its own beside the items, so it never
/// covers what sits at their right end (shortcuts, submenu arrows, a font's favourite star).
pub fn list_scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle { bar_width: 6.0, bar_inner_margin: 2.0, ..egui::style::ScrollStyle::solid() }
}

/// The body of a menu or popup list: as tall as its items up to the bottom bars of the window, and
/// scrolling only past that, so long menus (Window, Effect, a panel's menu) stay reachable on
/// small windows. Its scroll bar has a column of its own, so it never covers the shortcuts or the
/// submenu arrows (#2086).
pub fn menu_scroll<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let floor = ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::new("menu_floor"))).unwrap_or_else(|| ui.ctx().content_rect().bottom());
    let room = menu_room(floor, ui.next_widget_position().y);
    ui.scope(|ui| {
        ui.spacing_mut().scroll = list_scroll_style();
        // The minimum lets the list grow past a popup's default 400 pt size; it still shrinks to
        // its items when they need less.
        egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).show(ui, add_contents).inner
    })
    .inner
}

/// A column of tool or panel buttons (the toolbar, the dock's icon column) that scrolls when the
/// window is too short for it: with the mouse wheel, or the scroll bar shown while the pointer is
/// over it.
/// Its scroll bar stays a slim line at the edge, even while scrolled or dragged, so it never covers
/// the buttons (#808).
pub fn strip_scroll<R>(ui: &mut Ui, id: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().scroll = egui::style::ScrollStyle {
            bar_width: 4.0,
            floating_width: 2.0,
            bar_inner_margin: 0.0,
            bar_outer_margin: 1.0,
            ..egui::style::ScrollStyle::floating()
        };
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, true]).show(ui, add_contents).inner
    })
    .inner
}

/// Whether the blend-mode list draws a separator above `BlendMode::ALL[i]`: where a
/// [`BlendMode::group`] starts.
pub fn blend_separator_before(i: usize) -> bool {
    i > 0 && i < BlendMode::ALL.len() && BlendMode::ALL[i].group() != BlendMode::ALL[i - 1].group()
}

/// The blend-mode dropdown, its groups separated (Normal | darken | lighten | contrast | inversion
/// | component modes). `None` (objects that differ) shows blank. Returns the chosen mode.
pub fn blend_dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: Option<BlendMode>, width: f32) -> Option<BlendMode> {
    combo(ui, id, current.map_or("", |m| tl!(m.label())), width, false, |ui| {
        let mut chosen = None;
        for (i, m) in BlendMode::ALL.into_iter().enumerate() {
            if blend_separator_before(i) {
                ui.separator();
            }
            if ui.selectable_label(Some(m) == current, tl!(m.label())).clicked() {
                chosen = Some(m);
            }
        }
        chosen
    })
}

/// The blend mode an effect parameter names (`"mode": "multiply"`), which editors show as a
/// [`blend_param_dropdown`].
pub fn blend_param(key: &str, value: &serde_json::Value) -> Option<BlendMode> {
    value.as_str().filter(|_| key == "mode").and_then(BlendMode::parse)
}

/// [`blend_dropdown`] for a blend-mode parameter; returns the chosen mode's parameter value.
pub fn blend_param_dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: BlendMode) -> Option<serde_json::Value> {
    blend_dropdown(ui, id, Some(current), 120.0).map(|m| serde_json::Value::String(m.label().to_ascii_lowercase()))
}

/// An edit made with [`opacity_blend`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransparencyEdit {
    Blend(BlendMode),
    /// Opacity in percent, with the slider's drag phase (`Released` for a typed value).
    Opacity(f64, Live),
}

/// The transparency controls shared by the Transparency panel and the Appearance panel's Opacity
/// popups: the grouped blend-mode dropdown, the opacity percent field and its slider popup.
/// `opacity` (0..1) and `blend` are `None` where the objects differ (shown blank).
pub fn opacity_blend(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    opacity: Option<f32>,
    blend: Option<BlendMode>,
    enabled: bool,
) -> Option<TransparencyEdit> {
    let t = Tokens::get(ui.ctx());
    let mut edit = None;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(m) = blend_dropdown(ui, (id, "blend"), blend, 104.0) {
                edit = Some(TransparencyEdit::Blend(m));
            }
        });
        dim_label(ui, "Opacity:");
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(o) = mixed_field(ui, (id, "opacity"), opacity.map(|o| o as f64 * 100.0), "%", 0, 50.0) {
                edit = Some(TransparencyEdit::Opacity(o.clamp(0.0, 100.0), Live::Released));
            }
        });
        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
        ui.painter().rect_stroke(r, 2, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
        icons::paint(ui, "chevron-right", r.shrink2(vec2(3.0, 7.0)), if enabled { t.icon } else { t.text_disabled });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled && ui.is_enabled(), tl!("Opacity")));
        egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let mut o = opacity.unwrap_or(1.0) * 100.0;
            let r = slider(ui, egui::Slider::new(&mut o, 0.0..=100.0).show_value(false));
            let phase = if r.drag_stopped() || (r.changed() && !r.dragged()) {
                Live::Released
            } else if r.changed() {
                Live::Dragging
            } else {
                return;
            };
            edit = Some(TransparencyEdit::Opacity(o.round() as f64, phase));
        });
    });
    edit
}

/// The 3×3 reference point locator: nine squares, the eight outer ones joined by a line around
/// the box (the centre one stands alone), the current one filled. Returns a new index when clicked.
pub fn reference_point(ui: &mut Ui, current: usize) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    // 5 pt squares with 4 pt gaps; the origin is snapped to whole points so the 1 pt lines through
    // the squares' centres stay crisp.
    let (sq, step) = (5.0, 9.0);
    let size = 2.0 * step + sq;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let o = rect.min.round();
    let centre = |i: usize| o + Vec2::new(sq / 2.0 + (i % 3) as f32 * step, sq / 2.0 + (i / 3) as f32 * step);
    let line = Stroke::new(1.0, t.text_dim);
    ui.painter().rect_stroke(Rect::from_two_pos(centre(0), centre(8)), 0.0, line, StrokeKind::Middle);
    let mut out = None;
    // Screen readers name the squares left to right and top to bottom.
    let names = [
        tl!("Top Left"),
        tl!("Top"),
        tl!("Top Right"),
        tl!("Left"),
        tl!("Center"),
        tl!("Right"),
        tl!("Bottom Left"),
        tl!("Bottom"),
        tl!("Bottom Right"),
    ];
    for (i, name) in names.into_iter().enumerate() {
        let r = Rect::from_center_size(centre(i), Vec2::splat(sq));
        let resp = ui.interact(r.expand(2.0), ui.id().with(("refpt", i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), i == current, name));
        if resp.clicked() {
            out = Some(i);
        }
        if i == current {
            ui.painter().rect_filled(r, 0.0, t.text);
        } else {
            ui.painter().rect_filled(r, 0.0, t.panel);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, if resp.hovered() { t.text } else { t.text_dim }), StrokeKind::Inside);
        }
    }
    out
}

/// An angle dial `size` points across: a circle with a hand from its centre at `angle` (degrees,
/// counter-clockwise from 3 o'clock). Pressing or dragging points the hand at the pointer, in whole
/// degrees (-179..180), Shift in 45° steps. Returns the new angle.
pub fn angle_dial(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, angle: f64, size: f32) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let (c, r) = (rect.center(), size / 2.0 - 1.0);
    let ring = if resp.hovered() || resp.dragged() { t.text } else { t.input_border };
    ui.painter().circle(c, r, t.input, Stroke::new(1.0, ring));
    let a = (angle as f32).to_radians();
    ui.painter().line_segment([c, c + vec2(a.cos(), -a.sin()) * (r - 3.0)], Stroke::new(1.5, t.text));
    ui.painter().circle_filled(c, 2.0, t.text);
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), angle, tl!("Angle")));
    let (p, _) = pointer_phase(&resp)?;
    let v = p - c;
    if v.length() < 2.0 {
        return None;
    }
    let step = if ui.input(|i| i.modifiers.shift) { 45.0 } else { 1.0 };
    let deg = ((-v.y).atan2(v.x).to_degrees() as f64 / step).round() * step;
    // atan2 gives -180..180: -180 shows as 180.
    let deg = if deg <= -180.0 { deg + 360.0 } else { deg };
    (deg != angle).then_some(deg)
}

/// A toggle icon (e.g. eye / lock columns) — returns clicked.
pub fn toggle_icon(ui: &mut Ui, on_icon: &str, on: bool, size: f32, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if on {
        icons::paint(ui, on_icon, rect.shrink(size * 0.2), if resp.hovered() { t.text } else { t.icon });
    } else if resp.hovered() {
        icons::paint(ui, on_icon, rect.shrink(size * 0.2), t.text_disabled);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), on, tl!(tip)));
    resp.on_hover_text(tl!(tip)).clicked()
}

// ---------- panel widgets (Swatches, Color, Stroke, Gradient, Appearance, …) ----------

/// Icon button that can be disabled (greyed, no hover, never clicked).
pub fn icon_button_enabled(ui: &mut Ui, icon: &str, tip: &str, selected: bool, enabled: bool, size: f32) -> Response {
    if enabled {
        return icon_button(ui, icon, tip, selected, size);
    }
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let pad = (size * 0.2).round();
    icons::paint(ui, icon, rect.shrink(pad), t.text_disabled);
    resp.widget_info(|| icon_button_info(false, selected, tl!(tip)));
    if tip.is_empty() { resp } else { resp.on_hover_text(tl!(tip)) }
}

/// What screen readers get for an icon button: a button called `name` (already translated),
/// reported as pressed while `selected` and with no pressed state otherwise.
pub(crate) fn icon_button_info(enabled: bool, selected: bool, name: &str) -> egui::WidgetInfo {
    egui::WidgetInfo { selected: selected.then_some(true), ..egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name) }
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

/// Regular-weight panel sub-header ("Shape Modes:", "Align Objects:").
pub fn subheader(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(text)).size(12.5).color(t.text));
}

/// A label drawn with a dotted underline (Illustrator's link labels: "Stroke:", "Opacity:").
pub fn link_label(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), egui::FontId::proportional(12.5), t.text_strong);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(0.0, 3.0), Sense::click());
    let y = rect.top() + galley.size().y + 1.0;
    ui.painter().galley(rect.min, galley, t.text_strong);
    let mut x = rect.left();
    while x < rect.right() - 1.0 {
        ui.painter().line_segment([pos2(x, y), pos2((x + 1.0).min(rect.right()), y)], Stroke::new(1.0, t.text_dim));
        x += 2.5;
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(text)));
    resp
}

/// The row of a checkbox or radio button: allocates a 13 pt box plus `text` (already translated),
/// draws the text and returns the box, the response and the box's border colour.
fn choice_row(ui: &mut Ui, text: &str, enabled: bool) -> (Rect, Response, Color32) {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(12.5), if enabled { t.text } else { t.text_disabled });
    let (rect, resp) =
        ui.allocate_exact_size(vec2(18.0 + galley.size().x, 20.0f32.max(galley.size().y)), if enabled { Sense::click() } else { Sense::hover() });
    let bx = Rect::from_min_size(pos2(rect.left(), rect.center().y - 6.5), Vec2::splat(13.0));
    ui.painter().galley(pos2(bx.right() + 5.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    let border = if !enabled {
        t.divider
    } else if resp.hovered() {
        t.text
    } else {
        t.button_border
    };
    (bx, resp, border)
}

/// Panel-style checkbox with a disabled state. Returns true when toggled.
pub fn check(ui: &mut Ui, label: &str, value: bool, enabled: bool) -> bool {
    check3(ui, label, Some(value), enabled)
}

/// [`check`] with tooltip `tip` on the box and its label. Returns true when toggled.
pub fn check_tip(ui: &mut Ui, label: &str, value: bool, enabled: bool, tip: &str) -> bool {
    let (clicked, resp) = check_row(ui, tl!(label), Some(value), enabled);
    resp.on_hover_text(tip);
    clicked
}

/// Three-state [`check`]: `None` shows a dash (a neutral or mixed state). Returns true when clicked.
pub fn check3(ui: &mut Ui, label: &str, value: Option<bool>, enabled: bool) -> bool {
    check_row(ui, tl!(label), value, enabled).0
}

/// [`check`] for a name of the document's (an artboard's, an asset's), shown as it is, never
/// translated. Returns true when toggled.
pub fn check_name(ui: &mut Ui, name: &str, value: bool) -> bool {
    check_row(ui, name, Some(value), true).0
}

/// A [`check`] box without text, for a row whose label is drawn beside it in another column:
/// screen readers call it `name` (already translated). Returns true when toggled.
pub fn check_box(ui: &mut Ui, name: &str, value: bool, enabled: bool) -> bool {
    check_row_named(ui, "", name, Some(value), enabled).0
}

/// [`check3`] with its `text` already translated, also returning the row's response (what a
/// tooltip goes on: a widget around it never counts as hovered under the box).
fn check_row(ui: &mut Ui, text: &str, value: Option<bool>, enabled: bool) -> (bool, Response) {
    check_row_named(ui, text, text, value, enabled)
}

/// [`check_row`] that screen readers call `name` instead of its `text`.
fn check_row_named(ui: &mut Ui, text: &str, name: &str, value: Option<bool>, enabled: bool) -> (bool, Response) {
    let t = Tokens::get(ui.ctx());
    let (bx, resp, border) = choice_row(ui, text, enabled);
    let checked = value == Some(true);
    ui.painter().rect_filled(bx, CornerRadius::same(2), if checked && enabled { t.accent_strong } else { t.input });
    ui.painter().rect_stroke(bx, CornerRadius::same(2), Stroke::new(1.0, border), StrokeKind::Inside);
    match value {
        Some(true) => {
            let c = if enabled { Color32::WHITE } else { t.text_disabled };
            ui.painter().line_segment([bx.left_center() + vec2(3.0, 0.0), bx.center_bottom() + vec2(-1.0, -3.5)], Stroke::new(1.6, c));
            ui.painter().line_segment([bx.center_bottom() + vec2(-1.0, -3.5), bx.right_top() + vec2(-3.0, 3.0)], Stroke::new(1.6, c));
        }
        None => {
            let c = if enabled { t.text } else { t.text_disabled };
            ui.painter().line_segment([bx.left_center() + vec2(3.0, 0.0), bx.right_center() - vec2(3.0, 0.0)], Stroke::new(1.6, c));
        }
        Some(false) => {}
    }
    resp.widget_info(|| match value {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled && ui.is_enabled(), on, name),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, enabled && ui.is_enabled(), name),
    });
    (enabled && resp.clicked(), resp)
}

/// Radio button in the style of [`check`], with a disabled state. Returns true when clicked.
pub fn radio(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> bool {
    radio_row(ui, tl!(label), tl!(label), selected, enabled)
}

/// A [`radio`] button without text, for one followed by a field and the text that names it:
/// screen readers call it `name` (already translated). Returns true when clicked.
pub fn radio_button(ui: &mut Ui, name: &str, selected: bool, enabled: bool) -> bool {
    radio_row(ui, "", name, selected, enabled)
}

/// [`radio`] with its `text` already translated, which screen readers call `name`.
fn radio_row(ui: &mut Ui, text: &str, name: &str, selected: bool, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (bx, resp, border) = choice_row(ui, text, enabled);
    let c = bx.center();
    ui.painter().circle(c, 6.0, if selected && enabled { t.accent_strong } else { t.input }, Stroke::new(1.0, border));
    if selected {
        ui.painter().circle_filled(c, 2.5, if enabled { Color32::WHITE } else { t.text_disabled });
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled && ui.is_enabled(), selected, name));
    enabled && resp.clicked()
}

/// Numeric field with an up/down spinner on the left and a preset dropdown on the right
/// (Stroke weight, font size…): an editable combo box whose field keeps typing, ↑/↓ stepping and
/// label scrubbing. `value` (points) shows in `unit`, blank when `None` (the selection's values
/// differ); `presets` (points) are listed in `unit` too, and empty = no dropdown. Returns the
/// committed, stepped or picked value.
#[allow(clippy::too_many_arguments)]
pub fn spin_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: Option<f64>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    spin_field_auto(ui, id, value, None, unit, width, step, min, presets).and_then(SpinPick::value)
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
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: Option<f64>,
    auto: Option<bool>,
    unit: Unit,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<SpinPick> {
    spin_generic(ui, value, auto, width, step, min, presets, &|v| unit.format(v), &mut |ui, fw| num_field(ui, id, value, unit, fw))
}

/// [`spin_field`] for unitless values (percent, degrees, 1/1000 em) with a display suffix; a
/// `None` value (the selection's values differ) shows blank.
#[allow(clippy::too_many_arguments)]
pub fn spin_plain(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: impl Into<Option<f64>>,
    suffix: &str,
    decimals: usize,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
) -> Option<f64> {
    let value = value.into();
    let fmt = |v: f64| format!("{v}{suffix}");
    spin_generic(ui, value, None, width, step, min, presets, &fmt, &mut |ui, fw| mixed_field(ui, id, value, suffix, decimals, fw))
        .and_then(SpinPick::value)
}

#[allow(clippy::too_many_arguments)]
fn spin_generic(
    ui: &mut Ui,
    value: Option<f64>,
    auto: Option<bool>,
    width: f32,
    step: f64,
    min: f64,
    presets: &[f64],
    fmt: &dyn Fn(f64) -> String,
    field: &mut dyn FnMut(&mut Ui, f32) -> Option<f64>,
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
        sresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tl!("Step up or down")));
        // A blank field (values that differ) has nothing to step from.
        if sresp.clicked()
            && let Some(p) = sresp.interact_pointer_pos()
            && let Some(v) = value
        {
            let modifiers = ui.input(|i| i.modifiers);
            out = Some(SpinPick::Value(spinner_step(v, step, min, up.contains(p), modifiers)));
        }
        let fw = if presets.is_empty() { width - 15.0 } else { width - 34.0 };
        if let Some(v) = field(ui, fw) {
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
fn spinner_step(value: f64, step: f64, min: f64, up: bool, modifiers: egui::Modifiers) -> f64 {
    let multiplier = STEPS.iter().find(|(m, _)| modifiers.matches_logically(*m)).map_or(1.0, |(_, factor)| *factor);
    let delta = step * multiplier;
    (value + if up { delta } else { -delta }).max(min)
}

/// Drag phase of a live slider/handle: previews while dragging, commits on release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Live {
    Idle,
    Dragging,
    Released,
}

/// A colour slider with a gradient track (`track(t)` gives the colour at 0..1) and a triangular
/// thumb below it, like Illustrator's Color panel. Returns the new normalized value and the phase.
pub fn color_slider(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    color_slider_named(ui, id, "", value, width, track)
}

/// [`color_slider`] that screen readers get as a slider named `name` (already translated) with no
/// value (none when `name` is empty).
pub fn color_slider_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: &str,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    color_slider_valued(ui, id, name, None, value, width, track)
}

/// The value a slider gives screen readers: the number the field beside it shows, the range of the
/// slider's rail and the field's text.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderValue {
    pub value: f64,
    pub range: std::ops::RangeInclusive<f64>,
    /// The field's text, such as "50%"; `None` gives the number alone.
    pub text: Option<String>,
}

impl SliderValue {
    /// `value` on a rail over `range`, without a text.
    pub fn number(value: f64, range: std::ops::RangeInclusive<f64>) -> Self {
        Self { value, range, text: None }
    }

    /// `value` on a rail over `range` as [`plain_field`] shows it with `suffix` and at most
    /// `decimals` places: the number it shows (`value` rounded) and its text.
    pub fn field(value: f64, range: std::ops::RangeInclusive<f64>, suffix: &str, decimals: usize) -> Self {
        let number = field_text(value, "", decimals);
        Self { value: number.parse().unwrap_or(value), range, text: Some(format!("{number}{suffix}")) }
    }

    /// `value` on a rail over `range`, with `text`, for a field that writes its own text (a length
    /// in the document's unit, an egui `DragValue`).
    pub fn text(value: f64, range: std::ops::RangeInclusive<f64>, text: String) -> Self {
        Self { value, range, text: Some(text) }
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
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, enabled, name));
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
    id: impl std::hash::Hash + std::fmt::Debug,
    name: &str,
    shown: Option<SliderValue>,
    value: f32,
    width: f32,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let bar = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 4.0), vec2(width - 8.0, 7.0));
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    gradient_strip(ui, bar, (bar.width() / 2.0).ceil().max(2.0) as usize, false, track);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let v = value.clamp(0.0, 1.0);
    let x = bar.left() + v * bar.width();
    let tip = pos2(x, bar.bottom() - 1.0);
    let thumb = vec![tip, pos2(x + 5.5, tip.y + 6.0), pos2(x + 5.5, tip.y + 10.0), pos2(x - 5.5, tip.y + 10.0), pos2(x - 5.5, tip.y + 6.0)];
    let fill = if resp.dragged() || resp.hovered() { Color32::WHITE } else { t.icon };
    ui.painter().add(egui::Shape::convex_polygon(thumb, fill, Stroke::new(1.0, t.border)));
    slider_info(ui, &resp, name, shown);
    match pointer_phase(&resp) {
        Some((p, phase)) => (Some(((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0)), phase),
        None => (None, Live::Idle),
    }
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

/// The Color Picker's 2D colour field: `color_at(x, y)` gives the colour at a point (0..1 each, y
/// down) and a ring marks `pos`. Returns the clicked or dragged point and the phase.
pub fn color_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    size: Vec2,
    pos: (f32, f32),
    color_at: &dyn Fn(f32, f32) -> Color32,
) -> (Option<(f32, f32)>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(6.0));
    painter.add(color_mesh(rect, (32, 32), color_at));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    let c = pos2(rect.left() + pos.0.clamp(0.0, 1.0) * rect.width(), rect.top() + pos.1.clamp(0.0, 1.0) * rect.height());
    painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::BLACK));
    painter.circle_stroke(c, 4.0, Stroke::new(1.0, Color32::WHITE));
    match pointer_phase(&resp) {
        Some((p, phase)) => {
            let f = |v: f32, lo: f32, len: f32| ((v - lo) / len).clamp(0.0, 1.0);
            (Some((f(p.x, rect.left(), rect.width()), f(p.y, rect.top(), rect.height()))), phase)
        }
        None => (None, Live::Idle),
    }
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
pub fn channel_slider(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f32,
    size: Vec2,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    channel_slider_named(ui, id, "", value, size, track)
}

/// [`channel_slider`] that screen readers get as a slider named `name` (already translated) with
/// no value, as for [`color_slider_named`] (none when `name` is empty).
pub fn channel_slider_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
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
    id: impl std::hash::Hash + std::fmt::Debug,
    name: &str,
    shown: Option<SliderValue>,
    value: f32,
    size: Vec2,
    track: &dyn Fn(f32) -> Color32,
) -> (Option<f32>, Live) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
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
pub fn hex_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, hex: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new("#").size(15.0).color(t.text));
    let id = ui.id().with(id);
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
            // As tall as the other fields (26 points with the margin), whatever the font's line height.
            ui.set_height(18.0);
            ui.add(
                egui::TextEdit::singleline(&mut buf)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(62.0)
                    .char_limit(7)
                    .font(egui::FontId::proportional(12.5))
                    .text_color(t.text_strong),
            )
        })
        .inner;
    let commit = (resp.lost_focus() || kept) && buf != hex;
    let focused = ui.memory(|m| m.has_focus(id));
    ui.data_mut(|d| {
        d.insert_temp(id, buf.clone());
        d.insert_temp(edited_id, (edited || resp.changed()) && focused && !kept);
    });
    commit.then_some(buf)
}

/// One swatch tile (Illustrator: 1 px dark frame, white inset on hover/selection).
pub fn swatch_tile(ui: &Ui, rect: Rect, paint: &Paint, selected: bool, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    paint_chip(ui, rect, paint);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    if selected || hovered {
        ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, if selected { t.accent } else { t.text }), StrokeKind::Outside);
    }
}

/// A recessed search field with an italic `hint` (Layers' Search All, Swatches' Find), keeping its
/// text in egui memory under `id`. Returns the text.
pub fn search_field(ui: &mut Ui, id: egui::Id, hint: &str) -> String {
    let t = Tokens::get(ui.ctx());
    let mut query: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut query)
                    .id(id.with("edit"))
                    .frame(egui::Frame::NONE)
                    .hint_text(egui::RichText::new(tl!(hint)).italics())
                    .desired_width(ui.available_width()),
            );
        });
    ui.data_mut(|d| d.insert_temp(id, query.clone()));
    query
}

/// A bordered list box (Swatches tiles, Appearance rows, Artboards list).
pub fn list_box<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.stroke(Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(1)).show(ui, add).inner
}

/// A bottom button bar separated from the content by a divider (panel footers).
pub fn bottom_bar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.divider);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        add(ui)
    });
}

/// A menu row for panel (≡) menus: label, optional check mark, disabled when not implemented.
pub fn menu_item(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    menu_item_name(ui, tl!(label), enabled, checked)
}

/// [`menu_item`] for a name that is user or file data (a library, a recent URL), never translated.
/// Like the menu bar's items, the row lights up under the pointer.
pub fn menu_item_name(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    let text = if checked { format!("✓ {label}") } else { format!("   {label}") };
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(12.5)).frame_when_inactive(false)).clicked()
}

/// A number field that may be empty (Stroke dash/gap): `value` (points) shows in `unit` without
/// its suffix, to fit narrow fields. Returns `Some(new)` (points) on commit (Enter, Shift+Enter or
/// focus loss, as in [`num_field`]), where `new` is `None` when the field was cleared; typed units
/// and arithmetic work as in [`num_field`].
pub fn opt_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: Option<f64>, unit: Unit, width: f32) -> Option<Option<f64>> {
    let t = Tokens::get(ui.ctx());
    let id = ui.id().with(id);
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
    select_all_on_focus(ui, &resp, &buf);
    // ↑/↓ and the wheel step a value (an empty field stays empty); a drag on its label scrubs it.
    let stepped = step_value(ui, &resp, &mut buf, |b| Some(unit.from_pt(unit.parse(b)?)), |v| unit.number(unit.to_pt(v)));
    let scrubbed = scrub::field(ui, id, framed.response.rect, &buf, value.map(|v| unit.from_pt(v)), STEP_DECIMALS);
    let focused = ui.memory(|m| m.has_focus(id));
    ui.data_mut(|d| {
        d.insert_temp(id, buf.clone());
        d.insert_temp(edited_id, (edited || resp.changed()) && focused && !kept);
    });
    if (resp.lost_focus() || kept) && buf != shown {
        let s = buf.trim();
        if s.is_empty() { Some(None) } else { typed_unit(ui.ctx(), unit).parse(s).map(Some) }
    } else {
        stepped.or(scrubbed).map(|v| Some(unit.to_pt(v)))
    }
}

/// What panels drag onto art and onto each other. One payload type, so the canvas has one drop
/// handler for all of them (`canvas::panel_drop`); a chip of a dragged paint follows the pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelDrag {
    /// A paint: Swatches panel tiles, a Fill/Stroke proxy or the Gradient panel's thumbnail. Art it
    /// is dropped on takes `params` (`{swatch}`, `{color}`, `{gradient}`…; null for a colour group,
    /// which paints nothing) through the active proxy's `paint.setFill`/`paint.setStroke`; the
    /// Gradient panel's ramp takes its colour as a stop; the Swatches panel moves `rows`, or makes
    /// a swatch of a paint dragged from elsewhere. `library`: the Libraries panel colour it is
    /// (library id, colour name), which that panel moves to where it is dropped.
    Paint { paint: Paint, params: serde_json::Value, rows: Option<SwatchRows>, library: Option<(String, String)> },
    /// The Appearance panel's thumbnail: art it is dropped on takes object `0`'s appearance
    /// (`appearance.copyFrom`); the Graphic Styles panel makes a style of it.
    Appearance(vectorcraft_doc::NodeId),
    /// A Graphic Styles panel style: art it is dropped on takes it (`graphicStyle.apply` with its
    /// `ids`); the panel moves it to where it is dropped.
    GraphicStyle(String),
    /// The selected art, dragged off the canvas with the Selection tool: the Graphic Styles panel
    /// makes a style of the first object (`graphicStyle.new`), the Symbols panel a symbol of it all
    /// (`symbol.new`), the Swatches panel a pattern swatch of a copy (`object.pattern.make`) and the
    /// Brushes panel an Art brush (`brush.new`).
    Art(Vec<vectorcraft_doc::NodeId>),
    /// A Symbols panel symbol: the canvas places an instance of it centred where it is dropped
    /// (`symbol.place`).
    Symbol(String),
    /// A Brushes panel brush (`def`: its definition, for the chip at the pointer): the path it is
    /// dropped on takes it (`brush.apply`).
    Brush { name: String, def: serde_json::Value },
    /// A Libraries panel graphic (`item`: its id in `library`): the canvas places a copy centred
    /// where it is dropped (`library.use`); the panel moves it to where it is dropped.
    LibraryGraphic { library: String, item: String },
    /// A Libraries panel text style (`item`: its name in `library`), which the panel moves to where
    /// it is dropped.
    LibraryItem { library: String, kind: vectorcraft_engine::cmd::library::ItemKind, item: String },
    /// A Libraries panel group, by its header, which the panel moves to where it is dropped.
    LibraryGroup { library: String, group: String },
}

/// A panel list `zone` that takes art dragged off the canvas: outlined while art is held over it;
/// → the dragged ids when it is released there.
pub fn art_drop(ui: &Ui, zone: &Response) -> Option<Vec<u64>> {
    let drag = zone.dnd_hover_payload::<PanelDrag>()?;
    let PanelDrag::Art(ids) = &*drag else { return None };
    ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
    zone.dnd_release_payload::<PanelDrag>().map(|_| ids.iter().map(|id| id.0).collect())
}

/// The Swatches panel rows a drag from that panel moves: the swatch, None, Registration or colour
/// group under the pointer when the drag started (`grabbed`) and `names`, the panel selection when
/// the grabbed one is part of it (only colour groups, or only swatches, like the grabbed one), in
/// panel order.
#[derive(Clone, Debug, PartialEq)]
pub struct SwatchRows {
    pub grabbed: String,
    pub names: Vec<String>,
    /// The names are colour groups.
    pub groups: bool,
}

impl PanelDrag {
    /// `paint` dragged from a Fill/Stroke proxy or the Gradient panel's thumbnail. A gradient
    /// carries no placement: it fits the art it lands on (freeform points are placed afresh there,
    /// coloured like the dragged ones).
    pub fn paint(mut paint: Paint) -> Self {
        if let Paint::Gradient(g) = &mut paint {
            g.geom = None;
            g.freeform = None;
        }
        Self::Paint { params: crate::panels::paint_params(&paint), paint, rows: None, library: None }
    }
    /// The dragged colour (a solid paint's), which the Gradient panel's ramp takes.
    pub fn color(&self) -> Option<vectorcraft_color::Color> {
        match self {
            Self::Paint { paint, .. } => paint.color(),
            _ => None,
        }
    }
}

/// Make `resp` (which senses drags) a source of the [`PanelDrag`] `drag` builds when a drag starts
/// on it.
pub fn drag_source(ui: &Ui, resp: &Response, drag: impl FnOnce() -> PanelDrag) {
    if resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), drag());
    }
}

/// A small document rendered to a texture at the screen's pixel density (stroke previews: brushes,
/// arrowheads), transparent where nothing paints. `build` makes the `w`×`h` pt document only when
/// the texture for `key` at this size isn't cached yet.
pub fn doc_preview(ui: &Ui, key: &str, size: Vec2, build: impl FnOnce(f64, f64) -> Option<vectorcraft_doc::Document>) -> Option<egui::TextureHandle> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<String, egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let ppp = ui.ctx().pixels_per_point() as f64;
    let (w, h) = (size.x as f64, size.y as f64);
    let key = format!("{w}x{h}@{ppp}:{key}");
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let doc = build(w, h)?;
    let img = RENDERER.with(|r| r.borrow_mut().render_region(&doc, vectorcraft_geom::Rect::new(0.0, 0.0, w, h), ppp, false));
    let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
    let tex = ui.ctx().load_texture(format!("preview-{key}"), color, egui::TextureOptions::LINEAR);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    Some(tex)
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
        let drag = ui.id().with((id, "drag"));
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
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), tl!("Color wheel")));
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

/// A row of text tabs, the current one underlined in the accent colour (New Document's
/// categories, Document Setup's sections). Returns the clicked tab's index.
pub fn tab_bar(ui: &mut Ui, tabs: &[&str], current: usize) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (i, tab) in tabs.iter().enumerate() {
            let sel = i == current;
            let font = if sel { theme::semibold(13.0) } else { egui::FontId::proportional(13.0) };
            let galley = ui.painter().layout_no_wrap(tl!(tab).to_string(), font, t.text);
            let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x, 30.0), Sense::click());
            let color = if sel || resp.hovered() { t.text_strong } else { t.text_dim };
            ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0 - 2.0), galley, color);
            if sel {
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.right_bottom()),
                    CornerRadius::same(1),
                    t.accent,
                );
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), sel, tl!(tab)));
            if resp.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// A page-orientation toggle drawn in code: a portrait or landscape sheet with a folded corner,
/// accent-filled when `selected`. Returns clicked.
pub fn orientation_button(ui: &mut Ui, landscape: bool, selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if resp.hovered() && !selected {
        ui.painter().rect_filled(rect, CornerRadius::same(3), t.hover);
    }
    let size = if landscape { vec2(16.0, 12.0) } else { vec2(12.0, 16.0) };
    let page = Rect::from_center_size(rect.center(), size);
    let color = if selected { t.accent } else { t.icon };
    let fold = 4.0;
    let outline =
        vec![page.left_top(), page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(0.0, fold), page.right_bottom(), page.left_bottom()];
    let fill = if selected { t.accent } else { Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(outline.clone(), fill, Stroke::new(1.2, color)));
    let corner = [page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(-fold, fold), page.right_top() + vec2(0.0, fold)];
    ui.painter().add(egui::Shape::line(corner.to_vec(), Stroke::new(1.2, if selected { t.panel } else { color })));
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, tl!(tip)));
    resp.on_hover_text(tl!(tip)).clicked()
}

/// An artboard-layout toggle drawn in code (Rearrange All Artboards): small artboards in the grid
/// cells `tiles` (column, row), tinted, and over them their order as a line from a dot on the first
/// to an arrowhead on the last; accent-drawn in a pressed well when `selected`, greyed in a disabled
/// `ui`. `tip` is shown as it is (the caller translates it). Returns clicked.
pub fn layout_button(ui: &mut Ui, tiles: &[(u8, u8)], selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let enabled = ui.is_enabled();
    let bg = if selected {
        t.tool_active
    } else if resp.hovered() && enabled {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), bg);
    let color = if !enabled {
        t.text_disabled
    } else if selected {
        t.accent
    } else {
        t.icon
    };
    // Cells at most 10 points a side in a 20-point square: a row's artboards stand tall, a
    // column's lie flat.
    let (cols, rows) = tiles.iter().fold((1u8, 1u8), |(c, r), &(x, y)| (c.max(x.saturating_add(1)), r.max(y.saturating_add(1))));
    let cell = vec2((20.0 / f32::from(cols)).min(10.0), (20.0 / f32::from(rows)).min(10.0));
    let origin = rect.center() - vec2(f32::from(cols) * cell.x, f32::from(rows) * cell.y) / 2.0;
    let centre = |&(x, y): &(u8, u8)| origin + vec2((f32::from(x) + 0.5) * cell.x, (f32::from(y) + 0.5) * cell.y);
    for tile in tiles {
        ui.painter().rect_filled(Rect::from_center_size(centre(tile), cell - vec2(2.0, 2.0)), 1.0, color.gamma_multiply(0.35));
    }
    let path: Vec<Pos2> = tiles.iter().map(centre).collect();
    if let ([first, ..], [.., from, to]) = (path.as_slice(), path.as_slice()) {
        let (first, from, to) = (*first, *from, *to);
        let dir = (to - from).normalized();
        let side = dir.rot90() * 2.5;
        let head = vec![to + dir * 1.5, to - dir * 2.5 + side, to - dir * 2.5 - side];
        ui.painter().circle_filled(first, 1.6, color);
        ui.painter().add(egui::Shape::line(path, Stroke::new(1.2, color)));
        ui.painter().add(egui::Shape::convex_polygon(head, color, Stroke::NONE));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, tip));
    resp.on_hover_text(tip).clicked()
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
    let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

/// The chevron cell on the right of a field that opens its preset list (rounded on the right).
fn chevron_cell(ui: &mut Ui, h: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (dr, dresp) = ui.allocate_exact_size(vec2(20.0, h), Sense::click());
    ui.painter().rect_filled(dr, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, t.input);
    ui.painter().rect_stroke(dr, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let c = if !ui.is_enabled() {
        t.text_disabled
    } else if dresp.hovered() {
        t.text_strong
    } else {
        t.icon
    };
    icons::paint(ui, "chevron-down", Rect::from_center_size(dr.center(), Vec2::splat(12.0)), c);
    dresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Presets")));
    dresp
}

/// A [`text_field`] with a preset list on its right (Export for Screens' sizes: `2x`, `100w`…).
/// Returns the new text when a change is committed (Enter or focus loss) or a preset is picked.
pub fn text_presets(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &str, presets: &[&str], width: f32) -> Option<String> {
    let mut out = None;
    ui.scope(|ui| {
        // The field and the chevron share a border.
        ui.spacing_mut().item_spacing.x = -1.0;
        out = text_field(ui, &id, Some(value), width - 19.0, 1);
        let resp = chevron_cell(ui, 26.0);
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(width - 10.0);
            for p in presets {
                if ui.selectable_label(*p == value, *p).clicked() {
                    out = Some((*p).to_string());
                }
            }
        });
    });
    out
}

#[cfg(test)]
pub(crate) mod accessibility_tests {
    use egui::accesskit::{Node, Role, Toggled};

    use super::*;

    /// The screen-reader tree of the second of two frames of `add` in a 1280 × 900 window, in
    /// English.
    pub(crate) fn tree(mut add: impl FnMut(&mut Ui)) -> Vec<Node> {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0));
        let mut nodes = vec![];
        for time in [0.0, 0.1] {
            let input = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, &mut add);
            out.textures_delta.clear();
            nodes = out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default();
        }
        nodes
    }

    /// The node of `tree` that screen readers name `label`.
    pub(crate) fn named<'a>(tree: &'a [Node], label: &str) -> &'a Node {
        tree.iter().find(|n| n.label() == Some(label)).unwrap_or_else(|| panic!("nothing is named {label:?}"))
    }

    /// The role of the node named `label`, and whether it is pressed, checked or chosen.
    pub(crate) fn role_and_state(tree: &[Node], label: &str) -> (Role, Option<Toggled>) {
        let n = named(tree, label);
        (n.role(), n.toggled())
    }

    #[test]
    fn buttons_are_named_by_their_text_or_tip() {
        let tree = tree(|ui| {
            icon_button(ui, "x", "Remove", false, 24.0);
            flat_button(ui, "Make", 64.0);
            split_button_named(ui, "Image Trace", "Tracing Presets", 104.0);
            primary_button(ui, "OK");
            primary_button_enabled(ui, "Create", false);
            secondary_button(ui, "Cancel");
            link_label(ui, "Stroke:");
            icon_button_enabled(ui, "trash-2", "Delete Brush", false, false, 24.0);
            fill_stroke_proxy(ui, &Paint::None, &Paint::None, (false, false), true, 36.0);
            chip_button_named(ui, 22.0, true, "Graphic Style", |_, _| {});
            spin_field(ui, "weight", Some(1.0), Unit::Points, 80.0, 1.0, 0.0, &[1.0, 2.0]);
            opacity_blend(ui, "opacity", Some(1.0), Some(BlendMode::Normal), true);
        });
        for label in [
            "Remove",
            "Make",
            "Image Trace",
            "Tracing Presets",
            "OK",
            "Create",
            "Cancel",
            "Stroke:",
            "Delete Brush",
            "Swap Fill and Stroke (Shift+X)",
            "Graphic Style",
            "Step up or down",
            "Presets",
            "Opacity",
        ] {
            assert_eq!(role_and_state(&tree, label), (Role::Button, None), "{label}");
        }
        assert_eq!(role_and_state(&tree, "Default Fill and Stroke (D)"), (Role::Button, None));
        assert_eq!(role_and_state(&tree, "Fill (X), double-click for the Color Picker"), (Role::Button, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Stroke (X), double-click for the Color Picker"), (Role::Button, Some(Toggled::False)));
        assert!(named(&tree, "Create").is_disabled(), "a primary button that is not enabled");
        assert!(named(&tree, "Delete Brush").is_disabled(), "an icon button that is not enabled");
        assert!(!named(&tree, "OK").is_disabled());
    }

    #[test]
    fn toggles_say_whether_they_are_on() {
        let tree = tree(|ui| {
            icon_button(ui, "link", "Constrain Proportions", true, 24.0);
            toggle_icon(ui, "eye", true, 18.0, "Show all plates");
            toggle_icon(ui, "eye", false, 18.0, "Show/hide plate (Alt-click: only this plate)");
            orientation_button(ui, false, true, "Portrait");
            orientation_button(ui, true, false, "Landscape");
            layout_button(ui, &[(0, 0), (1, 0)], true, "Grid by Row");
            tab_bar(ui, &["Recent", "Saved"], 1);
        });
        let on = |label| assert_eq!(role_and_state(&tree, label), (Role::Button, Some(Toggled::True)), "{label}");
        let off = |label| assert_eq!(role_and_state(&tree, label), (Role::Button, Some(Toggled::False)), "{label}");
        on("Constrain Proportions");
        on("Show all plates");
        off("Show/hide plate (Alt-click: only this plate)");
        on("Portrait");
        off("Landscape");
        on("Grid by Row");
        off("Recent");
        on("Saved");
    }

    #[test]
    fn checkboxes_and_radio_buttons_are_named_and_checked() {
        let tree = tree(|ui| {
            check(ui, "Preview", true, true);
            check(ui, "Snap to Pixel", false, false);
            check3(ui, "Overprint Fill", None, true);
            check_name(ui, "Artboard 1", true);
            check_box(ui, "Cyan", false, true);
            radio(ui, "Relative", true, true);
            radio(ui, "Absolute", false, true);
            radio_button(ui, "per slice", true, true);
            reference_point(ui, 4);
        });
        assert_eq!(role_and_state(&tree, "Preview"), (Role::CheckBox, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Snap to Pixel"), (Role::CheckBox, Some(Toggled::False)));
        assert!(named(&tree, "Snap to Pixel").is_disabled());
        assert_eq!(role_and_state(&tree, "Overprint Fill"), (Role::CheckBox, Some(Toggled::Mixed)));
        assert_eq!(role_and_state(&tree, "Artboard 1"), (Role::CheckBox, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Relative"), (Role::RadioButton, Some(Toggled::True)));
        assert_eq!(role_and_state(&tree, "Absolute"), (Role::RadioButton, Some(Toggled::False)));
        assert_eq!(role_and_state(&tree, "Cyan"), (Role::CheckBox, Some(Toggled::False)));
        assert_eq!(role_and_state(&tree, "per slice"), (Role::RadioButton, Some(Toggled::True)));
        for (i, label) in ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"].into_iter().enumerate()
        {
            let chosen = if i == 4 { Toggled::True } else { Toggled::False };
            assert_eq!(role_and_state(&tree, label), (Role::RadioButton, Some(chosen)), "{label}");
        }
    }

    #[test]
    fn sliders_dials_and_combo_boxes_are_named() {
        let red = Color::from_hsb(0.0, 1.0, 1.0);
        let tree = tree(|ui| {
            color_slider_named(ui, "fidelity", "Fidelity:", 0.25, 120.0, &|_| Color32::GRAY);
            angle_dial(ui, "angle", 45.0, 28.0);
            channel_slider_named(ui, "channel", "H", 0.5, vec2(22.0, 80.0), &|_| Color32::GRAY);
            harmony_wheel(ui, "wheel", 120.0, &[red], Some(0));
            combo(ui, "font", "Regular", 120.0, true, |_| None::<usize>);
        });
        let slider = |label| {
            let n = named(&tree, label);
            (n.role(), n.numeric_value())
        };
        assert_eq!(slider("Fidelity:"), (Role::Slider, None));
        assert_eq!(slider("Angle"), (Role::Slider, Some(45.0)));
        assert_eq!(slider("H"), (Role::Slider, None));
        assert_eq!(slider("Color wheel"), (Role::Slider, None));
        assert!(tree.iter().any(|n| n.role() == Role::ComboBox && n.value() == Some("Regular")), "the searchable combo box shows its text");
    }

    /// The number, range and text of the slider of `tree` that screen readers name `label`.
    pub(crate) fn slider_value(tree: &[Node], label: &str) -> (Option<f64>, Option<f64>, Option<f64>, Option<String>) {
        let n = tree.iter().find(|n| n.role() == Role::Slider && n.label() == Some(label)).unwrap_or_else(|| panic!("no slider is named {label:?}"));
        (n.numeric_value(), n.min_numeric_value(), n.max_numeric_value(), n.value().map(str::to_string))
    }

    /// The `_valued` sliders give screen readers the number and text of their field, with the rail's
    /// range extended to a number past it; the harmony wheel's Brightness gives its percent.
    #[test]
    fn sliders_report_the_value_their_field_shows() {
        let dark_red = Color::from_hsb(0.0, 1.0, 0.5);
        let gray = |_: f32| Color32::GRAY;
        let tree = tree(|ui| {
            color_slider_valued(ui, "opacity", "Opacity:", Some(SliderValue::field(49.6, 0.0..=100.0, "%", 0)), 0.5, 120.0, &gray);
            color_slider_valued(ui, "scale", "Horizontal:", Some(SliderValue::field(312.5, 0.0..=200.0, "%", 2)), 1.0, 120.0, &gray);
            color_slider_valued(ui, "move", "Vertical:", Some(SliderValue::text(-1.5, -14.0..=14.0, "-1.5 in".into())), 0.4, 120.0, &gray);
            color_slider_valued(ui, "count", "Ridges per segment:", Some(SliderValue::number(4.0, 0.0..=100.0)), 0.04, 120.0, &gray);
            color_slider_valued(ui, "axis", "Weight", None, 0.5, 120.0, &gray);
            channel_slider_valued(ui, "channel", "H", Some(SliderValue::field(210.0, 0.0..=360.0, "°", 0)), 210.0 / 360.0, vec2(22.0, 80.0), &gray);
            harmony_wheel(ui, "wheel", 120.0, &[dark_red], Some(0));
        });
        let s = |label| slider_value(&tree, label);
        assert_eq!(s("Opacity:"), (Some(50.0), Some(0.0), Some(100.0), Some("50%".into())));
        assert_eq!(s("Horizontal:"), (Some(312.5), Some(0.0), Some(312.5), Some("312.5%".into())));
        assert_eq!(s("Vertical:"), (Some(-1.5), Some(-14.0), Some(14.0), Some("-1.5 in".into())));
        assert_eq!(s("Ridges per segment:"), (Some(4.0), Some(0.0), Some(100.0), None));
        assert_eq!(s("Weight"), (None, None, None, None), "a mixed value");
        assert_eq!(s("H"), (Some(210.0), Some(0.0), Some(360.0), Some("210°".into())));
        assert_eq!(s("Brightness"), (Some(50.0), Some(0.0), Some(100.0), Some("50%".into())));
    }
}

#[cfg(test)]
mod spinner_modifier_tests {
    use super::*;

    #[test]
    fn shift_steps_stroke_weight_by_ten_points() {
        assert_eq!(spinner_step(14.0, 1.0, 0.0, true, egui::Modifiers::SHIFT), 24.0);
        assert_eq!(spinner_step(14.0, 1.0, 0.0, false, egui::Modifiers::SHIFT), 4.0);
    }

    #[test]
    fn unmodified_and_command_steps_keep_existing_behavior() {
        assert_eq!(spinner_step(14.0, 1.0, 0.0, true, egui::Modifiers::NONE), 15.0);
        assert_eq!(spinner_step(14.0, 1.0, 0.0, false, egui::Modifiers::COMMAND), 13.9);
        assert_eq!(spinner_step(0.0, 1.0, 0.0, false, egui::Modifiers::SHIFT), 0.0);
    }
}

#[cfg(test)]
mod step_to_multiple_tests {
    use super::*;

    /// #2881: the first step goes to the next multiple of its size in its direction, and each further
    /// step adds or takes one size.
    #[test]
    fn a_step_goes_to_the_next_multiple_of_its_size() {
        for (v, size, n, want) in [
            (5.1, 1.0, 1.0, 6.0),
            (5.1, 1.0, -1.0, 5.0),
            (5.0, 1.0, 1.0, 6.0),
            (5.0, 1.0, -1.0, 4.0),
            (-2.5, 1.0, 1.0, -2.0),
            (-2.5, 1.0, -1.0, -3.0),
            (24.8, 1.0, 2.0, 26.0),
            (24.8, 1.0, -2.0, 23.0),
            (5.13, 0.1, 1.0, 5.2),
            (5.13, 0.1, -1.0, 5.1),
            (5.1, 1.0, 0.0, 5.1),
        ] {
            assert!((step_to_multiple(v, size, n) - want).abs() < 1e-9, "{v} by {size} × {n}: {}", step_to_multiple(v, size, n));
        }
    }

    /// A value a float error away from a multiple moves a whole step from it: 0.7 is 6.999… tenths.
    #[test]
    fn a_value_next_to_a_multiple_moves_a_whole_step() {
        assert!((step_to_multiple(0.7, 0.1, 1.0) - 0.8).abs() < 1e-9);
        assert!((step_to_multiple(0.3, 0.1, -1.0) - 0.2).abs() < 1e-9);
        assert!((step_to_multiple(5.000_000_1, 1.0, 1.0) - 6.0).abs() < 1e-9);
        assert!((step_to_multiple(4.999_999_9, 1.0, -1.0) - 4.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod menu_room_tests {
    use super::*;

    #[test]
    fn menu_stops_above_the_bottom_bars() {
        // Window 1000 pt tall, bars start at 948: a menu opening at y = 100 gets 836 pt, not 888.
        assert_eq!(menu_room(948.0, 100.0), 836.0);
    }

    #[test]
    fn menu_keeps_a_minimum_height() {
        assert_eq!(menu_room(300.0, 290.0), 120.0);
    }
}

#[cfg(test)]
mod gradient_chip_tests {
    use super::*;
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint};

    fn paint(kind: GradientKind, angle: f64) -> GradientPaint {
        let mut gp = GradientPaint::new(Gradient { kind, ..Gradient::default() });
        gp.angle = angle;
        gp
    }

    #[test]
    fn linear_chip_follows_the_angle() {
        // 5 x 5 grid points on a 20 x 20 chip; White (start) to Black (end).
        let at = |colors: &[Color32], i: usize, j: usize| colors[j * 5 + i];
        let side = gradient_chip_colors(&paint(GradientKind::Linear, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!(at(&side, 0, 2), Color32::WHITE);
        assert_eq!(at(&side, 4, 2), Color32::BLACK);
        assert_eq!(at(&side, 2, 0), at(&side, 2, 4));
        // 90 degrees runs bottom to top: constant along rows, white at the bottom.
        let up = gradient_chip_colors(&paint(GradientKind::Linear, 90.0), 20.0, 20.0, 4, 4);
        assert_eq!(at(&up, 0, 4), Color32::WHITE);
        assert_eq!(at(&up, 0, 0), Color32::BLACK);
        assert_eq!(at(&up, 0, 2), at(&up, 4, 2));
    }

    #[test]
    fn radial_chip_runs_from_the_centre_out() {
        let colors = gradient_chip_colors(&paint(GradientKind::Radial, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!(colors[2 * 5 + 2], Color32::WHITE);
        assert_eq!(colors[0], Color32::BLACK);
    }
}

#[cfg(test)]
mod combo_keyboard_tests {
    use super::*;

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, picked: &mut Option<usize>) {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if let Some(i) = dropdown(ui, "combo-test", "One", &["One", "Two", "Three"], 200.0) {
                *picked = Some(i);
            }
        });
        // No renderer takes the font atlas updates in a test; dropping them unapplied panics.
        out.textures_delta.clear();
    }

    /// An option picked with the keyboard closes the list (it used to stay open and keep the keys).
    #[test]
    fn option_picked_with_space_closes_the_list() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut picked = None;
        frame(&ctx, vec![], &mut picked);
        frame(&ctx, vec![], &mut picked);
        // Tab to the combo, open it with Space.
        for k in [egui::Key::Tab, egui::Key::Space] {
            frame(&ctx, vec![key(k, true)], &mut picked);
            frame(&ctx, vec![key(k, false)], &mut picked);
        }
        assert!(egui::Popup::is_any_open(&ctx), "Space opens the list");
        // Tab into the list, then Space picks the option under the focus.
        for _ in 0..8 {
            frame(&ctx, vec![key(egui::Key::Tab, true)], &mut picked);
            frame(&ctx, vec![key(egui::Key::Tab, false)], &mut picked);
            frame(&ctx, vec![key(egui::Key::Space, true)], &mut picked);
            frame(&ctx, vec![key(egui::Key::Space, false)], &mut picked);
            if picked.is_some() {
                break;
            }
        }
        assert!(picked.is_some(), "an option was picked");
        frame(&ctx, vec![], &mut picked);
        assert!(!egui::Popup::is_any_open(&ctx), "the list closes after a keyboard pick");
    }
}
