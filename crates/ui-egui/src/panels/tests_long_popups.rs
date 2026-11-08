//! A headless window for checking that a panel's popup with no fixed length stays inside the window.

use egui::{Event, PointerButton, Pos2, Rect, Ui, vec2};

use super::VectorcraftApp;

/// The frames that opening a popup takes.
const OPEN_FRAMES: usize = 8;
/// The frames that finishing a wheel's scrolling takes.
const SCROLL_FRAMES: usize = 40;
/// The window's width, wider than every popup.
const WIDTH: f32 = 500.0;
/// A window tall enough for every short list.
const TALL: f32 = 900.0;

/// What a window draws each frame.
type Draw<'a> = Box<dyn FnMut(&mut VectorcraftApp, &mut Ui) + 'a>;

/// A window `height` pt tall with its own clock, drawing `draw` each frame.
pub(super) struct PopupWindow<'a> {
    ctx: egui::Context,
    time: f64,
    height: f32,
    draw: Draw<'a>,
    texts: Vec<(String, Rect)>,
}

impl<'a> PopupWindow<'a> {
    pub(super) fn new(app: &mut VectorcraftApp, height: f32, draw: impl FnMut(&mut VectorcraftApp, &mut Ui) + 'a) -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut window = Self { ctx, time: 0.0, height, draw: Box::new(draw), texts: vec![] };
        window.frames(app, 2);
        window
    }

    /// One frame with `events`.
    fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, self.height)));
        let input = egui::RawInput { time: Some(self.time), screen_rect, events, ..Default::default() };
        let draw = &mut self.draw;
        let mut out = self.ctx.run_ui(input, |ui| draw(app, ui));
        out.textures_delta.clear();
        self.texts = crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>());
    }

    fn frames(&mut self, app: &mut VectorcraftApp, count: usize) {
        for _ in 0..count {
            self.frame(app, vec![]);
        }
    }

    /// Where the last frame drew the text `wanted`: the last one drawn, so a popup's row comes
    /// before a panel's text of the same name.
    pub(super) fn text(&self, wanted: &str) -> Option<Rect> {
        self.texts.iter().rfind(|(text, _)| text.trim() == wanted).map(|(_, rect)| *rect)
    }

    /// The rects of the last frame's clickable widgets `size` across, in the order they were drawn.
    pub(super) fn clickable(&self, size: egui::Vec2) -> Vec<Rect> {
        self.ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.size() == size)
                .map(|w| w.rect)
                .collect()
        })
    }

    /// A click at `at`, and the frames a popup takes to open.
    pub(super) fn click(&mut self, app: &mut VectorcraftApp, at: Pos2) {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        self.frame(app, vec![Event::PointerMoved(at), button(true)]);
        self.frame(app, vec![button(false)]);
        self.frames(app, OPEN_FRAMES);
    }

    /// Turns the wheel far down with the pointer at `over`, and lets the scrolling finish.
    pub(super) fn wheel_down(&mut self, app: &mut VectorcraftApp, over: Pos2) {
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        self.frame(app, vec![Event::PointerMoved(over), wheel]);
        self.frames(app, SCROLL_FRAMES);
    }
}

/// The Add Effect popup's first and last rows with no plug-ins: the effect catalog's groups, in order.
pub(super) fn fx_menu_ends() -> (String, String) {
    let mut groups: Vec<&str> = vec![];
    let catalog = vectorcraft_effects::effect_catalog();
    for group in catalog.iter().map(|effect| effect.menu.get(1).copied().unwrap_or("Other")) {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    (groups[0].to_string(), groups[groups.len() - 1].to_string())
}

/// In a window `height` pt tall, the popup of the button `pick` finds opens at its row `first`, and
/// the wheel brings its row `last` into the window.
pub(super) fn assert_last_row_scrolls_into_view(
    app: &mut VectorcraftApp,
    height: f32,
    draw: impl FnMut(&mut VectorcraftApp, &mut Ui),
    pick: impl FnOnce(&PopupWindow) -> Pos2,
    first: &str,
    last: &str,
) {
    let mut window = PopupWindow::new(app, height, draw);
    let button = pick(&window);
    window.click(app, button);
    let over = window.text(first).unwrap_or_else(|| panic!("the popup's first row, {first}")).center();
    assert!(window.text(last).is_none_or(|row| row.bottom() > height), "the popup opens at the top of its list, with {last} out of view");
    window.wheel_down(app, over);
    let row = window.text(last).unwrap_or_else(|| panic!("the wheel brings {last} into the {height} pt window"));
    assert!(row.bottom() <= height, "{last} at {row:?} is inside the {height} pt window");
}

/// In a tall window the popup of the button `pick` finds shows its rows `first` to `last` without
/// scrolling, and the wheel moves neither.
pub(super) fn assert_short_list_does_not_scroll(
    app: &mut VectorcraftApp,
    draw: impl FnMut(&mut VectorcraftApp, &mut Ui),
    pick: impl FnOnce(&PopupWindow) -> Pos2,
    first: &str,
    last: &str,
) {
    let mut window = PopupWindow::new(app, TALL, draw);
    let button = pick(&window);
    window.click(app, button);
    let shown = |window: &PopupWindow| {
        let (top, end) = (window.text(first).expect("the first row"), window.text(last).expect("the last row"));
        assert!(end.bottom() <= TALL, "{last} at {end:?} is inside the window");
        (top, end)
    };
    let before = shown(&window);
    window.wheel_down(app, before.0.center());
    assert_eq!(shown(&window), before, "a list that fits does not scroll");
}
