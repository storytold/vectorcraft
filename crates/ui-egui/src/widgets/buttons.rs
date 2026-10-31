use super::{Color32, CornerRadius, Rect, Response, Sense, Stroke, Tokens, Ui, Vec2, icons, pos2, theme, vec2};

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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
    resp
}

/// A [`flat_button`] that expands to fit localized labels wider than `min_width`.
pub fn flat_button_fit(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(tl!(text).to_string(), egui::FontId::proportional(12.5), t.text_strong);
    let width = (galley.size().x + 24.0).max(min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.button);
    flat_fill(ui, rect, &resp);
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, tl!(text), egui::FontId::proportional(12.5), t.text_strong);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
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

/// [`menu_item`] for a name that is user or file data (a library, a recent URL), never translated.
/// Like the menu bar's items, the row lights up under the pointer.
pub fn menu_item_name(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    let text = if checked { format!("✓ {label}") } else { format!("   {label}") };
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(12.5)).frame_when_inactive(false)).clicked()
}
