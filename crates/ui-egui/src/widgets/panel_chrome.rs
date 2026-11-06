use super::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, icons, pos2, theme, vec2};

pub(crate) const PANEL_FOOTER: &str = "vectorcraft-panel-footer";
const FOOTER_HEIGHT: f32 = 31.0;

/// The finite body edge supplied by a sized dock, float or user-resized flyout.
/// Untouched flyouts omit it so their content can still determine their height.
pub(crate) fn panel_footer_bottom(ui: &Ui) -> Option<f32> {
    ui.stack().iter().find_map(|stack| stack.tags().get_downcast::<Option<f32>>(PANEL_FOOTER)).copied().flatten()
}

pub(crate) fn panel_list_height(ui: &Ui, fallback: f32, after: f32) -> f32 {
    panel_footer_bottom(ui).map_or(fallback, |bottom| (bottom - ui.next_widget_position().y - FOOTER_HEIGHT - after).max(0.0))
}

pub(crate) fn panel_footer_space(ui: &mut Ui, height: f32) {
    if let Some(bottom) = panel_footer_bottom(ui) {
        ui.add_space((bottom - ui.next_widget_position().y - height).max(0.0));
    }
}

/// Pin a custom footer (for example the Variables panel's two rows).
pub(crate) fn panel_footer(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    let Some(bottom) = panel_footer_bottom(ui) else { return add(ui) };
    let top = bottom - height;
    if ui.next_widget_position().y > top {
        ui.add_space(height);
    }
    let rect = Rect::from_min_size(pos2(ui.max_rect().left(), top), vec2(ui.available_width(), height));
    ui.painter().rect_filled(rect, 0.0, Tokens::get(ui.ctx()).panel);
    // Own the blank parts too, so a click cannot select a scrolled row behind the footer.
    let _background = ui.interact(rect, ui.scope_id().with("panel-footer-background"), Sense::CLICK);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add);
    ui.advance_cursor_after_rect(rect);
}

/// A bordered list box (Swatches tiles, Appearance rows, Artboards list).
pub fn list_box<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.stroke(Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(1)).show(ui, add).inner
}

/// A bottom button bar separated from the content by a divider (panel footers).
pub fn bottom_bar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    if let Some(bottom) = panel_footer_bottom(ui) {
        // Reserve scrollable room after long content so its final row can move above
        // the pinned footer instead of remaining hidden underneath its buttons.
        let top = bottom - FOOTER_HEIGHT;
        if ui.next_widget_position().y > top {
            ui.add_space(FOOTER_HEIGHT);
        }
        let footer = Rect::from_min_size(pos2(ui.max_rect().left(), top), vec2(ui.available_width(), FOOTER_HEIGHT));
        ui.painter().rect_filled(footer, 0.0, t.panel);
        let _background = ui.interact(footer, ui.scope_id().with("panel-footer-background"), Sense::CLICK);
        ui.painter().hline(footer.x_range(), top + 4.0, Stroke::new(1.0, t.divider));
        let row = Rect::from_min_max(pos2(footer.left(), top + 7.0), footer.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            add(ui);
        });
        ui.advance_cursor_after_rect(footer);
        return;
    }
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
                    CornerRadius::same(1),
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, tip));
    resp.on_hover_text(tip).clicked()
}

/// The chevron cell on the right of a field that opens its preset list (rounded on the right).
pub(super) fn chevron_cell(ui: &mut Ui, h: f32) -> Response {
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
    dresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Presets")));
    dresp
}

#[cfg(test)]
mod footer_tests {
    use super::{PANEL_FOOTER, bottom_bar, panel_footer};

    #[test]
    fn blank_footer_space_does_not_click_a_row_scrolled_behind_it() {
        use egui::{Event, PointerButton, Rect, Sense, UiBuilder, UiStackInfo, pos2};
        for custom in [false, true] {
            let ctx = egui::Context::default();
            let mut clicked = false;
            let at = pos2(150.0, 190.0);
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            for events in [vec![], vec![], vec![Event::PointerMoved(at), button(true)], vec![button(false)]] {
                let input =
                    egui::RawInput { screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0))), events, ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    let info = UiStackInfo::default().with_tag_value(PANEL_FOOTER, Some(200.0_f32));
                    ui.scope_builder(UiBuilder::new().ui_stack_info(info), |ui| {
                        clicked |= ui.allocate_rect(Rect::from_min_max(pos2(0.0, 100.0), pos2(200.0, 200.0)), Sense::click()).clicked();
                        let contents = |ui: &mut egui::Ui| {
                            let _button = ui.button("Footer button");
                        };
                        if custom {
                            panel_footer(ui, 31.0, contents);
                        } else {
                            bottom_bar(ui, contents);
                        }
                    });
                });
                output.textures_delta.clear();
            }
            assert!(!clicked, "custom={custom}: a hidden row received the footer click");
        }
    }
}
