//! The modal dialog window: a backdrop that keeps the rest of the app from taking clicks, and a
//! window with no title bar that opens centred and moves by dragging its heading (the band from its
//! top edge down to the heading), as a title bar would, showing the arrow until it is dragged. Where
//! it was moved to stays, per window, for the session, and the window never leaves the screen. Rows
//! too many for the screen's height scroll inside it ([`scrolled_rows`], [`body_rows`]), so its
//! heading and buttons stay in view. A window without such rows, or with them at their smallest,
//! that is still too tall or too wide for the screen scrolls as a whole on that axis.

use egui::{Id, Rect, Vec2};

use crate::theme::{self, Tokens};

/// Show the modal dialog window `id` (`title` is its accessible name), `nudge` points below the
/// screen's centre until it is moved, with `margin` inside its frame. `add` draws the contents and
/// should start with [`heading`] (or [`drag_band`]) so the window can be moved.
pub(crate) fn show<R>(ctx: &egui::Context, title: &str, id: Id, nudge: f32, margin: i8, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    show_at(ctx, title, id, egui::vec2(0.0, nudge), margin, None, add)
}

/// A modal that starts at a chosen offset, then keeps the user's dragged position.
pub(crate) fn show_at<R>(
    ctx: &egui::Context,
    title: &str,
    id: Id,
    initial: Vec2,
    margin: i8,
    canvas_pick: Option<Rect>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let t = Tokens::get(ctx);
    let dim = backdrop();
    if let Some(canvas) = canvas_pick {
        let screen = ctx.content_rect();
        let canvas = canvas.intersect(screen);
        // Separate areas keep their hit rectangles outside the canvas. One area around a hole
        // would still cover the canvas with its enclosing rectangle.
        let regions = [
            Rect::from_min_max(screen.min, egui::pos2(screen.right(), canvas.top())),
            Rect::from_min_max(egui::pos2(screen.left(), canvas.top()), egui::pos2(canvas.left(), canvas.bottom())),
            Rect::from_min_max(egui::pos2(canvas.right(), canvas.top()), egui::pos2(screen.right(), canvas.bottom())),
            Rect::from_min_max(egui::pos2(screen.left(), canvas.bottom()), screen.max),
        ];
        for (index, region) in regions.into_iter().enumerate().filter(|(_, region)| region.width() > 0.0 && region.height() > 0.0) {
            egui::Area::new(dim.id.with(index)).order(dim.order).fixed_pos(region.min).show(ctx, |ui| {
                ui.allocate_rect(region, egui::Sense::CLICK);
            });
        }
    } else {
        egui::Area::new(dim.id).order(dim.order).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| {
            // Modal, but the canvas isn't dimmed so previews stay readable (as in the reference app).
            // Not focusable (`Sense::click()` is): Tab goes through the dialog's controls, not the backdrop.
            ui.allocate_rect(ctx.content_rect(), egui::Sense::CLICK);
        });
    }
    let opened = opened(ctx, id);
    let shown = egui::Window::new(title)
        .id(id)
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, offset(ctx, id, initial))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(margin)))
        .show(ctx, |ui| {
            // Just opened: what scrolls in the window starts at its top, wherever it was left when
            // the window last closed.
            if opened {
                ctx.data_mut(|m| {
                    m.insert_temp(id.with(TOP_ROWS_KEY), true);
                    m.insert_temp(id.with(TOP_WHOLE_KEY), true);
                });
            }
            // A window whose contents are wider than the screen has room for, or taller without
            // rows that scroll ([`scrolled_rows`], [`body_rows`]) or with its own rows at their
            // smallest ([`fits_itself_then_scrolls`]), scrolls as a whole on that axis, so none of
            // it is out of reach. The contents keep their ids whether the window scrolls or not, so
            // a field being edited keeps its text when that changes.
            let [wide, tall] = ctx.data(|m| m.get_temp::<[bool; 2]>(id.with(WHOLE_KEY))).unwrap_or_default();
            let scope = egui::UiBuilder::new().scope_id(id.with("modal-contents"));
            let (r, contents, view) = if wide || tall {
                let room = whole_room(ctx, margin);
                let mut area = egui::ScrollArea::new([wide, tall]);
                if ctx.data_mut(|m| m.remove_temp::<bool>(id.with(TOP_WHOLE_KEY))).is_some() {
                    area = area.scroll_offset(Vec2::ZERO);
                }
                let out = area
                    .id_salt("modal-whole")
                    .max_width(room.x)
                    .max_height(room.y)
                    .min_scrolled_width(room.x)
                    .min_scrolled_height(room.y)
                    .show(ui, |ui| ui.scope_builder(scope, add).inner);
                // The area's rectangle is the one it was given before its contents were drawn; on
                // an axis that does not scroll, the window grows to the contents anyway.
                (out.inner, out.content_size, if tall { out.inner_rect.height() } else { out.content_size.y })
            } else {
                let r = ui.scope_builder(scope, add).inner;
                let size = ui.min_rect().size();
                (r, size, size.y)
            };
            ctx.data_mut(|m| {
                m.insert_temp(id.with(CONTENTS_KEY), contents);
                m.insert_temp(id.with(VIEW_KEY), view);
            });
            r
        })?;
    keep_rest(ctx, id, shown.response.rect.height(), margin);
    shown.inner
}

/// Mark the modal window drawn next in this pass as opened over the windows drawn in the pass
/// before (a dialog and any alert of its own), which it hides until it closes and shows them again
/// (Import Shortcut Set over Keyboard Shortcuts). The windows under it keep where they were
/// scrolled to.
pub(crate) fn over_parent(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|m| m.insert_temp(Id::unique(OVER_KEY), pass));
}

/// Record that modal window `id` is drawn in this pass, and return whether it opened in it: it was
/// not drawn in the pass before, and none of the windows drawn in the pass before was one opened
/// over it ([`over_parent`]).
fn opened(ctx: &egui::Context, id: Id) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|m| {
        // The windows drawn in this pass so far and in the pass before.
        let (at, now, prev) = m.get_temp::<(u64, Vec<Id>, Vec<Id>)>(Id::unique(LAST_KEY)).unwrap_or_default();
        let (mut now, prev) = if at == pass {
            (now, prev)
        } else if at.saturating_add(1) == pass {
            (vec![], now)
        } else {
            (vec![], vec![])
        };
        let before: Vec<Id> = prev.iter().copied().filter(|&other| other != id).collect();
        now.push(id);
        m.insert_temp(Id::unique(LAST_KEY), (pass, now, prev));
        // Only the window drawn next after `over_parent` is opened over the others.
        let over = m.remove_temp::<u64>(Id::unique(OVER_KEY)) == Some(pass);
        let last = std::mem::replace(m.get_temp_mut_or(id.with(SHOWN_KEY), u64::MAX), pass);
        if last != u64::MAX && last.saturating_add(1) >= pass {
            return false;
        }
        // Shown again as the window opened over it closed.
        if before.iter().any(|under| m.get_temp::<Vec<Id>>(under.with(HOLDS_KEY)).is_some_and(|held| held.contains(&id))) {
            return false;
        }
        // Opened over the windows drawn in the pass before, it holds them; otherwise none.
        if over && !before.is_empty() {
            m.insert_temp(id.with(HOLDS_KEY), before);
        } else {
            m.remove::<Vec<Id>>(id.with(HOLDS_KEY));
        }
        true
    })
}

/// The windows modal window `id` was opened over and hides ([`over_parent`]).
#[cfg(test)]
fn holds(ctx: &egui::Context, id: Id) -> Vec<Id> {
    ctx.data(|m| m.get_temp::<Vec<Id>>(id.with(HOLDS_KEY))).unwrap_or_default()
}

/// The room for the contents of a modal window with `margin` inside its frame.
fn whole_room(ctx: &egui::Context, margin: i8) -> Vec2 {
    (ctx.content_rect().size() - Vec2::splat(2.0 * (super::EDGE_GAP + f32::from(margin)))).max(Vec2::ZERO)
}

/// The layer of the backdrop behind a modal dialog window, which takes the clicks the rest of
/// the app would get.
pub(crate) fn backdrop() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, Id::unique("modal-dim"))
}

/// Whether [`show_at`] drew a modal window in this pass. Unlike the backdrop's visibility, which
/// also counts the last pass, it is false in the pass after the window closes.
pub(crate) fn drawn_this_pass(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.get_temp_mut_or::<(u64, Vec<Id>, Vec<Id>)>(Id::unique(LAST_KEY), (u64::MAX, vec![], vec![])).0 == pass)
}

/// The dialog's heading, which also moves the window ([`drag_band`]).
pub(crate) fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    // Not selectable: a drag on it moves the window instead of selecting its text.
    let r = ui.add(egui::Label::new(egui::RichText::new(text).font(theme::semibold(16.0)).color(t.text)).selectable(false));
    drag_band(ui, r.rect.bottom());
}

/// Make the band of the dialog window [`show`] draws `ui` in, from its top edge down to `bottom`, a
/// handle that moves the window. Widgets added later on the band stay clickable (they sit above it).
pub(crate) fn drag_band(ui: &mut egui::Ui, bottom: f32) {
    let id = ui.layer_id().id;
    // Last frame's rectangle: none while the window measures itself the first time it shows.
    let Some(outer) = ui.ctx().memory(|m| m.area_rect(id)) else { return };
    let band = Rect::from_x_y_ranges(outer.x_range(), outer.top()..=bottom.max(outer.top()));
    // Not focusable (`Sense::drag()` is): Tab goes through the dialog's controls, not the band.
    let r = ui.interact(band, handle(id), egui::Sense::DRAG);
    // Hovered, it keeps the arrow, as a title bar does (the grab cursor is Windows' four-way move
    // arrows: #5246); only while it is dragged does it show the window being carried.
    if r.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
}

/// The drag handle of window `id`.
pub(super) fn handle(id: Id) -> Id {
    id.with("modal-drag")
}

/// Under a modal window's id: how tall its [`scrolled_rows`] (or [`body_rows`]) were drawn this
/// frame, and how tall the rest of it was when last drawn.
const ROWS_KEY: &str = "modal-rows";
const REST_KEY: &str = "modal-rest";
/// Under a modal window's id: its contents drew [`scrolled_rows`] of their own or fit themselves to
/// the screen ([`fits_itself`]) this frame (`OWN_ROWS_KEY`), or last frame (`OWN_BEFORE_KEY`).
const OWN_ROWS_KEY: &str = "modal-own-rows";
const OWN_BEFORE_KEY: &str = "modal-own-before";
/// Under a modal window's id: its contents fit themselves to the screen and let the window scroll as
/// a whole below their smallest ([`fits_itself_then_scrolls`]) this frame, and whether they are at
/// their smallest.
const OWN_WHOLE_KEY: &str = "modal-own-whole";
/// Under a modal window's id: how tall its [`body_rows`] would be without scrolling, this frame.
const BODY_KEY: &str = "modal-body";
/// Under a modal window's id: how large its contents would be without scrolling as a whole
/// (`CONTENTS_KEY`) and how tall the part of them in view is (`VIEW_KEY`), this frame; and how tall
/// the window would be without scrolling as a whole, as last drawn (`FULL_KEY`, [`full_height`]).
const CONTENTS_KEY: &str = "modal-contents";
const VIEW_KEY: &str = "modal-view";
const FULL_KEY: &str = "modal-full";
/// Under a modal window's id, for the next frame: its [`body_rows`] scroll (`TALL_KEY`), and on
/// which axes ([wide, tall]) it scrolls as a whole (`WHOLE_KEY`).
const TALL_KEY: &str = "modal-tall";
const WHOLE_KEY: &str = "modal-whole";
/// Under a modal window's id: the pass it was last drawn in (`SHOWN_KEY`), and, since it opened,
/// its rows (`TOP_ROWS_KEY`) or the window as a whole (`TOP_WHOLE_KEY`) have not scrolled yet, so
/// they start at the top when they first do.
const SHOWN_KEY: &str = "modal-shown";
const TOP_ROWS_KEY: &str = "modal-top-rows";
const TOP_WHOLE_KEY: &str = "modal-top-whole";
/// For all modal windows: the pass the last one was drawn in, the ids of those drawn in that pass
/// and of those drawn in the pass before it (`LAST_KEY`, which [`drawn_this_pass`] also reads), and
/// the pass in which the window drawn next was marked as opened over others ([`over_parent`],
/// `OVER_KEY`, taken by that window). Under a modal window's id: the windows it was opened over and
/// hides (`HOLDS_KEY`).
const LAST_KEY: &str = "modal-last";
const OVER_KEY: &str = "modal-over";
const HOLDS_KEY: &str = "modal-holds";

/// Rows of the modal window `ui` is in (one list of them per window) that scroll if the window
/// would be taller than the screen. Their height is the screen's less the rest of the window as
/// last drawn (a frame late, as only the drawn window knows its height), and at least
/// one control tall; below that, the window scrolls as a whole ([`fits_itself_then_scrolls`]).
/// Rows that fit are as tall as they are.
pub(crate) fn scrolled_rows<R>(ui: &mut egui::Ui, salt: impl egui::AsIdSalt, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    fits_itself_then_scrolls(ui, rows_room(ui.ctx(), ui.layer_id().id) <= ui.spacing().interact_size.y);
    let (r, drawn, _) = rows(ui, salt, true, add);
    ui.ctx().data_mut(|m| m.insert_temp(ui.layer_id().id.with(ROWS_KEY), drawn));
    r
}

/// Mark the contents of the modal window `ui` is in as fitting themselves to the screen (sizing
/// what scrolls in them from the screen's height), so neither [`body_rows`] nor the window
/// scroll them again.
pub(crate) fn fits_itself(ui: &egui::Ui) {
    ui.ctx().data_mut(|m| m.insert_temp(ui.layer_id().id.with(OWN_ROWS_KEY), true));
}

/// [`fits_itself`] for contents whose smallest size can still be taller than the screen has room
/// for: while they are drawn at their `smallest` and the window is taller than the room, it
/// scrolls as a whole. The contents fit themselves to the screen's height less the gaps above and
/// below the window and the rest of the window as [`full_height`] gives it.
pub(crate) fn fits_itself_then_scrolls(ui: &egui::Ui, smallest: bool) {
    fits_itself(ui);
    ui.ctx().data_mut(|m| m.insert_temp(ui.layer_id().id.with(OWN_WHOLE_KEY), smallest));
}

/// How tall modal window `id` was when last drawn, with all of its contents: while it scrolls as a
/// whole, taller than the part drawn on the screen.
pub(crate) fn full_height(ctx: &egui::Context, id: Id) -> Option<f32> {
    ctx.data(|m| m.get_temp::<f32>(id.with(FULL_KEY)))
}

/// The body of a shared dialog window. While the window would be taller than the screen has room
/// for, the body scrolls as [`scrolled_rows`] do, so the heading and the buttons stay in view;
/// otherwise it is drawn as it is. A body that fits itself ([`fits_itself`]) is drawn as it is.
pub(crate) fn body_rows<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let id = ui.layer_id().id;
    let (r, drawn, body) = rows(ui, "dialog-body", body_scrolls(ui.ctx(), id), add);
    if !ui.ctx().data(|m| m.get_temp::<bool>(id.with(OWN_ROWS_KEY))).unwrap_or(false) {
        ui.ctx().data_mut(|m| {
            m.insert_temp(id.with(ROWS_KEY), drawn);
            m.insert_temp(id.with(BODY_KEY), body);
        });
    }
    r
}

/// Whether the [`body_rows`] of modal window `id` scroll in this frame.
fn body_scrolls(ctx: &egui::Context, id: Id) -> bool {
    let own = ctx.data(|m| m.get_temp::<bool>(id.with(OWN_BEFORE_KEY))).unwrap_or(false);
    !own && ctx.data(|m| m.get_temp::<bool>(id.with(TALL_KEY))).unwrap_or(false)
}

/// While the [`body_rows`] of the modal window `ui` is in scroll: how tall they are drawn.
pub(crate) fn body_view(ui: &egui::Ui) -> Option<f32> {
    let id = ui.layer_id().id;
    body_scrolls(ui.ctx(), id).then(|| rows_room(ui.ctx(), id))
}

/// How modal window `id` scrolls: while its [`body_rows`] scroll, how tall they are drawn; and on
/// which axes ([wide, tall]) the window scrolls as a whole.
#[cfg(test)]
pub(crate) fn scrolling(ctx: &egui::Context, id: Id) -> (Option<f32>, [bool; 2]) {
    let body = body_scrolls(ctx, id).then(|| rows_room(ctx, id));
    (body, ctx.data(|m| m.get_temp::<[bool; 2]>(id.with(WHOLE_KEY))).unwrap_or_default())
}

/// The height of a list `height` tall at most, with a scroll bar of its own, in the [`body_rows`]
/// of the modal window `ui` is in: while the body scrolls, no taller than the body is drawn, so
/// the list and its bar can be scrolled into view whole.
pub(crate) fn list_height(ui: &egui::Ui, height: f32) -> f32 {
    body_view(ui).map_or(height, |view| view.min(height))
}

/// The height rows that scroll in modal window `id` are drawn: the screen's less the rest of the
/// window as last drawn (as tall as all of it while it scrolls as a whole, [`full_height`]).
fn rows_room(ctx: &egui::Context, id: Id) -> f32 {
    let rest = ctx.data(|m| m.get_temp::<f32>(id.with(REST_KEY))).unwrap_or(0.0);
    (ctx.content_rect().height() - rest - 2.0 * super::EDGE_GAP).max(0.0)
}

/// [`scrolled_rows`] without marking the window's body as having rows of its own or keeping their
/// height. Also returns how tall the rows were drawn and how tall they would be without scrolling.
/// Rows that do not `scroll` are as tall as they are, with no
/// scroll area (which would take the scroll requests of their fields from the window's). The rows
/// keep their ids either way, so a field being edited keeps its text when they start or stop
/// scrolling.
fn rows<R>(ui: &mut egui::Ui, salt: impl egui::AsIdSalt, scroll: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> (R, f32, f32) {
    let id = ui.layer_id().id;
    let mut room = rows_room(ui.ctx(), id);
    if ui.ctx().data(|m| m.get_temp::<bool>(id.with(OWN_WHOLE_KEY))).is_some() {
        room = room.max(ui.spacing().interact_size.y);
    }
    let scope = egui::UiBuilder::new().scope_id(ui.make_persistent_id(&salt).with("modal-rows"));
    if !scroll {
        let rows = ui.scope_builder(scope, add);
        let height = rows.response.rect.height();
        return (rows.inner, height, height);
    }
    // As tall as the room at once: by itself a scroll area grows a frame at a time with the window.
    let mut area = egui::ScrollArea::vertical().id_salt(salt).max_height(room).min_scrolled_height(room);
    if ui.ctx().data_mut(|m| m.remove_temp::<bool>(id.with(TOP_ROWS_KEY))).is_some() {
        area = area.scroll_offset(Vec2::ZERO);
    }
    let rows = ui.scope(|ui| area.show(ui, |ui| ui.scope_builder(scope, add).inner));
    (rows.inner.inner, rows.response.rect.height(), rows.inner.content_size.y)
}

/// After modal window `id` (with `margin` inside its frame) is drawn `height` tall: keep the height
/// of all of it but its [`scrolled_rows`] for the next frame. If the rows were sized by a different
/// one (the window just opened, or what is around them changed), the next frame is drawn at once,
/// for the rows to fit without waiting for input. From the next frame on, [`body_rows`] scroll
/// while the window would be taller than the screen has room for without them scrolling; a window
/// without rows that scroll, or with rows of its own at their smallest
/// ([`fits_itself_then_scrolls`]), scrolls as a whole while its contents would be taller than the
/// room, and any window scrolls sideways while its contents would be wider.
fn keep_rest(ctx: &egui::Context, id: Id, height: f32, margin: i8) {
    let rows = ctx.data_mut(|m| m.remove_temp::<f32>(id.with(ROWS_KEY)));
    let body = ctx.data_mut(|m| m.remove_temp::<f32>(id.with(BODY_KEY)));
    let contents = ctx.data_mut(|m| m.remove_temp::<Vec2>(id.with(CONTENTS_KEY))).unwrap_or_default();
    let view = ctx.data_mut(|m| m.remove_temp::<f32>(id.with(VIEW_KEY))).unwrap_or(contents.y);
    // While the window scrolls as a whole, the rest of it is measured from the window as tall as
    // all of its contents. Measured from the part in view, rows sized from the rest would stay
    // short when the screen grows, and the window would go on scrolling.
    let height = height + (contents.y - view).max(0.0);
    ctx.data_mut(|m| m.insert_temp(id.with(FULL_KEY), height));
    let own = ctx.data_mut(|m| m.remove_temp::<bool>(id.with(OWN_ROWS_KEY))).unwrap_or(false);
    let own_whole = ctx.data_mut(|m| m.remove_temp::<bool>(id.with(OWN_WHOLE_KEY)));
    ctx.data_mut(|m| m.insert_temp(id.with(OWN_BEFORE_KEY), own));
    let room = whole_room(ctx, margin);
    let screen = ctx.content_rect().height() - 2.0 * super::EDGE_GAP;
    let tall = match (rows, body) {
        (Some(rows), Some(body)) if !own => height - rows + body > screen + 0.5,
        _ => false,
    };
    // A window that fits itself to the screen can still be too wide for it, and too tall once what
    // it sizes is at its smallest.
    let over = contents.y > room.y + 0.5;
    let whole = [contents.x > room.x + 0.5, own_whole.map_or(!own && rows.is_none() && over, |smallest| smallest && over)];
    let was_tall = ctx.data_mut(|m| std::mem::replace(m.get_temp_mut_or(id.with(TALL_KEY), false), tall));
    let was_whole = ctx.data_mut(|m| std::mem::replace(m.get_temp_mut_or(id.with(WHOLE_KEY), [false; 2]), whole));
    let mut repaint = tall != was_tall || whole != was_whole;
    if let Some(rows) = rows {
        let rest = (height - rows).max(0.0);
        let used = ctx.data_mut(|m| std::mem::replace(m.get_temp_mut_or(id.with(REST_KEY), 0.0), rest));
        repaint |= (used - rest).abs() > 0.5;
    }
    if repaint {
        ctx.request_repaint();
    }
}

/// Where window `id` sits, from the screen's centre: `nudge` down until it is first moved. While
/// its [`drag_band`] is dragged, where it sat when the drag began plus how far the pointer has moved
/// since the press, kept on screen (applied before the window places itself, so it follows the
/// pointer without a frame's lag).
fn offset(ctx: &egui::Context, id: Id, initial: Vec2) -> Vec2 {
    let (key, start_key) = (id.with("modal-offset"), id.with("modal-drag-start"));
    let offset = ctx.data(|m| m.get_temp::<Vec2>(key)).unwrap_or(initial);
    let dragged = ctx.read_response(handle(id)).is_some_and(|r| r.dragged());
    let moved = ctx.input(|i| Some(i.pointer.latest_pos()? - i.pointer.press_origin()?));
    let (true, Some(moved), Some(rect)) = (dragged, moved, ctx.memory(|m| m.area_rect(id))) else {
        ctx.data_mut(|m| m.remove::<Vec2>(start_key));
        return offset;
    };
    let start = ctx.data_mut(|m| *m.get_temp_mut_or(start_key, offset));
    let screen = ctx.content_rect();
    let to = on_screen(Rect::from_center_size(screen.center() + start + moved, rect.size()), screen).center() - screen.center();
    ctx.data_mut(|m| m.insert_temp(key, to));
    to
}

/// `r` moved the least to lie inside `screen` (its top left inside when it is larger).
fn on_screen(r: Rect, screen: Rect) -> Rect {
    let x = r.left().min(screen.right() - r.width()).max(screen.left());
    let y = r.top().min(screen.bottom() - r.height()).max(screen.top());
    Rect::from_min_size(egui::pos2(x, y), r.size())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moved_window_stays_on_screen() {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        assert_eq!(on_screen(r(100.0, 100.0, 200.0, 100.0), screen), r(100.0, 100.0, 200.0, 100.0));
        assert_eq!(on_screen(r(-50.0, 550.0, 200.0, 100.0), screen), r(0.0, 500.0, 200.0, 100.0));
        assert_eq!(on_screen(r(700.0, -20.0, 200.0, 100.0), screen), r(600.0, 0.0, 200.0, 100.0));
        // Larger than the screen: its top left stays in view.
        assert_eq!(on_screen(r(-10.0, -10.0, 900.0, 700.0), screen), r(0.0, 0.0, 900.0, 700.0));
    }

    /// Draw `frames` frames of a 400×300 screen from `time`, the first with `events`, with the modal
    /// windows named in `open` open, in that order: the first a heading and 60 rows, in
    /// [`scrolled_rows`] when `in_rows`, and any other a short alert. A window named "over …" is
    /// opened over the one before it ([`over_parent`]). Returns whether a first row could be seen in
    /// the last frame (inside the screen and its clip).
    fn first_row_seen(ctx: &egui::Context, open: &[&str], in_rows: bool, events: Vec<egui::Event>, time: &mut f64, frames: usize) -> bool {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let add = |ui: &mut egui::Ui| {
            for i in 0..60 {
                ui.label(format!("Row {i}"));
            }
        };
        let mut events = Some(events);
        let mut seen = false;
        for _ in 0..frames {
            *time += 1.0 / 60.0;
            let input =
                egui::RawInput { screen_rect: Some(screen), events: events.take().unwrap_or_default(), time: Some(*time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                for (i, &name) in open.iter().enumerate() {
                    if name.starts_with("over ") {
                        over_parent(ui.ctx());
                    }
                    show(ui.ctx(), name, Id::unique(("modal-reopen-test", name)), 0.0, 8, |ui| {
                        heading(ui, "Top");
                        if i > 0 {
                            ui.label("Alert");
                        } else if in_rows {
                            scrolled_rows(ui, "rows", add);
                        } else {
                            add(ui);
                        }
                    });
                }
            });
            out.textures_delta.clear();
            seen = out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::Text(t) => {
                    let r = t.visual_bounding_rect();
                    t.galley.text() == "Row 0" && screen.contains_rect(r) && c.clip_rect.contains_rect(r)
                }
                _ => false,
            });
        }
        seen
    }

    /// A wheel turn far down, at the middle of the 400×300 screen.
    fn wheel_down() -> Vec<egui::Event> {
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -3000.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(egui::pos2(200.0, 200.0)), wheel]
    }

    #[test]
    fn a_window_that_scrolled_opens_again_at_its_top() {
        for in_rows in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx, Default::default());
            let mut time = 0.0;
            assert!(first_row_seen(&ctx, &["A"], in_rows, vec![], &mut time, 4), "rows {in_rows}: the window opens at its top");
            assert!(!first_row_seen(&ctx, &["A"], in_rows, wheel_down(), &mut time, 60), "rows {in_rows}: the wheel scrolls the first row away");
            first_row_seen(&ctx, &[], in_rows, vec![], &mut time, 2);
            assert!(first_row_seen(&ctx, &["A"], in_rows, vec![], &mut time, 4), "rows {in_rows}: opened again, the window starts at its top");
        }
    }

    #[test]
    fn a_window_hidden_under_one_opened_over_it_keeps_its_scroll() {
        // As Keyboard Shortcuts is while Import Shortcut Set is open over it: it is not drawn, but it
        // did not close. A window that another replaced did close. Any of them may have shown an
        // alert of its own, drawn after it in the same passes: A before B or C opens, or B or C.
        let others: [(&[&str], bool); 4] = [(&["over B"], true), (&["over B", "Alert"], true), (&["C"], false), (&["C", "Alert"], false)];
        let id = |name: &str| Id::unique(("modal-reopen-test", name));
        let mut wrong = vec![];
        for in_rows in [false, true] {
            for lead in [&["A"][..], &["A", "Alert"][..]] {
                for (other, kept) in others {
                    let ctx = egui::Context::default();
                    crate::theme::apply(&ctx, Default::default());
                    let mut time = 0.0;
                    first_row_seen(&ctx, &["A"], in_rows, vec![], &mut time, 4);
                    if first_row_seen(&ctx, &["A"], in_rows, wheel_down(), &mut time, 60) {
                        wrong.push(format!("rows {in_rows}: the wheel does not scroll A"));
                    }
                    first_row_seen(&ctx, lead, in_rows, vec![], &mut time, 4);
                    if !first_row_seen(&ctx, other, in_rows, vec![], &mut time, 4) {
                        wrong.push(format!("rows {in_rows}: after {lead:?}, {other:?} does not open at its top"));
                    }
                    // Only the window opened over A holds it; an alert drawn after that window holds none.
                    let held: Vec<Vec<Id>> = other.iter().map(|&name| holds(&ctx, id(name))).collect();
                    let want: Vec<Vec<Id>> =
                        other.iter().map(|&name| if name.starts_with("over ") { lead.iter().map(|&l| id(l)).collect() } else { vec![] }).collect();
                    if held != want {
                        wrong.push(format!("rows {in_rows}: after {lead:?}, {other:?} hold {held:?}, not {want:?}"));
                    }
                    if first_row_seen(&ctx, &["A"], in_rows, vec![], &mut time, 4) == kept {
                        let state = if kept { "is not" } else { "is still" };
                        wrong.push(format!("rows {in_rows}: after {lead:?} and {other:?}, A {state} where it was scrolled to"));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn a_field_keeps_its_text_and_focus_when_the_window_starts_or_stops_scrolling() {
        // A field's edit state and the keyboard focus are kept under its id, so typing goes on into
        // the same field when the screen's height changes whether the body or the window scrolls.
        let mut wrong = vec![];
        for in_body in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx, Default::default());
            let window = Id::unique("modal-field-id-test");
            let mut text = String::from("12");
            let mut time = 0.0;
            let mut field = None;
            let mut focus_given = false;
            let mut seen = vec![];
            for (height, typed) in [(900.0, "3"), (300.0, "4"), (900.0, "5")] {
                let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, height));
                for i in 0..4 {
                    time += 1.0 / 60.0;
                    // Typed once the window has settled at this height.
                    let events = if i == 3 { vec![egui::Event::Text(typed.into())] } else { vec![] };
                    let input = egui::RawInput { screen_rect: Some(screen), events, time: Some(time), ..Default::default() };
                    let mut out = ctx.run_ui(input, |ui| {
                        show(ui.ctx(), "Test", window, 0.0, 8, |ui| {
                            heading(ui, "Top");
                            let mut add = |ui: &mut egui::Ui| {
                                let r = ui.text_edit_singleline(&mut text);
                                for i in 0..30 {
                                    ui.label(format!("Row {i}"));
                                }
                                r.id
                            };
                            field = Some(if in_body { body_rows(ui, add) } else { add(ui) });
                        });
                    });
                    out.textures_delta.clear();
                    // The field is given the focus once, after the first frame; a resize that took it
                    // away would leave the keys typed after it untaken.
                    if let Some(id) = field.filter(|_| !focus_given) {
                        ctx.memory_mut(|m| m.request_focus(id));
                        focus_given = true;
                    }
                }
                let key = if in_body { window.with(TALL_KEY) } else { window.with(WHOLE_KEY) };
                let scrolls = ctx.data(|m| m.get_temp::<bool>(key).unwrap_or(false) || m.get_temp::<[bool; 2]>(key).is_some_and(|a| a[1]));
                seen.push((scrolls, text.clone(), ctx.memory(|m| m.focused()) == field && field.is_some()));
            }
            let want = [(false, "123", true), (true, "1234", true), (false, "12345", true)];
            let want: Vec<_> = want.iter().map(|&(s, t, f)| (s, t.to_string(), f)).collect();
            if seen != want {
                wrong.push((in_body, seen));
            }
        }
        // Only the short screen scrolls, and the field keeps its focus and takes each key.
        assert_eq!(wrong, vec![], "(body, [(scrolls, text, focused)]) that went wrong");
    }
}
