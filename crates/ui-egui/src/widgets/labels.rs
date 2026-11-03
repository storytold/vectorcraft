use super::{Pos2, Rect, Response, Sense, Stroke, Tokens, Ui, icons, pos2, scrub, theme, vec2};

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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, text));
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

/// The `Id` under which [`dim_label`]/[`field_label`] leave the label a following field should be
/// named by. egui 0.37's kittest fails an input with no accessible name, and VectorCraft paints
/// most labels beside, not inside, their fields.
fn next_label_id() -> egui::Id {
    egui::Id::unique("vectorcraft-next-input-label")
}

/// The label a field may be named by, and the widget id the field must be drawn under for the
/// label to belong to it. egui allocates a fresh auto id for every widget, but not for moving
/// between grid cells: the next field in a grid still has the id the label recorded, while a
/// checkbox or another widget between the label and a field has taken it. A label therefore names
/// the very next field, also in a grid, but not a later one after an unrelated widget.
#[derive(Clone, Copy)]
struct NextLabel {
    id: egui::Id,
    viewport: egui::ViewportId,
    pass: u64,
    at: Pos2,
    next_auto_id: egui::Id,
}

/// Record a label's widget id, viewport, pass, position and next auto id so only the immediately
/// following field can take the label.
fn remember_next_label(ui: &Ui, id: egui::Id) {
    let label = NextLabel {
        id,
        viewport: ui.ctx().viewport_id(),
        pass: ui.ctx().cumulative_pass_nr(),
        at: ui.next_widget_position(),
        next_auto_id: ui.next_auto_id(),
    };
    ui.ctx().data_mut(|d| d.insert_temp(next_label_id(), label));
}

/// A label taken for the next field, and whether no widget came between the two.
pub(crate) struct PreviousLabel {
    id: egui::Id,
    at: Pos2,
    immediate: bool,
}

/// The label drawn immediately before a field in this pass, if no other widget came between them
/// (an app-local adapter around [`egui::Response::labelled_by`], not a general compatibility
/// layer). Takes the record, so one label names one field.
pub(crate) fn take_previous_label(ui: &Ui) -> Option<PreviousLabel> {
    let label = ui.ctx().data_mut(|d| {
        let label = d.get_temp::<NextLabel>(next_label_id());
        d.remove::<NextLabel>(next_label_id());
        label
    })?;
    // Frame and pass numbers are per viewport; a label left in one viewport must never name a
    // field in another. The shipped hosts draw one root viewport, so this scoping is defensive.
    (label.viewport == ui.ctx().viewport_id() && label.pass == ui.ctx().cumulative_pass_nr()).then(|| PreviousLabel {
        id: label.id,
        at: label.at,
        immediate: ui.next_auto_id() == label.next_auto_id,
    })
}

/// Name `resp` by the field's real adjacent label (see [`take_previous_label`]).
pub(crate) fn label_field(resp: Response, label: Option<PreviousLabel>) -> Response {
    let Some(label) = label else { return resp };
    let adjacent = label.immediate || resp.rect.min.distance(label.at) <= 16.0 || resp.rect.contains(label.at);
    if adjacent { resp.labelled_by(label.id) } else { resp }
}

/// Give `resp` the explicit semantic name of a field whose visible label is not a widget beside
/// it (a label below a grid of dash cells, a heading over a row of fields). The name is already
/// translated at the call site. An empty name leaves the field unnamed rather than inventing one.
pub(super) fn name_field(resp: Response, name: Option<&str>) -> Response {
    match name {
        Some(name) if !name.trim().is_empty() => resp.accessible_name(name),
        _ => resp,
    }
}

/// A label in the panels' text colour; before a numeric field it scrubs the field
/// ([`scrub`]).
pub fn dim_label(ui: &mut Ui, text: &str) -> Response {
    let resp = dim_name(ui, tl!(text));
    scrub::note_label(ui, resp.rect);
    remember_next_label(ui, resp.id);
    resp
}

/// A field's label in other styles (`text` as given): before a numeric field it scrubs the field
/// ([`scrub`]).
pub fn field_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    let resp = ui.label(text);
    scrub::note_label(ui, resp.rect);
    remember_next_label(ui, resp.id);
    resp
}

/// [`dim_label`] for a name that is user or file data (a swatch, style, artboard or font name):
/// shown as it is, never translated.
pub fn dim_name(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text).size(12.5))
}

/// A form row: `label` in a column `label_width` wide, then what `add` draws.
pub fn label_row(ui: &mut Ui, label: &str, label_width: f32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(label_width, 24.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        let shown = tl!(label);
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, shown, egui::FontId::proportional(12.5), t.text);
        scrub::note_label(ui, r);
        // The label is painted, not a widget of its own: give it a real semantic node so the
        // input beside it can be named by `label_inputs_by` (egui 0.37's strict a11y gate).
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), shown));
        if label.trim().is_empty() {
            add(ui);
        } else {
            ui.label_inputs_by(resp.id, add);
        }
    });
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(text)));
    resp
}
