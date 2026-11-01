use super::labels::{PreviousLabel, label_field, name_field, take_previous_label};
use super::numeric::mixed_field_named;
use super::panel_chrome::chevron_cell;
use super::{Rect, Response, Stroke, StrokeKind, Tokens, Ui, id_with, vec2};

/// The flag `dialogs::show` raises while it draws the body of a dialog that has just opened; the
/// first enabled field drawn takes it ([`take_dialog_focus`]).
pub(crate) fn dialog_focus_flag() -> egui::Id {
    egui::Id::unique("dialog-focus-first-field")
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

/// The key under which [`arm_field_focus`] keeps the armed field's id salt.
fn armed_field_flag() -> egui::Id {
    egui::Id::unique("armed-field-focus")
}

/// Arm the field drawn with id salt `salt`: a numeric field's `id`, or a searchable menu's (the
/// font family menu's). The next one drawn, enabled and outside a sizing pass, takes the keyboard
/// focus ([`take_armed_focus`]) until [`disarm_field_focus`].
pub(crate) fn arm_field_focus(ctx: &egui::Context, salt: impl std::hash::Hash) {
    ctx.data_mut(|d| d.insert_temp(armed_field_flag(), id_with(egui::Id::NULL, salt)));
}

/// Disarm the field [`arm_field_focus`] armed. → Whether it was still armed: no field took it.
pub(crate) fn disarm_field_focus(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| {
        let armed = d.get_temp::<egui::Id>(armed_field_flag()).is_some();
        d.remove::<egui::Id>(armed_field_flag());
        armed
    })
}

/// Whether the field drawn with id salt `salt` is the one armed and can take the keyboard focus
/// now (it is enabled, and this is not a sizing pass). It is then disarmed.
pub(crate) fn take_armed_focus(ui: &Ui, salt: impl std::hash::Hash) -> bool {
    if ui.is_sizing_pass() || !ui.is_enabled() {
        return false;
    }
    let salt = id_with(egui::Id::NULL, salt);
    ui.data_mut(|d| {
        let armed = d.get_temp::<egui::Id>(armed_field_flag()) == Some(salt);
        if armed {
            d.remove::<egui::Id>(armed_field_flag());
        }
        armed
    })
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

/// Select all of `text` in text field `id`.
pub(super) fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
    st.cursor.set_char_range(Some(all));
    st.store(ui.ctx(), id);
}

/// The flag that tells the fields a dialog is open in this frame ([`set_dialog_open`]).
fn dialog_open_flag() -> egui::Id {
    egui::Id::unique("dialog-open")
}

/// The flag that tells the dialog runner a field rejected an invalid commit on Enter in this frame.
pub(crate) fn dialog_reject_commit_flag() -> egui::Id {
    egui::Id::unique("dialog-reject-commit")
}

/// Suppress confirming an open dialog on Enter during this frame when an invalid value was committed.
pub(crate) fn reject_dialog_commit(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(dialog_reject_commit_flag(), true));
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
pub(super) fn shift_enter_keeps_focus(ui: &Ui, id: egui::Id) -> bool {
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
pub(super) fn after_shift_enter(ui: &Ui, id: egui::Id) -> bool {
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
pub(super) fn select_all_on_focus(ui: &Ui, resp: &Response, text: &str) {
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
pub(super) fn recessed_text(
    ui: &mut Ui,
    id: egui::Id,
    shown: &str,
    width: f32,
    rows: usize,
    label: Option<PreviousLabel>,
    name: Option<&str>,
) -> (String, bool, Response, Rect) {
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
            let now = ui.input(|i| i.time);
            let flash_end = ui.data(|d| d.get_temp::<f64>(id.with("invalid-flash")));
            let invalid = flash_end.is_some_and(|end| now <= end);
            if invalid {
                ui.ctx().request_repaint();
            }
            let stroke = if invalid {
                Stroke::new(1.5, t.accent_strong)
            } else if editing {
                Stroke::new(1.0, t.accent)
            } else {
                Stroke::new(1.0, t.input_border)
            };
            ui.painter().rect(rect, 5.0, t.input, stroke, StrokeKind::Inside);
            let margin = egui::Margin { left: 7, right: 7, top: if rows > 1 { 4 } else { 0 }, bottom: 0 };
            let resp = egui::Frame::NONE
                .inner_margin(margin)
                .show(ui, |ui| {
                    crate::theme::strong_selected_text(ui);
                    let edit = if rows > 1 {
                        egui::TextEdit::multiline(&mut buf).desired_rows(rows)
                    } else {
                        egui::TextEdit::singleline(&mut buf).align(egui::Align2::LEFT_CENTER)
                    };
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
    let resp = name_field(label_field(resp, label), name);
    let focused = ui.memory(|m| m.has_focus(id));
    ui.data_mut(|d| {
        d.insert_temp(id, buf.clone());
        d.insert_temp(focus_id, focused);
    });
    // Plain Enter only: Ctrl/Cmd+Enter keeps editing a field that takes it (X and Y in Transform).
    let enter_commit = rows == 1 && editing && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none());
    let commit = resp.lost_focus() || kept || enter_commit;
    (buf, commit, resp, rect)
}

/// A recessed text field showing `value` (blank when `None`: the selection's values differ),
/// `rows` lines tall (1: single line, which Enter and Shift+Enter commit). Returns the new text,
/// trimmed, when a change is committed (Enter, Shift+Enter or focus loss).
pub fn text_field(ui: &mut Ui, id: impl std::hash::Hash, value: Option<&str>, width: f32, rows: usize) -> Option<String> {
    text_field_named(ui, id, None, value, width, rows)
}

/// [`text_field`] for a row named explicitly instead of by an adjacent label widget.
pub fn text_field_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, value: Option<&str>, width: f32, rows: usize) -> Option<String> {
    let label = take_previous_label(ui);
    let id = id_with(ui.scope_id(), id);
    let shown = value.unwrap_or_default();
    let (buf, commit, _, _) = recessed_text(ui, id, shown, width, rows, label, name);
    (commit && buf.trim() != shown).then(|| buf.trim().to_string())
}

/// A one-line [`text_field`] that keeps the text exactly as typed, spaces included (a tab
/// leader such as ". "). Returns it when a change is committed (Enter, Shift+Enter or focus loss).
pub fn exact_text_field(ui: &mut Ui, id: impl std::hash::Hash, value: &str, width: f32) -> Option<String> {
    exact_text_field_named(ui, id, None, value, width)
}

/// [`exact_text_field`] for a row named explicitly instead of by an adjacent label widget.
pub fn exact_text_field_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, value: &str, width: f32) -> Option<String> {
    let label = take_previous_label(ui);
    let id = id_with(ui.scope_id(), id);
    let (buf, commit, _, _) = recessed_text(ui, id, value, width, 1, label, name);
    (commit && buf != value).then_some(buf)
}

/// A plain number field (percent, degrees, counts) with optional suffix.
pub fn plain_field(ui: &mut Ui, id: impl std::hash::Hash, value: f64, suffix: &str, decimals: usize, width: f32) -> Option<f64> {
    plain_field_named(ui, id, None, value, suffix, decimals, width)
}

/// [`plain_field`] for a row named explicitly instead of by an adjacent label widget.
pub fn plain_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    value: f64,
    suffix: &str,
    decimals: usize,
    width: f32,
) -> Option<f64> {
    mixed_field_named(ui, id, name, Some(value), suffix, decimals, width)
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
                    .align(egui::Align2::LEFT_CENTER)
                    .hint_text(egui::RichText::new(tl!(hint)).italics())
                    .desired_width(ui.available_width()),
            );
        });
    ui.data_mut(|d| d.insert_temp(id, query.clone()));
    query
}

/// A [`text_field`] with a preset list on its right (Export for Screens' sizes: `2x`, `100w`…).
/// Returns the new text when a change is committed (Enter or focus loss) or a preset is picked.
pub fn text_presets(ui: &mut Ui, id: impl std::hash::Hash, value: &str, presets: &[&str], width: f32) -> Option<String> {
    text_presets_named(ui, id, None, value, presets, width)
}

/// [`text_presets`] for a row whose heading names it instead of an adjacent label widget.
pub fn text_presets_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, value: &str, presets: &[&str], width: f32) -> Option<String> {
    let mut out = None;
    ui.scope(|ui| {
        // The field and the chevron share a border.
        ui.spacing_mut().item_spacing.x = -1.0;
        out = text_field_named(ui, &id, name, Some(value), width - 19.0, 1);
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
