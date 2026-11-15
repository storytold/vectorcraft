//! The window's own title bar on Windows and Linux, where the window has no OS decorations
//! ([`VectorcraftApp::custom_titlebar`]): empty space in the app bar drags the window and a
//! double-click there maximizes it, caption buttons at the ends that [`CaptionLayout`] names (all three
//! at the far right by default) minimize, maximize/restore and close, and thin invisible zones along
//! the window's edges resize it. On macOS the app bar sits under the system's transparent title bar
//! and only its [`drag_area`] is used.

use egui::{CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};
use serde_json::json;

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// One caption button: as wide as the native Windows ones.
pub const BUTTON_WIDTH: f32 = 46.0;
/// The three caption buttons together.
pub const WIDTH: f32 = 3.0 * BUTTON_WIDTH;
/// Resize zone thickness along an edge, and the side of a corner zone.
const EDGE: f32 = 5.0;
const CORNER: f32 = 12.0;

/// A caption button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// The default order, left to right.
const CAPTIONS: [Caption; 3] = [Caption::Minimize, Caption::Maximize, Caption::Close];

/// Which caption buttons the app bar draws at its left end and at its right end, each list left to
/// right. The desktop app sets it from the desktop's window-button layout on Linux and the BSDs
/// ([`VectorcraftApp::caption_layout`]). The default has Minimize, Maximize and Close at the right.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CaptionLayout {
    pub left: Vec<Caption>,
    pub right: Vec<Caption>,
}

impl Default for CaptionLayout {
    fn default() -> Self {
        Self { left: Vec::new(), right: CAPTIONS.to_vec() }
    }
}

impl CaptionLayout {
    /// The width the buttons at the left end take.
    pub fn left_width(&self) -> f32 {
        self.left.len() as f32 * BUTTON_WIDTH
    }

    /// The width the buttons at the right end take.
    pub fn right_width(&self) -> f32 {
        self.right.len() as f32 * BUTTON_WIDTH
    }
}

/// Caption button ids (stable, so tests and agents can find them).
fn caption_id(c: Caption) -> Id {
    Id::unique(("titlebar-caption", c as u8))
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn toggle_maximize(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(ctx)));
}

/// The bar's background: a press-and-drag on empty space moves the window, a double-click toggles
/// maximize. Call it before laying out the bar so its widgets, registered later, take the clicks.
pub fn drag_area(ui: &Ui, rect: Rect) {
    let ctx = ui.ctx();
    // Not focusable (`click_and_drag` is): Tab goes through the bar's widgets, not its background.
    let resp = ui.interact(rect, Id::unique("titlebar-drag"), Sense::CLICK | Sense::DRAG);
    // egui hands a drag that starts on a click-only widget (a menu title) to the drag-sensing
    // background below it, so remember whether the press itself landed on bare background.
    if ctx.input(|i| i.pointer.any_pressed()) {
        let bare = ctx.viewport(|v| v.hits.click.is_some_and(|w| w.id == resp.id));
        ctx.data_mut(|d| d.insert_temp(resp.id, bare));
    }
    if resp.double_clicked() {
        toggle_maximize(ctx);
    } else if resp.drag_started_by(PointerButton::Primary) && ctx.data(|d| d.get_temp(resp.id).unwrap_or(false)) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        handed_to_os(ctx);
    }
}

/// The window manager runs the move or resize from here, and on X11 and Wayland it keeps the
/// button release: egui would hold the drag and the button until the next press, and that press
/// would start no drag on the bar or a resize zone (emilk/egui#7959). End both now. On Windows
/// winit hands the release back, which then ends nothing.
fn handed_to_os(ctx: &egui::Context) {
    ctx.stop_dragging();
    ctx.input_mut(|i| i.pointer = Default::default());
}

/// The buttons of [`VectorcraftApp::caption_layout`] at the left and right ends of `bar` (the app
/// bar's outer rect), full bar height down to its bottom border, flush with the window's edges so a
/// maximized window's corner hits the outermost button. Close runs `app.quit`, which asks about
/// unsaved documents first.
pub fn caption_buttons(app: &mut VectorcraftApp, ui: &mut Ui, bar: Rect) {
    let t = Tokens::get(ui.ctx());
    let max = maximized(ui.ctx());
    let mut clicked = None;
    let layout = &app.caption_layout;
    let left = layout.left.iter().enumerate().map(|(i, c)| (*c, bar.left() + i as f32 * BUTTON_WIDTH));
    let right_start = bar.right() - layout.right_width();
    let right = layout.right.iter().enumerate().map(|(i, c)| (*c, right_start + i as f32 * BUTTON_WIDTH));
    let placed: Vec<(Caption, f32)> = left.chain(right).collect();
    for (c, x) in placed {
        let r = Rect::from_min_size(pos2(x, bar.top()), vec2(BUTTON_WIDTH, bar.height() - 1.0));
        let resp = ui.interact(r, caption_id(c), Sense::click());
        let close = c == Caption::Close;
        let fill = match (resp.is_pointer_button_down_on(), resp.hovered()) {
            (true, _) if close => t.caption_close.gamma_multiply(0.8),
            (_, true) if close => t.caption_close,
            (true, _) => t.tool_active,
            (_, true) => t.hover,
            _ => egui::Color32::TRANSPARENT,
        };
        ui.painter().rect_filled(r, 0.0, fill);
        let ink = if close && resp.hovered() { t.caption_close_text } else { t.icon };
        paint_glyph(ui, c, max, r.center(), Stroke::new(1.0, ink));
        let tip = match c {
            Caption::Minimize => tl!("Minimize"),
            Caption::Maximize if max => tl!("Restore"),
            Caption::Maximize => tl!("Maximize"),
            Caption::Close => tl!("Close"),
        };
        if resp.on_hover_text(tip).clicked() {
            clicked = Some(c);
        }
    }
    match clicked {
        Some(Caption::Minimize) => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
        Some(Caption::Maximize) => toggle_maximize(ui.ctx()),
        Some(Caption::Close) => {
            app.run("app.quit", json!({})).ok();
        }
        None => {}
    }
}

/// The 10 pt line glyphs of the caption buttons.
fn paint_glyph(ui: &Ui, c: Caption, maximized: bool, at: egui::Pos2, s: Stroke) {
    let p = ui.painter();
    let g = Rect::from_center_size(at, vec2(10.0, 10.0));
    match c {
        Caption::Minimize => {
            p.line_segment([g.left_center(), g.right_center()], s);
        }
        // Restore: a front square with the corner of the one behind it.
        Caption::Maximize if maximized => {
            let front = Rect::from_min_max(g.min + vec2(0.0, 2.0), g.max - vec2(2.0, 0.0));
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Middle);
            p.line(vec![g.min + vec2(2.0, 2.0), g.min + vec2(2.0, 0.0), g.right_top(), g.max - vec2(0.0, 2.0), g.max - vec2(2.0, 2.0)], s);
        }
        Caption::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Middle);
        }
        Caption::Close => {
            p.line_segment([g.left_top(), g.right_bottom()], s);
            p.line_segment([g.right_top(), g.left_bottom()], s);
        }
    }
}

/// The edge and corner zones of a window with content rect `w`: (zone, direction, cursor). Corners
/// come last so they win where they overlap the edges.
fn zones(w: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    let (e, c) = (EDGE, CORNER);
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeVertical),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeVertical),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeHorizontal),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeHorizontal),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

/// Invisible resize zones along the window's edges, above everything else; a press in one starts
/// an OS resize in that direction. None while the window is maximized or fullscreen.
pub fn resize_zones(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    let layer = LayerId::new(Order::Foreground, Id::unique("titlebar-resize"));
    let zui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    for (i, (r, dir, cursor)) in zones(w).into_iter().enumerate() {
        // Not focusable (`Sense::drag()` is): Tab does not stop on the zones.
        let resp = zui.interact(r, Id::unique(("titlebar-resize", i)), Sense::DRAG).on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            handed_to_os(&ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, RawInput, ViewportId};
    use vectorcraft_engine::Session;

    /// A headless window `width` pt wide showing the app bar (as the title bar) and the resize zones.
    struct Win {
        ctx: egui::Context,
        app: VectorcraftApp,
        width: f32,
        time: f64,
        maximized: bool,
    }

    impl Win {
        fn new(width: f32) -> Self {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            crate::theme::apply(&ctx, Default::default());
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.custom_titlebar = true;
            Self { ctx, app, width, time: 0.0, maximized: false }
        }

        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            self.time += 0.05;
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            if let Some(v) = input.viewports.get_mut(&ViewportId::ROOT) {
                v.maximized = Some(self.maximized);
            }
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| {
                crate::chrome::app_bar(app, ui);
                resize_zones(ui);
            });
            out.textures_delta.clear();
            out
        }
    }

    fn texts(out: &egui::FullOutput) -> Vec<(String, Rect)> {
        fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    fn button(p: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    impl Win {
        /// Frames for a pointer gesture: move to `path[0]`, press, move along `path`, release. The
        /// viewport commands the frames sent.
        fn gesture(&mut self, path: &[Pos2]) -> Vec<ViewportCommand> {
            let mut events = vec![vec![Event::PointerMoved(path[0])], vec![button(path[0], true)]];
            events.extend(path[1..].iter().map(|p| vec![Event::PointerMoved(*p)]));
            events.push(vec![button(*path.last().unwrap_or(&path[0]), false)]);
            events.into_iter().flat_map(|e| self.frame(e).viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()).collect()
        }

        fn click(&mut self, p: Pos2) -> Vec<ViewportCommand> {
            self.gesture(&[p])
        }

        fn caption(&self, c: Caption) -> Rect {
            self.ctx.read_response(caption_id(c)).map(|r| r.rect).unwrap_or(Rect::NOTHING)
        }

        /// The bar's clickable widgets other than its background and the caption buttons.
        fn bar_widgets(&self) -> Vec<Rect> {
            let skip: Vec<Id> = std::iter::once(Id::unique("titlebar-drag")).chain(CAPTIONS.map(caption_id)).collect();
            self.ctx.viewport(|v| {
                v.prev_pass
                    .widgets
                    .layers()
                    .filter(|(l, _)| l.order != Order::Foreground)
                    .flat_map(|(_, ws)| ws.iter())
                    .filter(|w| w.sense.senses_click() && w.rect.top() < 32.0 && !skip.contains(&w.id))
                    .map(|w| w.rect)
                    .collect()
            })
        }
    }

    #[test]
    fn caption_buttons_take_the_right_end_and_nothing_overlaps_down_to_800_pt() {
        for width in [800.0, 1100.0, 1440.0] {
            let mut w = Win::new(width);
            w.frame(vec![]);
            let out = w.frame(vec![]);
            let [min, max, close] = CAPTIONS.map(|c| w.caption(c));
            assert_eq!((close.right(), close.top()), (width, 0.0), "Close sits in the window's corner at {width}");
            assert!(close.height() >= 30.0 && [min, max, close].iter().all(|r| r.width() == BUTTON_WIDTH));
            assert_eq!((min.right(), max.right()), (max.left(), close.left()));
            let widgets = w.bar_widgets();
            assert!(widgets.len() >= 11, "home, eight menus, workspace switcher, search: {widgets:?}");
            for (i, a) in widgets.iter().enumerate() {
                assert!(a.right() <= min.left() - 8.0, "{a:?} runs into the caption buttons at {width}");
                for b in &widgets[i + 1..] {
                    assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{a:?} overlaps {b:?} at {width}");
                }
            }
            // Wide windows show the whole group; narrow ones drop Discord, then the search icon.
            let texts: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
            let has = |s: &str| texts.iter().any(|t| t == s);
            assert_eq!(has("Discord"), width > 1000.0, "{width}: {texts:?}");
            // Search is an icon only (as in PhotoCraft): its label lives in the tooltip.
            assert!(!has("Search commands and tools"), "{width}: {texts:?}");
        }
    }

    /// The caption layout decides which buttons the bar draws and at which end, each end in its
    /// listed order: Close alone at the right, all three at the left with Close outermost, and a
    /// split. The bar's other controls stay between the two groups, and Close works at either end.
    #[test]
    fn caption_buttons_follow_the_caption_layout() {
        use Caption::*;
        let width = 1100.0;
        for (left, right) in [(vec![], vec![Close]), (vec![Close, Minimize, Maximize], vec![]), (vec![Close], vec![Minimize, Maximize])] {
            let label = format!("{left:?} : {right:?}");
            let mut w = Win::new(width);
            w.app.caption_layout = CaptionLayout { left: left.clone(), right: right.clone() };
            w.frame(vec![]);
            w.frame(vec![]);
            for (i, c) in left.iter().enumerate() {
                let r = w.caption(*c);
                assert_eq!((r.left(), r.top(), r.width()), (i as f32 * BUTTON_WIDTH, 0.0, BUTTON_WIDTH), "{c:?} in {label}");
            }
            for (i, c) in right.iter().enumerate() {
                let r = w.caption(*c);
                let right_edge = width - (right.len() - 1 - i) as f32 * BUTTON_WIDTH;
                assert_eq!((r.right(), r.top(), r.width()), (right_edge, 0.0, BUTTON_WIDTH), "{c:?} in {label}");
            }
            for c in CAPTIONS.into_iter().filter(|c| !left.contains(c) && !right.contains(c)) {
                assert_eq!(w.caption(c), Rect::NOTHING, "{c:?} is not drawn in {label}");
            }
            let (start, end) = (left.len() as f32 * BUTTON_WIDTH, width - right.len() as f32 * BUTTON_WIDTH);
            let widgets = w.bar_widgets();
            assert!(widgets.len() >= 11, "home, eight menus, workspace switcher, search in {label}: {widgets:?}");
            for r in &widgets {
                assert!(r.left() >= start + 8.0 && r.right() <= end - 8.0, "{r:?} runs into the caption buttons in {label}");
            }
            let close = w.caption(Close).center();
            w.click(close);
            assert_eq!(w.app.ui.status, "quit", "Close in {label}");
        }
    }

    #[test]
    fn close_runs_the_quit_flow_instead_of_closing_the_window() {
        let mut w = Win::new(1000.0);
        w.app.run("file.new", serde_json::json!({"width": 100, "height": 100})).unwrap();
        w.app.run("shape.rectangle", serde_json::json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        w.frame(vec![]);
        let close = w.caption(Caption::Close).center();
        let cmds = w.click(close);
        assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::Close)), "{cmds:?}");
        let dialog = w.app.ui.dialog.as_ref().map(|d| d.kind.clone());
        assert_eq!(dialog.as_deref(), Some(crate::unsaved::KIND), "a modified document asks first");
        assert_ne!(w.app.ui.status, "quit");
        // Nothing to save: the host is told to quit.
        w.app.ui.dialog = None;
        w.app.session.doc_mut().unwrap().mark_saved();
        w.click(close);
        assert_eq!(w.app.ui.status, "quit");
    }

    #[test]
    fn minimize_and_maximize_follow_the_window_state() {
        let mut w = Win::new(1000.0);
        w.frame(vec![]);
        let (min, max) = (w.caption(Caption::Minimize).center(), w.caption(Caption::Maximize).center());
        assert!(w.click(min).contains(&ViewportCommand::Minimized(true)));
        assert!(w.click(max).contains(&ViewportCommand::Maximized(true)));
        w.maximized = true;
        assert!(w.click(max).contains(&ViewportCommand::Maximized(false)), "Restore");
    }

    #[test]
    fn empty_bar_space_drags_and_double_click_maximizes() {
        let mut w = Win::new(1440.0);
        w.frame(vec![]);
        let empty = pos2(700.0, 16.0);
        assert!(w.bar_widgets().iter().all(|r| !r.contains(empty)), "the probe point must be empty bar space");
        let cmds = w.gesture(&[empty, empty + vec2(12.0, 4.0), empty + vec2(30.0, 8.0)]);
        assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
        // Double-click toggles maximize.
        let mut cmds = w.click(empty);
        cmds.extend(w.click(empty));
        assert!(cmds.contains(&ViewportCommand::Maximized(true)), "{cmds:?}");
        w.maximized = true;
        w.time += 1.0;
        let mut cmds = w.click(empty);
        cmds.extend(w.click(empty));
        assert!(cmds.contains(&ViewportCommand::Maximized(false)), "{cmds:?}");
        // Widgets keep their clicks: dragging from a menu title doesn't move the window.
        let file = texts(&w.frame(vec![])).into_iter().find(|(t, _)| t == "File").map(|(_, r)| r.center());
        let file = file.expect("the File menu title");
        let cmds = w.gesture(&[file, file + vec2(12.0, 4.0), file + vec2(30.0, 8.0)]);
        assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
    }

    #[test]
    fn a_double_click_on_the_macos_app_bar_zooms_the_window() {
        // macOS keeps the system decorations, with the app bar under its transparent title bar
        // (#1932); Windows and Linux with the system title bar leave it to the OS's bar.
        let mut w = Win::new(1440.0);
        w.app.custom_titlebar = false;
        w.frame(vec![]);
        let empty = pos2(700.0, 16.0);
        assert!(w.bar_widgets().iter().all(|r| !r.contains(empty)), "the probe point must be empty bar space");
        let mut cmds = w.click(empty);
        cmds.extend(w.click(empty));
        assert_eq!(cmds.contains(&ViewportCommand::Maximized(true)), cfg!(target_os = "macos"), "{cmds:?}");
        assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::Maximized(false))), "{cmds:?}");
    }

    #[test]
    fn edge_zones_resize_unless_maximized() {
        use ResizeDirection::*;
        let mut w = Win::new(1000.0);
        w.frame(vec![]);
        for (p, dir) in [
            (pos2(1.0, 300.0), West),
            (pos2(998.0, 300.0), East),
            (pos2(500.0, 1.0), North),
            (pos2(500.0, 598.0), South),
            (pos2(2.0, 2.0), NorthWest),
            (pos2(997.0, 3.0), NorthEast),
            (pos2(3.0, 597.0), SouthWest),
            (pos2(998.0, 598.0), SouthEast),
        ] {
            let cmds = w.gesture(&[p, p + vec2(6.0, 6.0)]);
            assert_eq!(cmds, vec![ViewportCommand::BeginResize(dir)], "at {p:?}");
        }
        let out = w.frame(vec![Event::PointerMoved(pos2(1.0, 300.0))]);
        assert_eq!(out.platform_output.cursor_icon, CursorIcon::ResizeHorizontal);
        // Maximized: no zones, and the window's top-right corner is Close.
        w.maximized = true;
        let cmds = w.gesture(&[pos2(1.0, 300.0), pos2(8.0, 300.0)]);
        assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))), "{cmds:?}");
        assert!(w.click(pos2(999.5, 0.5)).is_empty());
        assert_eq!(w.app.ui.status, "quit");
    }

    /// Tab goes through the bar's controls and does not stop on the bar's background or on the
    /// resize zones, which only the pointer uses.
    #[test]
    fn tab_skips_the_bar_background_and_the_resize_zones() {
        let mut w = Win::new(1000.0);
        w.frame(vec![]);
        let ctx = w.ctx.clone();
        let stops = crate::widgets::accessibility_tests::tab_stops(&ctx, 40, |events| {
            w.frame(events);
        });
        for c in CAPTIONS {
            assert!(stops.contains(&caption_id(c)), "Tab reaches the {c:?} button: {stops:?}");
        }
        let regions: Vec<Id> = std::iter::once(Id::unique("titlebar-drag")).chain((0..8usize).map(|i| Id::unique(("titlebar-resize", i)))).collect();
        assert!(!stops.iter().any(|id| regions.contains(id)), "{stops:?}");
    }

    #[test]
    fn a_move_or_resize_the_window_manager_finished_does_not_block_the_next_resize() {
        use ResizeDirection::*;
        fn sent(mut out: egui::FullOutput) -> Vec<ViewportCommand> {
            out.viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()
        }
        // After BeginResize or StartDrag the window manager owns the pointer and, on X11 and
        // Wayland, keeps the release: the app sees the press and the pointer leaving, nothing else.
        fn handed_off(w: &mut Win, path: &[Pos2]) -> Vec<ViewportCommand> {
            let mut events = vec![vec![Event::PointerMoved(path[0])], vec![button(path[0], true)]];
            events.extend(path[1..].iter().map(|p| vec![Event::PointerMoved(*p)]));
            events.push(vec![Event::PointerGone]);
            events.into_iter().flat_map(|e| sent(w.frame(e))).collect()
        }
        let (zone, out) = (pos2(1.0, 300.0), pos2(-39.0, 300.0));
        let mut w = Win::new(1440.0);
        w.frame(vec![]);
        assert_eq!(handed_off(&mut w, &[zone]), vec![ViewportCommand::BeginResize(West)]);
        assert_eq!(w.gesture(&[zone, out]), vec![ViewportCommand::BeginResize(West)], "the next resize");
        let (empty, moved) = (pos2(700.0, 16.0), pos2(712.0, 20.0));
        assert_eq!(handed_off(&mut w, &[empty, moved]), vec![ViewportCommand::StartDrag]);
        assert_eq!(w.gesture(&[zone, out]), vec![ViewportCommand::BeginResize(West)], "a resize after a move");
        // Windows hands the release back: it ends nothing, and a click right after it stays a single one.
        assert_eq!(handed_off(&mut w, &[empty, moved]), vec![ViewportCommand::StartDrag]);
        let mut cmds = sent(w.frame(vec![button(moved, false)]));
        cmds.extend(w.click(moved));
        assert!(cmds.is_empty(), "{cmds:?}");
    }
}
