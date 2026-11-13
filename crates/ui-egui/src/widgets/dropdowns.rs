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
            if ui.add_enabled(enabled(i), egui::Button::selectable(*o == current, tl!(o))).clicked() {
                chosen = Some(i);
            }
        }
        chosen
    })
}

/// The id [`craft_ui::ComboBox::from_id_salt`] gives its button in this `Ui`: `from_id_salt`
/// stores `IdSalt::new(salt)` and hashes it once more with the scope, so the app must hash `id`
/// the same way to read the button's focus and open state before it is drawn.
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
    let t = Tokens::get(ui.ctx());
    let label = take_previous_label(ui);
    egui::Frame::NONE
        .fill(t.input)
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(CornerRadius::same(3))
        .show(ui, |ui| {
            let button_id = combo_button_id(ui, &id);
            let enter_for_combo = ui.memory(|m| m.has_focus(button_id)) || craft_ui::ComboBox::is_open(ui.ctx(), button_id);
            let response = craft_ui::ComboBox::from_id_salt(HashOnly(&id))
                .selected_text(egui::RichText::new(current).size(12.0))
                .width(width - 4.0)
                .height(ui.spacing().combo_height)
                .show_ui(ui, list);
            // The button or open list handles Enter before the modal can accept it.
            if enter_for_combo {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            }
            let _ = name_field(label_field(response.response.clone(), label), name);
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
pub fn list_scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle { bar_width: 6.0, bar_inner_margin: 2.0, ..egui::style::ScrollStyle::solid() }
}

/// The widget that opened a popup and whether it is a submenu's row (`None` when none was found),
/// the frame the popup was opened in, and the frame it was last drawn in.
type Opener = (Option<(egui::Id, bool)>, u64, u64);

/// The frames after a popup opens in which a search that found no opener runs again.
const OPENER_RETRY_FRAMES: u64 = 3;

/// The widget that opened `popup`: a button whose popup has egui's default id, or a submenu's row,
/// found among the widgets of this frame and the previous one. Once found, later frames of the same
/// opening reuse it; a search that finds nothing runs again for the first few frames.
fn popup_opener(ctx: &egui::Context, popup: egui::Id) -> Option<(egui::Id, bool)> {
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
/// anchor.
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

/// The body of a menu or popup list: as tall as its items up to the bottom bars of the window it
/// opens in, and scrolling only past that, so long menus (Window, Effect, a panel's menu) stay
/// reachable on small windows and in a floating panel's short OS window. Its scroll bar has a column
/// of its own, so it never covers the shortcuts or the submenu arrows (#2086, #5347). When the list
/// scrolls, that column widens the popup instead of narrowing the items, so their labels stay on one
/// line.
///
/// The list's height comes from the room below and above the button, row or point the popup opens
/// from. A popup with an id of its own measures from its own top instead.
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
    // its bar. Later frames lay the items out at least that wide, and the bar's column is added to
    // the popup's width. The width is kept until the popup closes.
    let key = ui.next_auto_id().with("menu-scroll-width");
    let frame = ctx.cumulative_frame_nr();
    let first = ctx.data(|d| d.get_temp::<(u64, f32)>(key)).filter(|(shown, _)| shown + 1 >= frame).map(|(_, width)| width);
    let scroll = list_scroll_style();
    // The widest the items can be with the popup's margins and the bar's column inside the window.
    let widest = (ctx.content_rect().width() - margins.x - scroll.allocated_width()).max(0.0);
    ui.scope(|ui| {
        ui.spacing_mut().scroll = scroll;
        // The minimum lets the list grow past a popup's default 400 pt size; it still shrinks to
        // its items when they need less.
        let area = egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room);
        let out = area.show(ui, |ui| {
            // Always the same child, so the items' ids stay the same on the frame the width changes.
            let max = ui.max_rect();
            let width = first.map_or(max.width(), |first| first.min(widest).max(max.width()));
            let rect = egui::Rect::from_min_size(max.min, egui::vec2(width, max.height()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()), add_contents).inner
        });
        let width = first.unwrap_or(out.content_size.x);
        ctx.data_mut(|d| d.insert_temp(key, (frame, width)));
        out.inner
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
