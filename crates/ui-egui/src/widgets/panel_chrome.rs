use super::{Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, icons, pos2, theme, vec2};

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
                    Tokens::get(ui.ctx()).cr_small(),
                    t.accent,
                );
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), sel, tl!(tab)));
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
        ui.painter().rect_filled(rect, Tokens::get(ui.ctx()).cr(), t.hover);
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, tl!(tip)));
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
    ui.painter().rect_filled(rect, Tokens::get(ui.ctx()).cr(), bg);
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, tip));
    resp.on_hover_text(tip).clicked()
}

/// The field at `field`, before a preset chevron, when it has the keyboard focus and Alt+↓ (with
/// no other modifier) was pressed. The key is taken before the field is drawn, so that the field
/// does not step on it; the caller then opens the preset list ([`open_presets_from_keys`]).
pub(super) fn alt_down_in_field(ui: &Ui, field: Rect) -> Option<egui::Id> {
    let focused = ui.memory(|m| m.focused())?;
    let in_field = ui.ctx().read_response(focused).is_some_and(|r| field.expand(1.0).contains_rect(r.rect));
    let alt_down = |e: &egui::Event| matches!(e, egui::Event::Key { key: egui::Key::ArrowDown, pressed: true, modifiers, .. } if modifiers.matches_exact(egui::Modifiers::ALT));
    let pressed = in_field
        && ui.is_enabled()
        && ui.input_mut(|i| {
            let before = i.events.len();
            i.events.retain(|e| !alt_down(e));
            i.events.len() != before
        });
    pressed.then_some(focused)
}

/// Opens the preset list of `chevron` when Alt+↓ was pressed in `field` ([`alt_down_in_field`]),
/// and remembers the field so that [`return_focus_from_presets`] gives it the keyboard focus back
/// once the list closes. Returns whether it opened the list.
pub(super) fn open_presets_from_keys(ui: &Ui, chevron: &Response, field: Option<egui::Id>) -> bool {
    let Some(field) = field else { return false };
    let popup = egui::Popup::default_response_id(chevron);
    egui::Popup::open_id(ui.ctx(), popup);
    ui.data_mut(|d| d.insert_temp(popup.with("return focus"), field));
    true
}

/// Whether the preset list of `chevron` is open, read before the list is drawn for
/// [`keep_enter_in_presets`].
pub(super) fn presets_open(ui: &Ui, chevron: &Response) -> bool {
    egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(chevron))
}

/// Takes this frame's Enter presses after an open preset list was drawn: Enter picks the focused
/// entry and does not also reach the dialog's OK, as in [`super::dropdowns`]' combo boxes.
pub(super) fn keep_enter_in_presets(ui: &Ui, was_open: bool) {
    if was_open {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
}

/// After the preset list of `chevron` was drawn: when a list that Alt+↓ opened has closed (an
/// entry was picked, or Escape or a click outside closed it), the field gets the keyboard focus
/// back, unless the focus has moved on to a widget outside the list.
pub(super) fn return_focus_from_presets(ui: &Ui, chevron: &Response) {
    let popup = egui::Popup::default_response_id(chevron);
    let key = popup.with("return focus");
    let Some(field) = ui.data(|d| d.get_temp::<egui::Id>(key)) else { return };
    if egui::Popup::is_id_open(ui.ctx(), popup) {
        return;
    }
    ui.data_mut(|d| d.remove::<egui::Id>(key));
    // The list's entries are drawn in the popup's own layer, whose id is the popup id.
    let elsewhere = ui.memory(|m| m.focused()).and_then(|id| ui.ctx().read_response(id)).is_some_and(|r| r.layer_id.id != popup);
    if !elsewhere {
        ui.memory_mut(|m| m.request_focus(field));
    }
}

/// The entry of a preset list that gets the keyboard focus when Alt+↓ opened the list: the
/// chosen one, else the first. `None` when the list was not opened from the keys.
pub(super) fn entry_to_focus(opened_from_keys: bool, chosen: impl IntoIterator<Item = bool>) -> Option<usize> {
    opened_from_keys.then(|| chosen.into_iter().position(|c| c).unwrap_or(0))
}

/// The chevron cell on the right of a field that opens its preset list (rounded on the right). It
/// is not a Tab stop: Tab goes from the field to the next field, and Alt+↓ in the field opens the
/// list ([`alt_down_in_field`]).
pub(super) fn chevron_cell(ui: &mut Ui, h: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (dr, dresp) = ui.allocate_exact_size(vec2(20.0, h), Sense::CLICK);
    ui.painter().rect_filled(dr, super::group_end(&t), t.input);
    ui.painter().rect_stroke(dr, super::group_end(&t), Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let c = if !ui.is_enabled() {
        t.text_disabled
    } else if dresp.hovered() {
        t.text_strong
    } else {
        t.icon
    };
    icons::paint(ui, "chevron-down", Rect::from_center_size(dr.center(), Vec2::splat(12.0)), c);
    dresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Presets")));
    dresp
}
