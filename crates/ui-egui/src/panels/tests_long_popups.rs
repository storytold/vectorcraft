//! A headless window for checking that a panel's popup with no fixed length stays inside the window,
//! and the tests of menus built on `widgets::menu_scroll` that no panel draws.

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

/// The frames a popup is watched for after it opens.
const STEADY_FRAMES: usize = 12;

/// In a window `height` pt tall, the popup of the button `pick` finds draws its row `first` at the
/// same place on every frame it is shown, from the click on, inside the window. With the pointer on
/// that row, the row's submenu opens and every text of both stays in place too. Returns where the
/// row is drawn.
pub(super) fn assert_popup_stays_put(
    app: &mut VectorcraftApp,
    height: f32,
    draw: impl FnMut(&mut VectorcraftApp, &mut Ui),
    pick: impl FnOnce(&PopupWindow) -> Pos2,
    first: &str,
) -> Rect {
    let opened = Opened { with: PointerButton::Primary, first, hover: first };
    assert_popup_opened_stays_put(app, height, draw, pick, opened).0
}

/// How a popup is opened and checked: the mouse button `with` opens it, `first` is its first row,
/// and the pointer then moves onto the row `hover` to open its submenu.
struct Opened<'s> {
    with: PointerButton,
    first: &'s str,
    hover: &'s str,
}

/// [`assert_popup_stays_put`] for a popup opened as `opened` says. Returns where the first row is
/// drawn and where every text is drawn with the submenu open.
fn assert_popup_opened_stays_put(
    app: &mut VectorcraftApp,
    height: f32,
    draw: impl FnMut(&mut VectorcraftApp, &mut Ui),
    pick: impl FnOnce(&PopupWindow) -> Pos2,
    opened: Opened,
) -> (Rect, Vec<(String, Rect)>) {
    let Opened { with, first, hover } = opened;
    let mut window = PopupWindow::new(app, height, draw);
    let at = pick(&window);
    let button = |pressed| Event::PointerButton { pos: at, button: with, pressed, modifiers: Default::default() };
    window.frame(app, vec![Event::PointerMoved(at), button(true)]);
    window.frame(app, vec![button(false)]);
    let mut seen = vec![];
    for _ in 0..STEADY_FRAMES {
        seen.extend(window.text(first).map(|row| row.min));
        window.frame(app, vec![]);
    }
    let row = window.text(first).unwrap_or_else(|| panic!("the popup's first row, {first}"));
    seen.push(row.min);
    assert!(seen.iter().all(|seen| *seen == row.min), "{first} is drawn at one place on every frame: {seen:?}");
    assert!(row.top() >= 0.0 && row.bottom() <= height, "{first} at {row:?} is inside the {height} pt window");
    let popup_texts = window.texts.clone();
    let over = window.text(hover).unwrap_or_else(|| panic!("the popup's row {hover}"));
    window.frame(app, vec![Event::PointerMoved(over.center())]);
    window.frames(app, OPEN_FRAMES);
    let places = |window: &PopupWindow| window.texts.iter().map(|(text, rect)| (text.clone(), rect.min)).collect::<Vec<_>>();
    let shown = places(&window);
    assert!(shown.iter().any(|(text, _)| !popup_texts.iter().any(|(old, _)| old == text)), "the pointer on {hover} opens its submenu");
    for _ in 0..STEADY_FRAMES {
        window.frame(app, vec![]);
        assert_eq!(places(&window), shown, "the popup and its submenu stay in place");
    }
    (row, window.texts.clone())
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

/// Rows in each list of [`long_menu`]: more than any window in these tests holds.
const LONG: usize = 40;

/// A menu of rows `Row 0`, `Row 1`…, each a submenu of as many items.
fn long_menu(ui: &mut Ui) {
    menu_of(ui, LONG);
}

/// A menu of `rows` rows `Row 0`, `Row 1`…, each a submenu of [`LONG`] items.
fn menu_of(ui: &mut Ui, rows: usize) {
    crate::widgets::menu_scroll(ui, |ui| {
        for row in 0..rows {
            ui.menu_button(format!("Row {row}"), |ui| {
                crate::widgets::menu_scroll(ui, |ui| {
                    for item in 0..LONG {
                        let _ = ui.button(format!("Item {row}.{item}"));
                    }
                })
            });
        }
    });
}

/// A blank document, for windows that draw no panel.
fn app() -> VectorcraftApp {
    VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default())
}

/// A long menu opened from a button `y` pt down a window stays at one place on every frame, and so
/// does its submenu, whether it opens below the button, above it, or in a window shorter than it.
#[test]
fn a_long_menu_stays_put_wherever_its_button_is() {
    for height in [160.0, 300.0, 600.0] {
        for y in [4.0, height / 2.0 - 12.0, height - 28.0] {
            let draw = move |_: &mut VectorcraftApp, ui: &mut Ui| {
                ui.add_space(y);
                let button = ui.button("Open");
                egui::Popup::menu(&button).show(long_menu);
            };
            let open = |window: &PopupWindow| window.text("Open").expect("the button").center();
            assert_popup_stays_put(&mut app(), height, draw, open, "Row 0");
        }
    }
}

/// A long context menu opened near the top, the middle or the bottom of a short window stays at
/// one place on every frame, and so does its submenu.
#[test]
fn a_long_context_menu_stays_put_wherever_it_opens() {
    for height in [160.0, 300.0, 600.0] {
        for y in [4.0, height / 2.0, height - 4.0] {
            let draw = |_: &mut VectorcraftApp, ui: &mut Ui| {
                let area = ui.interact(ui.max_rect(), egui::Id::unique("context-area"), egui::Sense::click());
                area.context_menu(long_menu);
            };
            let at = move |_: &PopupWindow| Pos2::new(20.0, y);
            let opened = Opened { with: PointerButton::Secondary, first: "Row 0", hover: "Row 0" };
            assert_popup_opened_stays_put(&mut app(), height, draw, at, opened);
        }
    }
}

/// The long submenu of a row near the top of a window opens down from the row's top, and its list
/// reaches down to the window's bottom less the menu gap.
#[test]
fn a_long_submenu_of_a_high_row_reaches_down_to_the_window_bottom() {
    for height in [300.0, 600.0] {
        let draw = |_: &mut VectorcraftApp, ui: &mut Ui| {
            ui.add_space(4.0);
            let button = ui.button("Open");
            egui::Popup::menu(&button).show(|ui| menu_of(ui, 6));
        };
        let open = |window: &PopupWindow| window.text("Open").expect("the button").center();
        let mut app = app();
        let mut window = PopupWindow::new(&mut app, height, draw);
        let at = open(&window);
        window.click(&mut app, at);
        let row = window.text("Row 0").expect("the menu's first row");
        window.frame(&mut app, vec![Event::PointerMoved(row.center())]);
        window.frames(&mut app, OPEN_FRAMES);
        let item = window.text("Item 0.0").expect("the submenu's first item");
        let areas = window.ctx.memory(|m| m.layer_ids().filter_map(|layer| m.area_rect(layer.id)).collect::<Vec<_>>());
        let submenu = areas.into_iter().filter(|rect| rect.contains(item.center())).min_by(|a, b| a.area().total_cmp(&b.area()));
        let submenu = submenu.expect("the submenu's area");
        assert!(item.top() < row.bottom(), "the submenu opens down from {row:?}: {item:?}");
        assert!((height - 12.0..=height).contains(&submenu.bottom()), "in a {height} pt window the submenu reaches its bottom: {submenu:?}");
    }
}

/// The submenu of a row near the bottom of a window opens up from the row and stays at one place
/// on every frame.
#[test]
fn a_long_submenu_of_a_low_row_opens_up_and_stays_put() {
    for height in [300.0, 600.0] {
        let draw = move |_: &mut VectorcraftApp, ui: &mut Ui| {
            ui.add_space(height - 28.0);
            let button = ui.button("Open");
            egui::Popup::menu(&button).show(|ui| menu_of(ui, 6));
        };
        let open = |window: &PopupWindow| window.text("Open").expect("the button").center();
        let opened = Opened { with: PointerButton::Primary, first: "Row 0", hover: "Row 5" };
        let (_, texts) = assert_popup_opened_stays_put(&mut app(), height, draw, open, opened);
        let at = |wanted: &str| texts.iter().rfind(|(text, _)| text.trim() == wanted).map(|(_, rect)| *rect);
        let (row, item) = (at("Row 5").expect("the row"), at("Item 5.0").expect("the submenu's first item"));
        assert!(item.top() < row.top() - 100.0, "in a {height} pt window the submenu opens up from {row:?}: {item:?}");
    }
}
