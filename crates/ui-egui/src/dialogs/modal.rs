//! The modal dialog window: a backdrop that keeps the rest of the app from taking clicks, and a
//! window with no title bar that opens centred and moves by dragging its heading (the band from its
//! top edge down to the heading), as a title bar would. Where it was moved to stays, per window, for
//! the session, and the window never leaves the screen.

use egui::{Id, Rect, Vec2};

use crate::theme::{self, Tokens};

/// Show the modal dialog window `id` (`title` is its accessible name), `nudge` points below the
/// screen's centre until it is moved, with `margin` inside its frame. `add` draws the contents and
/// should start with [`heading`] (or [`drag_band`]) so the window can be moved.
pub(crate) fn show<R>(ctx: &egui::Context, title: &str, id: Id, nudge: f32, margin: i8, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    show_at(ctx, title, id, egui::vec2(0.0, nudge), margin, add)
}

/// A modal that starts at a chosen offset, then keeps the user's dragged position.
pub(crate) fn show_at<R>(ctx: &egui::Context, title: &str, id: Id, initial: Vec2, margin: i8, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let t = Tokens::get(ctx);
    let dim = backdrop();
    egui::Area::new(dim.id).order(dim.order).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| {
        // Modal, but the canvas isn't dimmed so previews stay readable (as in the reference app).
        ui.allocate_rect(ctx.content_rect(), egui::Sense::click());
    });
    egui::Window::new(title)
        .id(id)
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, offset(ctx, id, initial))
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(margin)))
        .show(ctx, add)
        .and_then(|r| r.inner)
}

/// The layer of the backdrop behind a modal dialog window, which takes the clicks the rest of
/// the app would get.
pub(crate) fn backdrop() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, Id::new("modal-dim"))
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
    let r = ui.interact(band, handle(id), egui::Sense::drag());
    if r.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// The drag handle of window `id`.
fn handle(id: Id) -> Id {
    id.with("modal-drag")
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
}
