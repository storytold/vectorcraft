use egui::IntoAtoms as _;

use super::drag::PanelDrag;
use super::labels::{PreviousLabel, label_field, name_field, take_previous_label};
use super::text::take_armed_focus;
use super::{CornerRadius, HashOnly, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, id_with, vec2};

/// A compact dropdown of interface labels (shown in the UI language). Returns the chosen index.
pub fn dropdown(ui: &mut Ui, id: impl std::hash::Hash, current: &str, options: &[&str], width: f32) -> Option<usize> {
    dropdown_with_named(ui, id, None, current, options, width, |_| true)
}

/// [`dropdown`] for a row whose visible label is not an adjacent label widget.
pub fn dropdown_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, current: &str, options: &[&str], width: f32) -> Option<usize> {
    dropdown_with_named(ui, id, name, current, options, width, |_| true)
}

/// [`dropdown`] of names that are user, file or system data (font styles, artboards, presets the
/// user saved, printers, profiles), shown as they are, never translated. For a list that mixes
/// built-in labels with such names, translate the built-in ones (`tl!`) before passing them.
pub fn dropdown_names(ui: &mut Ui, id: impl std::hash::Hash, current: &str, options: &[&str], width: f32) -> Option<usize> {
    dropdown_names_named(ui, id, None, current, options, width)
}

/// [`dropdown_names`] for a row whose visible label is not an adjacent label widget.
pub fn dropdown_names_named(ui: &mut Ui, id: impl std::hash::Hash, name: Option<&str>, current: &str, options: &[&str], width: f32) -> Option<usize> {
    combo_named(ui, id, name, current, width, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            if option_row(ui, true, *o == current, *o).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// [`dropdown_names`] whose options each show an icon before their name, in the open list and in
/// the closed field, so that a name that says little on its own is shown with a picture of what it
/// means (#6304). `icon(ui, rect, index, ink, highlighted)` paints option `index`'s icon into
/// `rect`, an `icon_size` square, with `ink`, the colour the row's name is drawn in;
/// `highlighted` is true on the open list's row of the current option, drawn on the selection
/// fill, where an accent-coloured mark would lose its contrast. `current` is the option shown;
/// `None` shows a blank field (characters that differ).
pub fn dropdown_icon_names(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    current: Option<usize>,
    options: &[&str],
    width: f32,
    icon_size: f32,
    icon: impl Fn(&Ui, Rect, usize, egui::Color32, bool),
) -> Option<usize> {
    dropdown_icon_names_sized(ui, id, current, options, width, Vec2::splat(icon_size), icon)
}

/// [`dropdown_icon_names`] with icons of any size, for a picture wider than it is tall, such as a
/// line with an arrowhead.
pub fn dropdown_icon_names_sized(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    current: Option<usize>,
    options: &[&str],
    width: f32,
    icon_size: Vec2,
    icon: impl Fn(&Ui, Rect, usize, egui::Color32, bool),
) -> Option<usize> {
    let icon = &icon;
    let atom = move |i: usize, highlighted: bool| egui::Atom::paint(icon_size, move |ui, a| icon(ui, a.rect, i, a.fallback_text_color, highlighted));
    let shown = match current.and_then(|i| options.get(i).map(|name| (i, *name))) {
        Some((i, name)) => (atom(i, false), egui::RichText::new(name).size(12.0)).into_atoms(),
        None => egui::RichText::new("").size(12.0).into_atoms(),
    };
    combo_atoms_named(ui, id, None, shown, width, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            let on = current == Some(i);
            if option_row(ui, true, on, (atom(i, on), *o)).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// [`dropdown`] whose options can be disabled (`enabled(index)`: greyed and not choosable).
pub fn dropdown_with(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    current: &str,
    options: &[&str],
    width: f32,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    dropdown_with_named(ui, id, None, current, options, width, enabled)
}

/// [`dropdown_with`] for a row whose visible label is not an adjacent label widget.
#[allow(clippy::too_many_arguments)]
pub fn dropdown_with_named(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    current: &str,
    options: &[&str],
    width: f32,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    combo_named(ui, id, name, tl!(current), width, |ui| {
        let mut chosen = None;
        for (i, o) in options.iter().enumerate() {
            if option_row(ui, enabled(i), *o == current, tl!(o)).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// What [`option_row`] found in a list that [`combo_atoms_named`] draws on the pass the list opens
/// and the pass after it.
#[derive(Clone, Copy, Default)]
struct Opening {
    /// The list draws its options with [`option_row`].
    rows: bool,
    /// A current option was scrolled into view.
    scrolled: bool,
}

thread_local! {
    /// The [`Opening`] that [`combo_atoms_named`] keeps while it draws a list on the pass the list
    /// opens and the pass after it; `None` otherwise. A row reads it without a lookup in the
    /// context's data.
    static OPENING: std::cell::Cell<Option<Opening>> = const { std::cell::Cell::new(None) };
}

/// One option of a dropdown list, `selected` for the current one. On the pass the list opens and
/// the next one, the first current option scrolls to the middle of the list, or as near to it as
/// the list's ends allow, so that it shows whole (#5941).
fn option_row<'a>(ui: &mut Ui, enabled: bool, selected: bool, atoms: impl egui::IntoAtoms<'a>) -> Response {
    let response = ui.add_enabled(enabled, egui::Button::selectable(selected, atoms));
    if let Some(opening) = OPENING.get() {
        let scroll = selected && !opening.scrolled;
        if scroll {
            ui.scroll_to_rect_animation(response.rect, Some(egui::Align::Center), egui::style::ScrollAnimation::none());
        }
        OPENING.set(Some(Opening { rows: true, scrolled: opening.scrolled || scroll }));
    }
    response
}

/// The id of the button of [`combo_atoms_named`] in this `Ui`, the one
/// [`craft_ui::ComboBox::from_id_salt`] gives its button: `from_id_salt` stores
/// `IdSalt::new(salt)` and hashes it once more with the scope. The app reads the button's focus and
/// open state with it before the button is drawn.
pub(super) fn combo_button_id(ui: &Ui, id: &impl std::hash::Hash) -> egui::Id {
    ui.make_persistent_id(egui::IdSalt::new(HashOnly(id)))
}

/// The recessed combo box of [`dropdown`] showing `current`; `list` draws the options and returns
/// the chosen one.
pub(crate) fn combo<R>(ui: &mut Ui, id: impl std::hash::Hash, current: &str, width: f32, list: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    combo_named(ui, id, None, current, width, list)
}

/// [`combo`] for a row whose visible label is not an adjacent label widget.
pub(crate) fn combo_named<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    current: &str,
    width: f32,
    list: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R> {
    combo_atoms_named(ui, id, name, egui::RichText::new(current).size(12.0), width, list)
}

/// [`combo_named`] whose closed field shows the atoms `current` (an icon and a name, say).
fn combo_atoms_named<'a, R>(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    current: impl egui::IntoAtoms<'a>,
    width: f32,
    list: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R> {
    let t = Tokens::get(ui.ctx());
    let label = take_previous_label(ui);
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(CornerRadius::same(3))
        .show(ui, |ui| {
            let button_id = combo_button_id(ui, &id);
            // A list whose options are `option_row`s opens with its current option in view, or at
            // its first option, not where it was scrolled when it last closed (#5941). A list that
            // draws its own rows has no current option to show, and opens where it was scrolled
            // when it last closed.
            let shown_key = button_id.with("list-shown-pass");
            let opened_key = button_id.with("list-opened-pass");
            let offset_key = button_id.with("list-offset");
            let list = move |ui: &mut Ui| {
                let pass = ui.ctx().cumulative_pass_nr();
                let last = ui.data_mut(|d| d.get_temp::<u64>(shown_key));
                ui.data_mut(|d| d.insert_temp(shown_key, pass));
                let opening = last.is_none_or(|p| p + 1 < pass);
                if opening {
                    ui.data_mut(|d| d.insert_temp(opened_key, pass));
                }
                // The list is laid out unseen to measure it on the pass it opens and placed on the
                // next one, so the current option is scrolled into view on both.
                let settling = ui.data(|d| d.get_temp::<u64>(opened_key)).is_some_and(|p| pass <= p + 1);
                // A list drawn inside this one's rows finds its own record, and this one's comes back after it.
                let outer = if settling { OPENING.replace(Some(Opening::default())) } else { OPENING.take() };
                let (content_left, content_top) = (ui.max_rect().left(), ui.max_rect().top());
                let at = move |y: f32| Rect::from_min_size(egui::pos2(content_left, content_top + y), Vec2::ZERO);
                let chosen = list(ui);
                let found = OPENING.replace(outer).unwrap_or_default();
                let none = egui::style::ScrollAnimation::none();
                if settling {
                    if found.rows {
                        if opening && !found.scrolled {
                            ui.scroll_to_rect_animation(at(0.0), Some(egui::Align::TOP), none);
                        }
                    } else {
                        // The list is taller on the pass it opens than beside its field, and egui
                        // clamps the scroll to the taller list, so the scroll it had when it closed
                        // is put back on both passes. A scroll area leaves an item spacing above a
                        // target aligned to its top, so the target is that much below the scroll.
                        let saved = ui.data(|d| d.get_temp::<f32>(offset_key)).unwrap_or(0.0);
                        ui.scroll_to_rect_animation(at(saved + ui.spacing().item_spacing.y), Some(egui::Align::TOP), none);
                    }
                }
                if !settling || chosen.is_some() {
                    // The scroll while the list is open: the content's top is above the clip's by it.
                    ui.data_mut(|d| d.insert_temp(offset_key, (ui.clip_rect().top() - content_top).max(0.0)));
                }
                chosen
            };
            let enter_for_combo = ui.memory(|m| m.has_focus(button_id)) || craft_ui::ComboBox::is_open(ui.ctx(), button_id);
            let response = combo_button(ui, button_id, current, width - 4.0);
            let shown = combo_list(ui, &response, list);
            // The button or open list handles Enter before the modal can accept it.
            if enter_for_combo {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            }
            let _ = name_field(label_field(response, label), name);
            let chosen = shown.flatten();
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

/// The button of [`combo_atoms_named`] with the id `button_id`, at least `width` wide, showing
/// `current`: [`craft_ui::ComboBox`]'s button, with its look, its tooltip for a value cut short and
/// its role for screen readers. It is drawn here so that its list opens in [`combo_list`].
fn combo_button<'a>(ui: &mut Ui, button_id: egui::Id, current: impl egui::IntoAtoms<'a>, width: f32) -> Response {
    use egui::class::HasClasses as _;
    use egui::{AtomExt as _, NumExt as _};
    let open = egui::Popup::is_id_open(ui.ctx(), button_id.with("popup"));
    let mut classes = egui::class::Classes::default();
    classes.add_class(craft_ui::ComboBox::CLASS);
    classes.set_class(craft_ui::ComboBox::CLASS_OPEN, open);
    let egui::widget_style::ButtonStyle { atom_layout } = ui.widget_style(button_id, &classes);
    let visuals = if open {
        ui.visuals().widgets.open
    } else if let Some(response) = ui.ctx().read_response(button_id) {
        *ui.style().interact(&response)
    } else {
        ui.visuals().widgets.inactive
    };
    let min_size = vec2(width, 0.0).at_least(atom_layout.min_size);
    let current = current.into_atoms();
    let text = current.text().map(String::from).unwrap_or_default();
    let rtl = ui.style().right_to_left;
    // The combo box's arrow: a downward triangle.
    let arrow = egui::Atom::paint(Vec2::splat(ui.spacing().icon_width), move |ui, args| {
        let rect = args.rect.expand(visuals.expansion);
        let rect = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.7, rect.height() * 0.45));
        ui.painter().add(egui::Shape::rotated_triangle(rect, 0.0, visuals.fg_stroke.color));
    })
    .atom_grow(true)
    .atom_align(if rtl { egui::Align2::LEFT_CENTER } else { egui::Align2::RIGHT_CENTER });
    let button = atom_layout
        .apply(
            egui::WidgetAtom::new((current, arrow))
                .id(button_id)
                .sense(Sense::click())
                .fallback_font(egui::TextStyle::Button)
                .wrap_mode(ui.wrap_mode())
                .direction(if rtl { egui::Direction::RightToLeft } else { egui::Direction::LeftToRight }),
        )
        .min_size(min_size)
        .measure(ui, ui.available_size());
    let elided = button.container.iter_texts().any(|galley| galley.elided);
    let response = button.allocate(ui).paint(ui).response;
    // A value cut short shows in full on hover, as a truncated `Label`'s does.
    let response = if elided {
        response.on_hover_ui(|ui| {
            ui.set_max_width(ui.spacing().tooltip_width);
            ui.add(egui::Label::new(text.as_str()).wrap());
        })
    } else {
        response
    };
    response.widget_info(|| egui::WidgetInfo {
        enabled: ui.is_enabled(),
        current_text_value: Some(text.clone()),
        ..egui::WidgetInfo::new(egui::Role::ComboBox)
    });
    response
}

/// The list of [`combo_atoms_named`] while it is open, drawn by `list` in a popup of its own that
/// opens from `button` and takes the whole list whenever the window has room for it (#5941): below
/// the button when it fits there, above it when it fits there instead, and otherwise moved up from
/// below, over the button, until it ends at the bottom bars. It stays above the bottom bars
/// ([`menu_floor`]), and a list taller than the room above them scrolls. A click closes
/// the list, except a click on its scroll bar, which scrolls it. Returns what `list` returned, or
/// `None` while the list is closed.
fn combo_list<R>(ui: &Ui, button: &Response, list: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let ctx = ui.ctx();
    let id = button.id.with("popup");
    let style = egui::style::StyleModifier::new(|style| {
        egui::containers::menu::menu_style(style);
        style.spacing.scroll = list_scroll_style();
    });
    // The id of the list's scroll area. egui senses its vertical bar under the area's id with the
    // axis, 1, and names the widget a click went to at the start of the pass.
    let area_key = id.with("scroll-area-id");
    let on_bar =
        ctx.data(|d| d.get_temp::<egui::Id>(area_key)).is_some_and(|area| ctx.interaction_snapshot(|s| s.clicked == Some(area.with(1_usize))));
    let close = if on_bar { egui::PopupCloseBehavior::IgnoreClicks } else { egui::PopupCloseBehavior::CloseOnClick };
    let popup = egui::Popup::menu(button).id(id).width(button.rect.width()).style(style.clone()).close_behavior(close);
    let screen = ctx.content_rect();
    // The list stops above the bottom bars, as other menus do (#2086).
    let floor = menu_floor(ctx).min(screen.bottom());
    // The room on each side of the rect the popup opens from: the part of the button in view.
    let anchor = popup.get_anchor_rect().unwrap_or(button.rect);
    let below = floor - anchor.bottom();
    let above = anchor.top() - screen.top();
    // The list's size from its last pass: egui lays a popup out unseen on the pass it opens, so the
    // list is measured before it is placed. A right-to-left layout lines the list up with the
    // button's right edge, as egui's menus do.
    let rtl = ui.style().right_to_left;
    let expected = popup.get_expected_size();
    let (align, popup) = match expected {
        Some(size) if size.y > below && size.y <= above => (if rtl { egui::RectAlign::TOP_END } else { egui::RectAlign::TOP_START }, popup),
        // A list too tall for either side moves up from below until it ends at the floor.
        Some(size) if size.y > below => {
            let top = (floor - size.y).max(screen.top());
            let from = Rect::from_min_max(egui::pos2(anchor.left(), top), egui::pos2(anchor.right(), top));
            (if rtl { egui::RectAlign::BOTTOM_END } else { egui::RectAlign::BOTTOM_START }, popup.anchor(from))
        }
        _ => (if rtl { egui::RectAlign::BOTTOM_END } else { egui::RectAlign::BOTTOM_START }, popup),
    };
    let mut popup_style = (*ctx.global_style()).clone();
    style.apply(&mut popup_style);
    let margins = egui::Frame::popup(&popup_style).total_margin().sum();
    let height = (floor - screen.top() - margins.y).max(ui.spacing().interact_size.y);
    // A list that was wider than the window keeps its scroll bar in view: until it closes, it is no
    // wider than it measured or than the window, its names cut short and shown in full on hover.
    // Only a size measured while the list is open counts: egui keeps the last one after it closes.
    // The width it needs is its rows' width on the last pass before it was narrowed, with its
    // margins, and the bar's column while the rows scroll.
    let too_wide_key = id.with("too-wide");
    let natural_key = id.with("natural-width");
    let scrolls_key = id.with("scrolls");
    let open_last_pass = ctx.read_response(id).is_some();
    if popup.get_expected_size().is_some_and(|size| open_last_pass && size.x > screen.width()) {
        // The first measurement is kept: later ones are of the list already narrowed.
        ctx.data_mut(|d| {
            if d.get_temp::<f32>(too_wide_key).is_none() {
                let rows = d.get_temp::<f32>(natural_key).unwrap_or(screen.width());
                d.insert_temp(too_wide_key, rows);
            }
        });
    }
    let (measured, scrolls) = ctx.data(|d| (d.get_temp::<f32>(too_wide_key), d.get_temp::<bool>(scrolls_key).unwrap_or(false)));
    let too_wide = measured.is_some();
    let shown = popup.align(align).align_alternatives(&[]).show(|ui| {
        if let Some(rows) = measured {
            let bar = if scrolls { list_scroll_style().allocated_width() } else { 0.0 };
            ui.set_max_width((rows + bar).min(screen.width() - margins.x).max(0.0));
        }
        ui.set_min_width(ui.available_width());
        // A popup offers its contents the height it had on its last pass; the list is offered the
        // window's instead.
        ui.set_max_height(height);
        // Labels widen a narrow list rather than wrap, as in egui's combo box.
        ui.style_mut().wrap_mode = Some(if too_wide { egui::TextWrapMode::Truncate } else { egui::TextWrapMode::Extend });
        let painted = ui.ctx().graphics(|g| g.get(ui.layer_id()).map_or(0, |shapes| shapes.all_entries().len()));
        let shown = egui::ScrollArea::vertical().max_height(height).show(ui, list);
        if too_wide {
            show_shortened_rows(ui, painted);
        }
        ui.data_mut(|d| {
            if !too_wide {
                d.insert_temp(natural_key, shown.content_size.x);
            }
            d.insert_temp(scrolls_key, shown.content_size.y > shown.inner_rect.height());
            d.insert_temp(area_key, shown.id);
        });
        shown.inner
    });
    if shown.is_some() {
        ctx.accesskit_node_builder(id, |node| node.set_role(egui::Role::ListBox));
    } else {
        ctx.data_mut(|d| {
            d.remove::<f32>(too_wide_key);
            d.remove::<f32>(natural_key);
            d.remove::<bool>(scrolls_key);
        });
    }
    shown.map(|r| r.inner)
}

/// The shapes painted so far in this pass on the topmost popup that was shown in the last one, such
/// as an open dropdown list, without what lies under it. Read at the end of a test's UI.
#[cfg(test)]
pub(crate) fn top_popup_shapes(ctx: &egui::Context) -> Vec<egui::epaint::ClippedShape> {
    let top = ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last());
    top.map_or_else(Vec::new, |layer| ctx.graphics(|g| g.get(layer).map(|p| p.all_entries().cloned().collect()).unwrap_or_default()))
}

/// The recessed combo box of [`dropdown`] showing `current`, whose menu keeps a search field and
/// filters in place over a list that scrolls (the font menu and the shortcut import's target
/// picker, #555). `menu` shows the menu with one of [`craft_ui::combo::Menu`]'s header forms and
/// returns what it returned (`None` while the menu is closed). A click inside the menu keeps it
/// open: it closes itself when an option is chosen. A click outside closes it. Armed with id salt
/// `id` ([`arm_field_focus`]), the menu opens and no widget keeps the keyboard focus, so the
/// menu's search field can take it.
pub(crate) fn combo_menu<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    current: &str,
    width: f32,
    menu: impl FnOnce(craft_ui::combo::Menu<'_>) -> Option<R>,
) -> Option<R> {
    combo_menu_field(ui, id, current, width, menu).1
}

/// [`combo_menu`], also returning the closed field's response (the font family field reads the
/// mouse wheel turned over it).
pub(crate) fn combo_menu_field<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    current: &str,
    width: f32,
    menu: impl FnOnce(craft_ui::combo::Menu<'_>) -> Option<R>,
) -> (Response, Option<R>) {
    combo_menu_field_named(ui, id, None, current, width, menu)
}

/// [`combo_menu_field`] for a field whose visible label is not an adjacent label widget.
pub(crate) fn combo_menu_field_named<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    name: Option<&str>,
    current: &str,
    width: f32,
    menu: impl FnOnce(craft_ui::combo::Menu<'_>) -> Option<R>,
) -> (Response, Option<R>) {
    let t = Tokens::get(ui.ctx());
    let label = take_previous_label(ui);
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(CornerRadius::same(3))
        .show(ui, |ui| {
            let armed = take_armed_focus(ui, &id);
            let resp = combo_field(ui, id_with(ui.scope_id(), id), label, name, current, width - 4.0);
            if armed {
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
                ui.memory_mut(|m| m.stop_text_input());
            }
            let shown = menu(craft_ui::combo::Menu::new(ui, &resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside));
            (resp, shown)
        })
        .inner
}

/// The field of a searchable combo box, drawn as egui's combo box button, with the id `id`: its
/// menu is the popup `id.with("popup")`.
fn combo_field(ui: &mut Ui, id: egui::Id, label: Option<PreviousLabel>, name: Option<&str>, current: &str, width: f32) -> Response {
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("popup"));
    let pad = ui.spacing().button_padding;
    let icon = Vec2::splat(ui.spacing().icon_width);
    let galley = egui::WidgetText::from(egui::RichText::new(current).size(12.0)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width - 2.0 * pad.x - ui.spacing().icon_spacing - icon.x,
        egui::TextStyle::Button,
    );
    let height = (galley.size().y.max(icon.y) + 2.0 * pad.y).max(ui.spacing().interact_size.y);
    let size = vec2(width, height);
    let (_, space) = ui.allocate_space(size);
    let rect = egui::Align2::CENTER_CENTER.align_size_within_rect(size, space);
    let resp = ui.interact(rect, id, Sense::click());
    // Screen readers get what egui's combo box gives them: its role and the text it shows.
    resp.widget_info(|| egui::WidgetInfo {
        enabled: ui.is_enabled(),
        current_text_value: Some(current.to_owned()),
        ..egui::WidgetInfo::new(egui::Role::ComboBox)
    });
    let resp = name_field(label_field(resp, label), name);
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
    resp
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
    let was_open = popover_is_open(ctx, resp.id);
    let mut open = if toggle {
        !was_open
    } else {
        // Popups (this one and those it opened) are on foreground layers; a click anywhere
        // else closes it. While the pointer is down or a panel paint drag is active, keep the
        // popover open so a Fill/Stroke proxy drag can finish (#3440).
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        let panel_drag = egui::DragAndDrop::payload::<PanelDrag>(ctx).is_some();
        let click_pos = ctx.input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()).flatten());
        let clicked_anchor = click_pos.is_some_and(|pos| resp.rect.contains(pos));
        let clicked_elsewhere = !pointer_down
            && !panel_drag
            && !clicked_anchor
            && click_pos.is_some_and(|pos| ctx.layer_id_at(pos).is_none_or(|l| l.order != egui::Order::Foreground));
        was_open && !clicked_elsewhere
    };
    // Closing with a field in it that has the keyboard: it draws once more, so the field lets go of
    // the keyboard to the click and commits what was typed, as in a panel (#6429).
    let last = was_open && !open && field_focused_in(ctx, egui::LayerId::new(egui::Order::Foreground, id));
    let mut shown = open || last;
    let inner =
        egui::Popup::menu(resp).id(id).open_bool(&mut shown).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(content).map(|r| r.inner);
    open = shown && !last;
    ctx.data_mut(|d| {
        if open {
            d.insert_temp(id, frame);
        } else {
            d.remove::<u64>(id);
        }
    });
    inner
}

/// Whether a field in the popup on `layer` has the keyboard. A popup closing with one draws once
/// more before it goes: the field lets go of the keyboard to the click away and commits what was
/// typed, as it does in a panel. A popup that closed without drawing again dropped it (#6429).
pub(crate) fn field_focused_in(ctx: &egui::Context, layer: egui::LayerId) -> bool {
    ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id)).is_some_and(|r| r.layer_id == layer)
}

/// Whether the popover for this anchor was open in the latest completed frame.
pub(crate) fn popover_is_open(ctx: &egui::Context, anchor: egui::Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<u64>(anchor.with("popover"))).is_some_and(|f| f + 1 >= frame)
}

/// Records where the bottom bars (the status and hint bars, and the Control bar docked at the
/// bottom) start this frame in the window being drawn, so its menus can stop above them (#2086).
/// Without bars it is the bottom of the window.
pub fn set_menu_floor(ctx: &egui::Context, y: f32) {
    let id = menu_floor_id(ctx);
    ctx.data_mut(|d| d.insert_temp(id, y));
}

/// The floor's key: egui viewports share one context, and each OS window has its own bottom edge.
fn menu_floor_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::unique("menu_floor").with(ctx.viewport_id())
}

/// Where menus of the window being drawn stop: its recorded floor, or its bottom edge when nothing
/// recorded one, as in a floating panel's OS window.
pub(crate) fn menu_floor(ctx: &egui::Context) -> f32 {
    let id = menu_floor_id(ctx);
    ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or_else(|| ctx.content_rect().bottom())
}

/// The space a menu list leaves between its end and the window's edge or bars.
const MENU_GAP: f32 = 12.0;
/// The least height a menu list is given.
const MENU_MIN_HEIGHT: f32 = 120.0;

/// The height a menu list may take: from `top` down to `floor` less a small gap, at least 120 pt,
/// and never more than `tallest`, the most that fits in the window.
pub fn menu_room(floor: f32, top: f32, tallest: f32) -> f32 {
    (floor - top - MENU_GAP).max(MENU_MIN_HEIGHT).min(tallest)
}

/// The height a menu list may take when its popup can open below or above the rect it opens from:
/// `below` and `above` are the heights each side leaves for the list. The list goes below while that
/// side has room for the minimum height, otherwise on the side with more room. It gets at least
/// 120 pt and never more than `tallest`.
pub(crate) fn menu_room_either(below: f32, above: f32, tallest: f32) -> f32 {
    let (below, above) = (below - MENU_GAP, above - MENU_GAP);
    let room = if below >= MENU_MIN_HEIGHT { below } else { below.max(above) };
    room.max(MENU_MIN_HEIGHT).min(tallest)
}

/// The scroll bar of a menu's list: slim, in a column of its own beside the items, so it never
/// covers what sits at their right end (shortcuts, submenu arrows, a font's favourite star).
/// `theme::apply` makes it the style of every scroll area that does not set its own, so the bar
/// shows whenever there is more to scroll to (#5941).
pub fn list_scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle { bar_width: 6.0, bar_inner_margin: 2.0, ..egui::style::ScrollStyle::solid() }
}

/// The widget that opened a popup and whether it is a submenu's row (`None` when none was found),
/// the frame the popup was opened in, and the frame it was last drawn in.
type Opener = (Option<(egui::Id, bool)>, u64, u64);

/// The frames after a popup opens in which a search that found no opener runs again.
const OPENER_RETRY_FRAMES: u64 = 3;

/// The widget that opened `popup`: the button [`menu_opener`] recorded for it, a button whose popup
/// has egui's default id, or a submenu's row, found among the widgets of this frame and the previous
/// one. Once found, later frames of the same opening reuse it; a search that finds nothing runs
/// again for the first few frames.
fn popup_opener(ctx: &egui::Context, popup: egui::Id) -> Option<(egui::Id, bool)> {
    if let Some(button) = ctx.data(|d| d.get_temp::<egui::Id>(own_opener_key(popup))) {
        return Some((button, false));
    }
    let key = popup.with("menu-scroll-opener");
    let frame = ctx.cumulative_frame_nr();
    let cached = ctx.data(|d| d.get_temp::<Opener>(key)).filter(|&(_, _, seen)| seen + 1 >= frame);
    let opened = cached.map_or(frame, |(_, opened, _)| opened);
    let opener = match cached {
        Some((found, _, _)) if found.is_some() || frame >= opened + OPENER_RETRY_FRAMES => found,
        _ => {
            let found = ctx.viewport(|vp| {
                [&vp.this_pass, &vp.prev_pass].into_iter().flat_map(|pass| pass.widgets.layers()).flat_map(|(_, widgets)| widgets.iter()).find_map(
                    |w| {
                        let submenu = egui::containers::menu::SubMenu::id_from_widget_id(w.id) == popup;
                        (submenu || w.id.with("popup") == popup).then_some((w.id, submenu))
                    },
                )
            });
            found.filter(|&(widget, submenu)| {
                submenu || ctx.read_response(widget).is_some_and(|response| egui::Popup::default_response_id(&response) == popup)
            })
        }
    };
    ctx.data_mut(|d| d.insert_temp::<Opener>(key, (opener, opened, frame)));
    opener
}

/// Where the popup or menu that `ui` is drawn in opens from: the rect of the button or row that
/// opened it, or the point a context menu was opened at. The flag is true for a submenu, which
/// opens beside its row: down from the row's top, or up from its bottom. While the opening widget
/// is not drawn, the rect it last had is used. A popup with an id of its own (`Popup::id`) has no
/// anchor unless [`menu_opener`] recorded its button.
fn popup_anchor(ui: &Ui) -> Option<(Rect, bool)> {
    let ctx = ui.ctx();
    let popup = ui.layer_id().id;
    let root = ui.stack().iter().last()?;
    if !matches!(root.kind(), Some(egui::UiKind::Menu | egui::UiKind::Popup)) {
        return None;
    }
    if let Some(at) = egui::Popup::position_of_id(ctx, popup) {
        return Some((Rect::from_pos(at), false));
    }
    let (widget, submenu) = popup_opener(ctx, popup)?;
    let key = popup.with("menu-scroll-anchor");
    let drawn = ctx.read_response(widget).and_then(|response| egui::PopupAnchor::from(&response).rect(popup, ctx));
    let rect = match drawn {
        Some(rect) => {
            ctx.data_mut(|d| d.insert_temp(key, rect));
            rect
        }
        None => ctx.data(|d| d.get_temp::<Rect>(key))?,
    };
    // A submenu opens from its row grown by half the menu's margin, as egui's `SubMenu` places it.
    let grow = if submenu { egui::Frame::menu(ui.style()).total_margin().sum().y * 0.5 } else { 0.0 };
    Some((rect.expand2(vec2(0.0, grow)), submenu))
}

/// Records `button` as the widget that opens the menu `popup`, a popup with an id of its own
/// (`Popup::id`), so that [`menu_scroll`] measures the menu's room from the button, as it does for a
/// popup with egui's default id. Call it before the popup is shown. The record is kept for later
/// openings of `popup` and is replaced when another button is recorded for it.
pub fn menu_opener(button: &Response, popup: egui::Id) {
    button.ctx.data_mut(|d| d.insert_temp(own_opener_key(popup), button.id));
}

/// Where [`menu_opener`] records the button of a popup with an id of its own.
fn own_opener_key(popup: egui::Id) -> egui::Id {
    popup.with("menu-scroll-own-opener")
}

/// The body of a menu or popup list: as tall as its items up to the bottom bars of the window it
/// opens in, and scrolling only past that, so long menus (Window, Effect, a panel's menu) stay
/// reachable on small windows and in a floating panel's short OS window. Its scroll bar has a column
/// of its own, so it never covers the shortcuts or the submenu arrows (#2086, #5347). When the list
/// scrolls, that column widens the popup instead of narrowing the items, so their labels stay on one
/// line. In a window too narrow for the items, the list is as wide as the window allows, and a row
/// that does not fit ends in an ellipsis and shows its full text in a tooltip. Screen readers and
/// keyboard selection still use the full text.
///
/// The list's height comes from the room below and above the button, row or point the popup opens
/// from. A popup with an id of its own measures from its own top instead, unless [`menu_opener`]
/// recorded the button it opens from.
pub fn menu_scroll<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let margins = egui::Frame::popup(ui.style()).total_margin().sum();
    let tallest = (ctx.content_rect().height() - margins.y).max(0.0);
    let top = ui.next_widget_position().y;
    // The rows above the list (the popup's margin, a search field) move with the popup.
    let popup_top = ui.stack().iter().last().map_or(top, |root| root.max_rect.top());
    let ahead = top - popup_top;
    let (floor, window_top) = (menu_floor(&ctx), ctx.content_rect().top());
    let room = if let Some((anchor, submenu)) = popup_anchor(ui) {
        let (down_from, up_to) = if submenu { (anchor.top(), anchor.bottom()) } else { (anchor.bottom(), anchor.top()) };
        menu_room_either(floor - down_from - ahead, up_to - window_top - ahead, tallest)
    } else {
        menu_room(floor, top, tallest)
    };
    // The items' width on the first frame of this showing, when the scroll area does not yet show
    // its bar. Later frames lay the items out that wide, up to the widest the window allows, and
    // the bar's column is added to the popup's width. The width is kept until the popup closes.
    let key = ui.next_auto_id().with("menu-scroll-width");
    let frame = ctx.cumulative_frame_nr();
    let first = ctx.data(|d| d.get_temp::<(u64, f32)>(key)).filter(|(shown, _)| shown + 1 >= frame).map(|(_, width)| width);
    let scroll = list_scroll_style();
    let widest = menu_list_widest(ui);
    ui.scope(|ui| {
        ui.spacing_mut().scroll = scroll;
        // The minimum lets the list grow past a popup's default 400 pt size; it still shrinks to
        // its items when they need less.
        let area = egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room);
        let out = area.show(ui, |ui| {
            // Always the same child, so the items' ids stay the same on the frame the width changes.
            let max = ui.max_rect();
            let width = first.map_or(max.width(), |first| first.max(max.width()).min(widest));
            let rect = egui::Rect::from_min_size(max.min, egui::vec2(width, max.height()));
            let painted = ctx.graphics(|g| g.get(ui.layer_id()).map_or(0, |list| list.all_entries().len()));
            let inner = ui
                .scope_builder(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()), |ui| {
                    // The first frame measures the items at their full width.
                    if first.is_some() {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    }
                    add_contents(ui)
                })
                .inner;
            show_shortened_rows(ui, painted);
            inner
        });
        let width = first.unwrap_or(out.content_size.x);
        ctx.data_mut(|d| d.insert_temp(key, (frame, width)));
        out.inner
    })
    .inner
}

/// The widest the items of a [`menu_scroll`] list can be with the popup's margins and the scroll
/// bar's column inside the window.
fn menu_list_widest(ui: &Ui) -> f32 {
    let margins = egui::Frame::popup(ui.style()).total_margin().sum();
    (ui.ctx().content_rect().width() - margins.x - list_scroll_style().allocated_width()).max(0.0)
}

/// Makes a menu at least `width` wide, or as wide as the window allows the items of a
/// [`menu_scroll`] list when that is less, so the menu stays inside a narrow window.
pub(crate) fn menu_min_width(ui: &mut Ui, width: f32) {
    ui.set_min_width(width.min(menu_list_widest(ui)));
}

/// For each text that `ui`'s layer drew after its first `painted` shapes and that ends in an
/// ellipsis, the clickable row it is drawn in shows the full text in a tooltip that wraps to fit
/// inside the window.
fn show_shortened_rows(ui: &Ui, painted: usize) {
    let ctx = ui.ctx();
    let layer = ui.layer_id();
    let shortened: Vec<(egui::Pos2, egui::text::LayoutJob)> = ctx.graphics(|g| {
        let Some(list) = g.get(layer) else { return vec![] };
        list.all_entries()
            .skip(painted)
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.elided => {
                    let job = egui::text::LayoutJob {
                        sections: text.galley.job.sections.clone(),
                        text: text.galley.job.text.clone(),
                        language: text.galley.job.language.clone(),
                        ..Default::default()
                    };
                    Some((Rect::from_min_size(text.pos, text.galley.size()).center(), job))
                }
                _ => None,
            })
            .collect()
    });
    if shortened.is_empty() {
        return;
    }
    let rows: Vec<(egui::Id, Rect)> =
        ctx.viewport(|vp| vp.this_pass.widgets.get_layer(layer).filter(|w| w.sense.senses_click()).map(|w| (w.id, w.rect)).collect());
    for (at, job) in shortened {
        let Some(response) = rows.iter().rfind(|(_, rect)| rect.contains(at)).and_then(|(id, _)| ctx.read_response(*id)) else { continue };
        let tip = |ui: &mut Ui| {
            let margins = ui.spacing().menu_margin.sum().x + 2.0 * ui.spacing().item_spacing.x;
            ui.set_max_width(ui.spacing().tooltip_width.min(ui.ctx().content_rect().width() - margins).max(0.0));
            ui.add(egui::Label::new(job).wrap());
        };
        // A dimmed row shows its full text too.
        if response.enabled() {
            response.on_hover_ui(tip);
        } else {
            response.on_disabled_hover_ui(tip);
        }
    }
}

/// A column of tool or panel buttons (the toolbar, the dock's icon column) that scrolls when the
/// window is too short for it: with the mouse wheel, or with its scroll bar.
/// Its scroll bar stays a slim line at the edge, even while scrolled or dragged, so it never covers
/// the buttons (#808). The line shows while the pointer is elsewhere too (#5941).
pub fn strip_scroll<R>(ui: &mut Ui, id: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let floating = egui::style::ScrollStyle::floating();
        ui.spacing_mut().scroll = egui::style::ScrollStyle {
            bar_width: 4.0,
            floating_width: 2.0,
            bar_inner_margin: 0.0,
            bar_outer_margin: 1.0,
            dormant_handle_opacity: floating.active_handle_opacity,
            ..floating
        };
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, true]).show(ui, add_contents).inner
    })
    .inner
}

/// A panel's scrolling body (Properties, a dock column, a panel in the shared dock): `area` with
/// the scroll bar of a menu's list ([`list_scroll_style`]), in a column of its own at the right
/// edge, so it never covers the controls there, such as a dropdown's arrow (#3093). The scroll
/// areas inside the body keep their own style.
pub fn panel_scroll<R>(ui: &mut Ui, area: egui::ScrollArea, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let inner = std::mem::replace(&mut ui.spacing_mut().scroll, list_scroll_style());
        area.show(ui, |ui| {
            ui.spacing_mut().scroll = inner;
            add_contents(ui)
        })
        .inner
    })
    .inner
}
