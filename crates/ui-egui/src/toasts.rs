//! Messages that `app.toast` or a call of [`vectorcraft_engine::Session::toast`] queues, shown
//! over the window without taking the keyboard or stopping work: a stack at the window's bottom
//! right, the newest lowest. Each one goes after its time or when clicked; past
//! [`MAX_SHOWN`] the oldest goes first. A message shows at most [`MAX_ROWS`] lines, and the whole
//! text on hover; a stack taller than the window scrolls.

use std::time::Duration;

use vectorcraft_engine::cmd::toast::{Level, Toast};

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// The most messages shown at once.
pub const MAX_SHOWN: usize = 5;

/// The widest a message is, in points; longer text wraps.
const WIDTH: f32 = 320.0;

/// The narrowest a message gets in a narrow window, in points: its frame's margin and edge (22
/// points) and room for one character.
const MIN_WIDTH: f32 = 32.0;

/// The most lines a message shows; the rest shows on hover.
const MAX_ROWS: usize = 6;

/// The gap between the stack and the window's right and bottom edges, in points.
const MARGIN: f32 = 16.0;

/// A message shown.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) toast: Toast,
    /// When it goes, in [`crate::clock::now_secs`] seconds.
    until: f64,
    id: u64,
    /// Where the last pass drew it.
    pub(crate) rect: Option<egui::Rect>,
}

/// The messages shown, oldest first.
#[derive(Debug, Default)]
pub struct Shown {
    pub(crate) list: Vec<Entry>,
    next_id: u64,
}

impl Shown {
    /// Show `new` from `now` on, dropping the oldest past [`MAX_SHOWN`].
    fn add(&mut self, new: Vec<Toast>, now: f64) {
        for toast in new {
            let until = now + toast.seconds;
            self.list.push(Entry { toast, until, id: self.next_id, rect: None });
            self.next_id = self.next_id.wrapping_add(1);
        }
        let over = self.list.len().saturating_sub(MAX_SHOWN);
        self.list.drain(..over);
    }

    /// Drop the messages whose time is up at `now`; the seconds until the next one's is.
    fn expire(&mut self, now: f64) -> Option<f64> {
        self.list.retain(|e| e.until > now);
        self.list.iter().map(|e| e.until - now).reduce(f64::min)
    }
}

/// The color of a message's edge.
fn color(t: &Tokens, level: Level) -> egui::Color32 {
    match level {
        Level::Info => t.text_dim,
        Level::Success => t.success,
        Level::Warning => t.warning,
        Level::Error => t.error,
    }
}

/// Take the messages queued since the last pass and draw the ones shown.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    show_at(app, ctx, crate::clock::now_secs());
}

/// [`show`] at `now`, in [`crate::clock::now_secs`] seconds.
pub(crate) fn show_at(app: &mut VectorcraftApp, ctx: &egui::Context, now: f64) {
    // Messages whose time is up go before new ones count toward the limit.
    app.toasts.expire(now);
    let new = app.session.take_toasts();
    app.toasts.add(new, now);
    let Some(left) = app.toasts.expire(now) else {
        return;
    };
    // Finite and positive: every message's time is 0.5 to 60 seconds.
    ctx.request_repaint_after(Duration::from_secs_f64(left.clamp(0.0, 60.0)));
    let t = Tokens::get(ctx);
    let corner = egui::vec2(-MARGIN, -MARGIN);
    let area = egui::Area::new(egui::Id::unique("toasts")).order(egui::Order::Foreground).anchor(egui::Align2::RIGHT_BOTTOM, corner);
    // A stack taller than the window scrolls, kept at its newest message.
    let height = (ctx.content_rect().height() - 2.0 * MARGIN).max(0.0);
    // In a window narrower than a message, the messages narrow to fit it.
    let width = WIDTH.min(ctx.content_rect().width() - 2.0 * MARGIN).max(MIN_WIDTH);
    let dismissed = area
        .show(ctx, |ui| {
            let scroll = egui::ScrollArea::vertical().max_height(height).stick_to_bottom(true);
            scroll.show(ui, |ui| stack(ui, &mut app.toasts.list, &t, width)).inner
        })
        .inner;
    if let Some(id) = dismissed {
        app.toasts.list.retain(|e| e.id != id);
        ctx.request_repaint();
    }
}

/// Draw `list`, oldest first, `width` points wide; the id of the message clicked.
fn stack(ui: &mut egui::Ui, list: &mut [Entry], t: &Tokens, width: f32) -> Option<u64> {
    ui.set_max_width(width);
    let mut clicked = None;
    for e in list {
        // Each message's widgets keep its id when the messages before it go, so a press and a
        // release land on the same message.
        let r = ui.push_id(e.id, |ui| message(ui, e, t, width)).inner;
        if r.clicked() {
            clicked = Some(e.id);
        }
        e.rect = Some(r.rect);
        ui.add_space(6.0);
    }
    clicked
}

/// Draw one message `width` points wide → its response to clicks.
fn message(ui: &mut egui::Ui, e: &Entry, t: &Tokens, width: f32) -> egui::Response {
    let frame = egui::Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, color(t, e.toast.level)))
        .corner_radius(t.radius)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            // The frame's margin and edge take 11 points on each side.
            ui.set_width(width - 22.0);
            let format = egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), t.text);
            let mut job = egui::text::LayoutJob::single_section(e.toast.message.clone(), format);
            job.wrap.max_width = width - 22.0;
            job.wrap.max_rows = MAX_ROWS;
            let galley = ui.painter().layout_job(job);
            let elided = galley.elided;
            ui.add(egui::Label::new(galley).selectable(false));
            elided
        });
    // The click takes its id from the message's own scope, not from the frame's position among
    // the messages, so a press survives an older message going.
    let r = ui.interact(frame.response.rect, ui.scope_id().with("toast"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    // A message cut short shows in full on hover.
    if frame.inner { r.on_hover_text(&e.toast.message) } else { r }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    fn app() -> (VectorcraftApp, egui::Context) {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Brightness::Dark);
        (app, ctx)
    }

    /// One pass at `now` with `events`; the text the pass drew.
    fn pass(app: &mut VectorcraftApp, ctx: &egui::Context, now: f64, events: Vec<egui::Event>) -> Vec<String> {
        pass_in(app, ctx, now, events, 700.0)
    }

    /// [`pass`] in a window `height` points tall.
    fn pass_in(app: &mut VectorcraftApp, ctx: &egui::Context, now: f64, events: Vec<egui::Event>, height: f32) -> Vec<String> {
        pass_sized(app, ctx, now, events, egui::vec2(1000.0, height))
    }

    /// [`pass`] in a window of `size` points.
    fn pass_sized(app: &mut VectorcraftApp, ctx: &egui::Context, now: f64, events: Vec<egui::Event>, size: egui::Vec2) -> Vec<String> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| show_at(app, ui.ctx(), now));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape {
                texts.push(text.galley.text().to_string());
            }
        }
        texts
    }

    fn messages(app: &VectorcraftApp) -> Vec<&str> {
        app.toasts.list.iter().map(|e| e.toast.message.as_str()).collect()
    }

    #[test]
    fn a_message_shows_until_its_time_is_up() {
        let (mut app, ctx) = app();
        app.run("app.toast", json!({"message": "Exported 4 files", "seconds": 2})).unwrap();
        // A new area measures itself in its first pass and draws from the next.
        pass(&mut app, &ctx, 10.0, vec![]);
        let texts = pass(&mut app, &ctx, 10.0, vec![]);
        assert!(texts.iter().any(|t| t == "Exported 4 files"), "{texts:?}");
        let rect = app.toasts.list[0].rect.unwrap();
        assert!(rect.right() <= 1000.0 - MARGIN + 0.5 && rect.bottom() <= 700.0 - MARGIN + 0.5, "{rect:?}");
        assert!(pass(&mut app, &ctx, 11.9, vec![]).iter().any(|t| t == "Exported 4 files"));
        assert!(!pass(&mut app, &ctx, 12.0, vec![]).iter().any(|t| t == "Exported 4 files"));
        assert!(app.toasts.list.is_empty());
    }

    #[test]
    fn newer_messages_show_below_older_ones_and_the_oldest_goes_past_the_limit() {
        let (mut app, ctx) = app();
        for i in 0..MAX_SHOWN + 1 {
            app.session.toast(Toast::new(format!("m{i}"), Level::Info));
        }
        pass(&mut app, &ctx, 0.0, vec![]);
        assert_eq!(messages(&app), vec!["m1", "m2", "m3", "m4", "m5"]);
        let tops: Vec<f32> = app.toasts.list.iter().map(|e| e.rect.unwrap().top()).collect();
        assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tops:?}");
    }

    #[test]
    fn messages_whose_time_is_up_do_not_count_toward_the_limit() {
        let (mut app, ctx) = app();
        app.session.toast(Toast { seconds: 60.0, ..Toast::new("long", Level::Info) });
        for i in 0..MAX_SHOWN - 1 {
            app.session.toast(Toast { seconds: 0.5, ..Toast::new(format!("short{i}"), Level::Info) });
        }
        pass(&mut app, &ctx, 0.0, vec![]);
        app.session.toast(Toast::new("new", Level::Info));
        pass(&mut app, &ctx, 0.5, vec![]);
        assert_eq!(messages(&app), vec!["long", "new"]);
    }

    #[test]
    fn a_click_dismisses_only_that_message() {
        let (mut app, ctx) = app();
        app.session.toast(Toast::new("first", Level::Warning));
        app.session.toast(Toast::new("second", Level::Error));
        pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.0, vec![]);
        let at = app.toasts.list[0].rect.unwrap().center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        pass(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
        pass(&mut app, &ctx, 0.2, vec![press(true)]);
        pass(&mut app, &ctx, 0.3, vec![press(false)]);
        assert_eq!(messages(&app), vec!["second"]);
    }

    #[test]
    fn a_message_that_goes_between_press_and_release_closes_no_other() {
        let (mut app, ctx) = app();
        for (message, seconds) in [("top", 60.0), ("pressed", 0.5), ("bottom", 60.0)] {
            app.session.toast(Toast { seconds, ..Toast::new(message, Level::Info) });
        }
        pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.0, vec![]);
        let at = app.toasts.list[1].rect.unwrap().center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        pass(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
        pass(&mut app, &ctx, 0.2, vec![press(true)]);
        // "pressed" goes, and "top" moves down under the pointer.
        pass(&mut app, &ctx, 0.6, vec![]);
        pass(&mut app, &ctx, 0.7, vec![]);
        pass(&mut app, &ctx, 0.8, vec![press(false)]);
        assert_eq!(messages(&app), vec!["top", "bottom"]);
    }

    #[test]
    fn a_message_pressed_while_an_older_one_goes_still_closes_on_release() {
        let (mut app, ctx) = app();
        for (message, seconds) in [("older", 0.5), ("pressed", 60.0), ("bottom", 60.0)] {
            app.session.toast(Toast { seconds, ..Toast::new(message, Level::Info) });
        }
        pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.0, vec![]);
        let at = app.toasts.list[1].rect.unwrap().center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        pass(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
        pass(&mut app, &ctx, 0.2, vec![press(true)]);
        // "older" goes while the button is down; the newer messages stay where they were.
        pass(&mut app, &ctx, 0.6, vec![]);
        assert_eq!(messages(&app), vec!["pressed", "bottom"]);
        pass(&mut app, &ctx, 0.7, vec![]);
        pass(&mut app, &ctx, 0.8, vec![press(false)]);
        assert_eq!(messages(&app), vec!["bottom"]);
    }

    #[test]
    fn an_action_records_a_message_and_shows_it_again_when_played() {
        use crate::panels::actions::{START_RECORDING, STOP_RECORDING, play};
        let (mut app, _) = app();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run(START_RECORDING, json!({})).unwrap();
        app.run("app.toast", json!({"message": "Step done", "level": "success"})).unwrap();
        app.run(STOP_RECORDING, json!({})).unwrap();
        // The recording is the set's last action.
        let last = app.ui.action_sets[0].actions.len() - 1;
        let steps = &app.ui.action_sets[0].actions[last].steps;
        assert_eq!(steps, &vec![("app.toast".to_string(), json!({"message": "Step done", "level": "success"}))]);
        app.session.take_toasts();
        let undo = app.session.active().unwrap().history.undo.len();
        play(&mut app, 0, last).unwrap();
        assert_eq!(app.session.take_toasts(), vec![Toast::new("Step done", Level::Success)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "a message adds no undo step");
    }

    #[test]
    fn a_long_message_shows_its_first_lines_and_the_stack_stays_in_the_window() {
        let (mut app, ctx) = app();
        app.run("app.toast", json!({"message": "line"})).unwrap();
        app.run("app.toast", json!({"message": "line\n".repeat(100)})).unwrap();
        pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.0, vec![]);
        let (short, long) = (app.toasts.list[0].rect.unwrap(), app.toasts.list[1].rect.unwrap());
        // 100 lines would make it about 100 times as tall as a message of one line.
        assert!(long.height() > short.height() && long.height() < short.height() * MAX_ROWS as f32, "{short:?} {long:?}");
        app.toasts.list.remove(0);
        // Five such messages in a short window: the newest shows whole at the bottom, and none
        // reaches past the window's edges.
        for _ in 0..MAX_SHOWN - 1 {
            app.run("app.toast", json!({"message": "line\n".repeat(100)})).unwrap();
        }
        let height = 300.0;
        pass_in(&mut app, &ctx, 0.0, vec![], height);
        pass_in(&mut app, &ctx, 0.0, vec![], height);
        let newest = app.toasts.list.last().unwrap().rect.unwrap();
        assert!(newest.top() >= MARGIN - 0.5 && newest.bottom() <= height - MARGIN + 0.5, "{newest:?}");
        // The stack scrolls inside the window instead of growing past its top.
        let stack = ctx.memory(|m| m.area_rect(egui::Id::unique("toasts"))).unwrap();
        assert!(stack.top() >= MARGIN - 0.5 && stack.bottom() <= height - MARGIN + 0.5, "{stack:?}");
        assert_eq!(app.toasts.list.len(), MAX_SHOWN);
    }

    #[test]
    fn a_message_in_a_narrow_window_stays_inside_it() {
        let (mut app, ctx) = app();
        app.run("app.toast", json!({"message": "A message long enough to wrap onto more than one line"})).unwrap();
        let size = egui::vec2(200.0, 700.0);
        pass_sized(&mut app, &ctx, 0.0, vec![], size);
        pass_sized(&mut app, &ctx, 0.0, vec![], size);
        let rect = app.toasts.list[0].rect.unwrap();
        assert!(rect.left() >= MARGIN - 0.5 && rect.right() <= size.x - MARGIN + 0.5, "{rect:?}");
    }

    #[test]
    fn no_message_draws_nothing() {
        let (mut app, ctx) = app();
        assert!(pass(&mut app, &ctx, 0.0, vec![]).is_empty());
    }
}
