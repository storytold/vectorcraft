use super::{Color32, CornerRadius, Rect, Response, Sense, Stroke, Tokens, Ui, Vec2, icons, pos2, theme, vec2};

/// Square icon button; `selected` draws the pressed well.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, selected: bool, size: f32) -> Response {
    let (_, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_icon_button(ui, resp, icon, tip, selected)
}

/// [`icon_button`] drawn for a response the caller made (its own id or sense).
pub fn paint_icon_button(ui: &mut Ui, resp: Response, icon: &str, tip: &str, selected: bool) -> Response {
    paint_icon_button_named(ui, resp, icon, tl!(tip), tl!(tip), selected)
}

/// [`paint_icon_button`] that screen readers call `name` and that shows the tool tip `tip` (none
/// when empty), both as given.
pub(crate) fn paint_icon_button_named(ui: &mut Ui, resp: Response, icon: &str, name: &str, tip: &str, selected: bool) -> Response {
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
    resp.widget_info(|| icon_button_info(ui.is_enabled(), selected, name));
    if !tip.is_empty() { resp.on_hover_text(tip) } else { resp }
}

/// Small flat text button (Quick Actions style).
pub fn flat_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.button);
    flat_fill(ui, rect, &resp);
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, tl!(text), egui::FontId::proportional(12.5), t.text_strong);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
    resp
}

/// A [`flat_button`] that expands to fit localized labels wider than `min_width`.
pub fn flat_button_fit(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let width = flat_button_fit_width(ui, text, min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.button);
    flat_fill(ui, rect, &resp);
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, tl!(text), egui::FontId::proportional(12.5), t.text_strong);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
    resp
}

/// The width [`flat_button_fit`] gives `text`, translated: its label and padding, at least
/// `min_width`.
pub(crate) fn flat_button_fit_width(ui: &Ui, text: &str, min_width: f32) -> f32 {
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), egui::FontId::proportional(12.5), Color32::PLACEHOLDER);
    (galley.size().x + 24.0).max(min_width)
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
    let main = ui.interact(main_rect, ui.scope_id().with((text, "split")), Sense::click());
    let arrow = ui.interact(arrow_rect, ui.scope_id().with((text, "split-arrow")), Sense::click());
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
    main.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
    if arrow_tip.is_empty() {
        return (main, arrow);
    }
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(arrow_tip)));
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

/// The width [`primary_button`] and [`secondary_button`] give `text` (their label plus padding, at
/// least 72), for centring a row of them.
pub(crate) fn button_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), theme::semibold(12.5), Color32::PLACEHOLDER);
    (galley.size().x + 32.0).max(72.0)
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled && ui.is_enabled(), tl!(text)));
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
    resp
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, tl!(tip)));
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
    egui::WidgetInfo { selected: selected.then_some(true), ..egui::WidgetInfo::labeled(egui::Role::Button, enabled, name) }
}

/// A menu row for panel (≡) menus: label, optional check mark, disabled when not implemented.
pub fn menu_item(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    menu_item_name(ui, tl!(label), enabled, checked)
}

/// [`menu_item`] that returns the row's response, for a caller that needs to know how it was
/// clicked.
pub(crate) fn menu_item_response(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> egui::Response {
    menu_item_name_response(ui, tl!(label), enabled, checked)
}

/// [`menu_item`] for a name that is user or file data (a library, a recent URL), never translated.
/// Like the menu bar's items, the row lights up under the pointer.
pub fn menu_item_name(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    menu_item_name_response(ui, label, enabled, checked).clicked()
}

/// [`menu_item_name`] that returns the row's response.
fn menu_item_name_response(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> egui::Response {
    let text = check_row_text(ui, if checked { "✓ " } else { "   " }, label, Some(12.5));
    ui.add_enabled(enabled, egui::Button::new(text).frame_when_inactive(false))
}

/// [`menu_item`] that shows `hint` under the pointer while the row is disabled, to say why it is
/// gray.
pub fn menu_item_hint(ui: &mut Ui, label: &str, enabled: bool, checked: bool, hint: &str) -> bool {
    let text = check_row_text(ui, if checked { "✓ " } else { "   " }, tl!(label), Some(12.5));
    ui.add_enabled(enabled, egui::Button::new(text).frame_when_inactive(false)).on_disabled_hover_text(tl!(hint)).clicked()
}

/// The width of the check-mark column of the in-app menus: room for the widest mark a row puts
/// before its label (a check mark and two spaces, or five spaces) in the button font. A font
/// without the check mark cannot draw one, so only the spaces count then.
fn check_column_width(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    once_per_pass(ui, egui::Id::unique(("menu-check-column-width", &font)), || {
        ui.ctx().fonts_mut(|f| {
            let mark = if f.has_glyph(&font, '✓') { "✓  " } else { "     " };
            let mut width = |text: &str| f.layout_no_wrap(text.to_string(), font.clone(), Color32::PLACEHOLDER).size().x;
            width(mark).max(width("     "))
        })
    })
}

/// The width of a row's `mark` in `font`.
fn mark_width(ui: &Ui, mark: &str, font: &egui::FontId) -> f32 {
    once_per_pass(ui, egui::Id::unique(("menu-check-mark-width", mark, font)), || {
        ui.ctx().fonts_mut(|f| f.layout_no_wrap(mark.to_string(), font.clone(), Color32::PLACEHOLDER).size().x)
    })
}

/// `measure()`, worked out on the first call of a pass and kept under `id` for the rest of it:
/// every row of a menu (one per font family in Type › Font) asks for the same widths.
fn once_per_pass(ui: &Ui, id: egui::Id, measure: impl FnOnce() -> f32) -> f32 {
    let pass = ui.ctx().cumulative_pass_nr();
    if let Some((at, width)) = ui.data(|d| d.get_temp::<(u64, f32)>(id))
        && at == pass
    {
        return width;
    }
    let width = measure();
    ui.data_mut(|d| d.insert_temp(id, (pass, width)));
    width
}

/// Starts the text of a menu row for `label` with `mark` (a check mark, blank room for one, or
/// nothing) in `format`, and returns the leading space for the label after it. Every row's label
/// then starts at the end of the same check column, whatever the width of its mark, as in native
/// menus. A label with a line that runs right to left keeps its mark as plain text before it and
/// no leading space, so it is laid out in its own direction as before.
pub(crate) fn check_column_job(ui: &Ui, mark: &str, label: &str, format: &egui::TextFormat) -> (egui::text::LayoutJob, f32) {
    let mut job = egui::text::LayoutJob::default();
    if !mark.is_empty() {
        job.append(mark, 0.0, format.clone());
    }
    if label.split(is_paragraph_separator).any(|line| vectorcraft_text::paragraph_is_rtl(line, None)) {
        return (job, 0.0);
    }
    let mark_width = if mark.is_empty() { 0.0 } else { mark_width(ui, mark, &format.font_id) };
    (job, (check_column_width(ui) - mark_width).max(0.0))
}

/// Does `c` end a paragraph for the bidirectional algorithm (bidi class B: line feed, carriage
/// return, the information separators U+001C to U+001E, next line and the paragraph separator)?
fn is_paragraph_separator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{1C}'..='\u{1E}' | '\u{85}' | '\u{2029}')
}

/// A menu row's text: `mark`, then `label` at the end of the check column
/// ([`check_column_job`]), in the button font at `size` when one is given.
pub(crate) fn check_row_text(ui: &Ui, mark: &str, label: &str, size: Option<f32>) -> egui::WidgetText {
    let mut font_id = egui::TextStyle::Button.resolve(ui.style());
    if let Some(size) = size {
        font_id.size = size;
    }
    let format = egui::TextFormat { font_id, color: Color32::PLACEHOLDER, ..Default::default() };
    let (mut job, lead) = check_column_job(ui, mark, label, &format);
    job.append(label, lead, format);
    job.into()
}
