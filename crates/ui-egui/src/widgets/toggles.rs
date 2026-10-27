use super::*;

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
        Some(on) => egui::WidgetInfo::selected(egui::Role::CheckBox, enabled && ui.is_enabled(), on, name),
        None => egui::WidgetInfo::labeled(egui::Role::CheckBox, enabled && ui.is_enabled(), name),
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled && ui.is_enabled(), selected, name));
    enabled && resp.clicked()
}
