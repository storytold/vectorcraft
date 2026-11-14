use super::{Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, pos2, vec2};

/// The row of a checkbox or radio button: allocates a 13 pt box plus `text` (already translated),
/// draws the text and returns the box, the response and the box's border colour. A checkbox row
/// (`drag`) also senses drags, for [`drag_toggle`].
fn choice_row(ui: &mut Ui, text: &str, enabled: bool, drag: bool) -> (Rect, Response, Color32) {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(12.5), if enabled { t.text } else { t.text_disabled });
    let (rect, resp) = ui.allocate_exact_size(
        vec2(18.0 + galley.size().x, 20.0f32.max(galley.size().y)),
        match (enabled, drag) {
            (true, true) => Sense::click_and_drag(),
            (true, false) => Sense::click(),
            (false, _) => Sense::hover(),
        },
    );
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
    check3_tip(ui, label, Some(value), enabled, tip)
}

/// [`check3`] with tooltip `tip` on the box and its label. Returns true when clicked.
pub fn check3_tip(ui: &mut Ui, label: &str, value: Option<bool>, enabled: bool, tip: &str) -> bool {
    let (clicked, resp) = check_row(ui, tl!(label), value, enabled);
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
    let (bx, resp, border) = choice_row(ui, text, enabled, true);
    let toggled = enabled && drag_toggle(ui, &resp, value);
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
    (toggled, resp)
}

/// The state a drag across checkboxes sets (#1477), while the primary button is down.
fn drag_target_id() -> egui::Id {
    egui::Id::unique("checkbox-drag-target")
}

/// Click or drag across checkboxes (#1477). A click toggles the box. A press and drag toggles it
/// and keeps its new state; every other checkbox the pointer then goes over takes that state, as
/// the eye and lock columns of the Layers panel do, until the button is released. `resp` is the
/// row's response (it senses drags) and `value` the box's state (`None`: mixed, which a drag
/// leaves alone). Returns true when the box must toggle: the caller flips it, as for a click.
fn drag_toggle(ui: &Ui, resp: &Response, value: Option<bool>) -> bool {
    let ctx = ui.ctx();
    if !ctx.input(|i| i.pointer.primary_down()) {
        ctx.data_mut(|d| d.remove::<bool>(drag_target_id()));
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        ctx.data_mut(|d| d.insert_temp(drag_target_id(), value != Some(true)));
        return true;
    }
    if resp.clicked() {
        return true;
    }
    let Some(target) = ctx.data(|d| d.get_temp::<bool>(drag_target_id())) else { return false };
    let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| resp.rect.contains(p));
    over && value == Some(!target)
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
    let (bx, resp, border) = choice_row(ui, text, enabled, false);
    let c = bx.center();
    ui.painter().circle(c, 6.0, if selected && enabled { t.accent_strong } else { t.input }, Stroke::new(1.0, border));
    if selected {
        ui.painter().circle_filled(c, 2.5, if enabled { Color32::WHITE } else { t.text_disabled });
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled && ui.is_enabled(), selected, name));
    enabled && resp.clicked()
}

#[cfg(test)]
mod drag_toggle_tests {
    use egui::{Event, PointerButton, Pos2};

    use super::*;

    /// Runs one frame of a column of checkboxes over `state` (`None`: mixed) with `events`,
    /// toggling a box as its callers do. Returns the rect of each row.
    fn frame(ctx: &egui::Context, state: &mut [Option<bool>], events: Vec<Event>) -> Vec<Rect> {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))), events, ..Default::default() };
        let mut rects = vec![];
        let mut out = ctx.run_ui(input, |ui| {
            rects.clear();
            for v in state.iter_mut() {
                let (toggled, resp) = check_row(ui, "Box", *v, true);
                if toggled {
                    *v = Some(*v != Some(true));
                }
                rects.push(resp.rect);
            }
        });
        out.textures_delta.clear();
        rects
    }

    fn press(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// Moves the pointer from `a` to `b` in small steps, one frame each.
    fn sweep(ctx: &egui::Context, state: &mut [Option<bool>], a: Pos2, b: Pos2) {
        for i in 1..=8 {
            frame(ctx, state, vec![Event::PointerMoved(a + (b - a) * (i as f32 / 8.0))]);
        }
    }

    /// #1477: a press on a checkbox and a drag over its neighbours sets them all to the state the
    /// first one switched to. A box past the drag, and a mixed box, stay as they are.
    #[test]
    fn dragging_across_checkboxes_sets_them_like_the_first() {
        let ctx = egui::Context::default();
        let mut state = [Some(false), Some(true), None, Some(false), Some(false)];
        let rects = frame(&ctx, &mut state, vec![]);
        let at = |i: usize| rects[i].left_center() + vec2(6.0, 0.0);
        frame(&ctx, &mut state, vec![Event::PointerMoved(at(0)), press(at(0), true)]);
        sweep(&ctx, &mut state, at(0), at(3));
        frame(&ctx, &mut state, vec![press(at(3), false)]);
        frame(&ctx, &mut state, vec![]);
        assert_eq!(state, [Some(true), Some(true), None, Some(true), Some(false)]);
        // After the release, moving over a box changes nothing.
        sweep(&ctx, &mut state, at(3), at(4));
        assert_eq!(state[4], Some(false));
        // A drag from a checked box clears the boxes it goes over.
        frame(&ctx, &mut state, vec![press(at(4), true)]);
        frame(&ctx, &mut state, vec![press(at(4), false)]);
        assert_eq!(state[4], Some(true), "a click toggles one box");
        frame(&ctx, &mut state, vec![Event::PointerMoved(at(4)), press(at(4), true)]);
        sweep(&ctx, &mut state, at(4), at(0));
        frame(&ctx, &mut state, vec![press(at(0), false)]);
        frame(&ctx, &mut state, vec![]);
        assert_eq!(state, [Some(false), Some(false), None, Some(false), Some(false)]);
    }
}
