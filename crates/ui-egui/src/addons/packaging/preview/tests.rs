use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft::egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2, vec2};
use craft::ui::testing::fixture::Fixture;
use craft::ui::testing::frames::Frames;
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_three_d::fold::raster::{Image, MAX_PIXELS, MAX_SIDE};

use super::{PrintKey, Settings, Shown, View, inside_color, show, status};
use crate::VectorcraftApp;

/// How long a test waits for the print to render.
const PRINT_WAIT: Duration = Duration::from_secs(30);

/// An app with a tray blank (a 100 pt base at x 140 to 240, y 140 to 240, and a 40 pt wall on each
/// side) and a red square printed on the base, nothing selected → (app, the red square's id).
fn app() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let path = |app: &mut VectorcraftApp, d: &str| app.run("path.create", json!({ "d": d })).unwrap()["id"].as_u64().unwrap();
    let outline = path(&mut app, "M140 100 L240 100 L240 140 L280 140 L280 240 L240 240 L240 280 L140 280 L140 240 L100 240 L100 140 L140 140 Z");
    let creases = [
        path(&mut app, "M140 140 L240 140"),
        path(&mut app, "M240 140 L240 240"),
        path(&mut app, "M240 240 L140 240"),
        path(&mut app, "M140 240 L140 140"),
    ];
    app.run("addon.packaging.setRole", json!({ "role": "cut", "ids": [outline] })).unwrap();
    app.run("addon.packaging.setRole", json!({ "role": "crease", "ids": creases })).unwrap();
    let red = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 180, "height": 180})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({ "ids": [red] })).unwrap();
    app.run("paint.setFill", json!({ "color": "#ff0000" })).unwrap();
    app.run("paint.setStroke", json!({ "none": true })).unwrap();
    app.run("object.arrange.sendToBack", json!({})).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app, red)
}

fn color_at(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = (y * img.width + x) as usize * 4;
    [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2], img.pixels[i + 3]]
}

/// The color at the middle of the window's last image.
fn middle(img: &Image) -> [u8; 4] {
    color_at(img, img.width / 2, img.height / 2)
}

/// The window in a room of `room` points at `ppp` pixels a point, over frames of the app's own.
struct Window {
    frames: Frames,
    room: Vec2,
}

impl Window {
    fn new(ppp: f32, room: Vec2) -> Self {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let mut frames = Frames::new();
        frames.pixels_per_point(ppp);
        Window { frames, room }
    }

    fn frame(&mut self, app: &mut VectorcraftApp) {
        let room = self.room;
        self.frames.frame(vec![], |ui| {
            ui.allocate_ui(room, |ui| show(app, ui));
        });
    }

    /// Frames until the print of the document as it is has been rendered and drawn.
    fn settle(&mut self, app: &mut VectorcraftApp) -> Shown {
        for _ in 0..SETTLE_FRAMES {
            self.frame(app);
            if print_ready(app) {
                break;
            }
            wait_for_print(app);
        }
        assert!(print_ready(app), "the print renders");
        for _ in 0..3 {
            self.frame(app);
        }
        app.addon_panels.fold_view.shown.clone().expect("a frame is drawn")
    }
}

/// Most frames a test runs while it waits for the print: the window waits for the document to
/// settle (by the real clock) before it renders a print of a changed document.
const SETTLE_FRAMES: usize = 200;

/// Whether the print shown is the one of the document and Show Fold Lines setting as they are.
fn print_ready(app: &VectorcraftApp) -> bool {
    let view = &app.addon_panels.fold_view;
    let wanted = (app.session.active().map(|st| st.revision), app.ui.fold_preview.show_fold_lines);
    view.pending.is_none() && view.print.as_ref().map(|(k, _)| (Some(k.revision), k.fold_lines)) == Some(wanted)
}

/// Lets a queued print finish, and gives the settle gate time to pass.
fn wait_for_print(app: &VectorcraftApp) {
    if let Some(queue) = &app.addon_panels.fold_view.prints {
        queue.finish(PRINT_WAIT);
    }
    std::thread::sleep(Duration::from_millis(20));
}

/// Draws the window in a 520 × 440 room and settles it.
fn drawn(app: &mut VectorcraftApp) -> Shown {
    Window::new(1.0, vec2(520.0, 440.0)).settle(app)
}

#[test]
fn ui_fold_preview_sets_the_window_and_refuses_bad_values() {
    let (mut app, _) = app();
    let r = app
        .run(
            "ui.foldPreview",
            json!({"amount": 0.5, "view": "top", "zoom": 2, "perspective": 40, "inside": "#00ff00", "showFoldLines": true, "base": [120, 190]}),
        )
        .unwrap();
    let want = Settings {
        amount: 0.5,
        rotate_x: 90.0,
        rotate_y: 0.0,
        zoom: 2.0,
        pan: [0.0; 2],
        perspective: 40.0,
        inside: "#00ff00".into(),
        show_fold_lines: true,
        base: Some([120.0, 190.0]),
    };
    assert_eq!(app.ui.fold_preview, want);
    assert_eq!(r["rotateX"], 90.0, "{r}");
    assert!(r.get("frame").is_some(), "{r}");
    app.run("ui.foldPreview", json!({"rotateX": -30, "rotateY": 170, "base": null})).unwrap();
    assert_eq!((app.ui.fold_preview.rotate_x, app.ui.fold_preview.rotate_y, app.ui.fold_preview.base), (-30.0, 170.0, None));
    let kept = app.ui.fold_preview.clone();
    for p in [
        json!({"amount": 2}),
        json!({"amount": "half"}),
        json!({"view": "sideways"}),
        json!({"rotateX": 200}),
        json!({"zoom": 0}),
        json!({"perspective": 170}),
        json!({"inside": "no such swatch"}),
        json!({"inside": 7}),
        json!({"showFoldLines": "yes"}),
        json!({"base": [1, "2"]}),
        json!({"base": [5, 5]}),
        // A good value before a bad one sets nothing either.
        json!({"amount": 0.2, "zoom": 99}),
    ] {
        assert!(app.run("ui.foldPreview", p.clone()).is_err(), "{p} is refused");
        assert_eq!(app.ui.fold_preview, kept, "{p} sets nothing");
    }
    // A swatch with a plain color names the inside.
    app.run("ui.foldPreview", json!({"inside": "White"})).unwrap();
    assert_eq!(app.ui.fold_preview.inside, "White");
    // So does one whose name starts with #, and a swatch's name comes before a #rrggbb that reads the same.
    app.run("swatch.new", json!({"name": "#brand", "color": "#0000ff"})).unwrap();
    app.run("swatch.new", json!({"name": "#00ff00", "color": "#0000ff"})).unwrap();
    app.run("ui.foldPreview", json!({"inside": "#brand"})).unwrap();
    assert_eq!(app.ui.fold_preview.inside, "#brand");
    let doc = &app.session.active().expect("a document").doc;
    assert_eq!(inside_color(doc, "#brand"), Some([0, 0, 255, 255]));
    assert_eq!(inside_color(doc, "#00ff00"), Some([0, 0, 255, 255]));
    assert_eq!(inside_color(doc, "#ff0000"), Some([255, 0, 0, 255]));
    // White, the default, stays white beside a swatch named #ffffff.
    app.run("swatch.new", json!({"name": "#ffffff", "color": "#0000ff"})).unwrap();
    let doc = &app.session.active().expect("a document").doc;
    assert_eq!(inside_color(doc, &Settings::default().inside), Some([255; 4]));
    assert_eq!(inside_color(doc, "#ffffff"), Some([0, 0, 255, 255]));
    app.run("ui.foldPreview", json!({"inside": ""})).unwrap();
    assert_eq!(app.ui.fold_preview.inside, "");
}

#[test]
fn the_window_draws_the_print_on_the_folded_panels_and_the_inside_on_their_backs() {
    let (mut app, _) = app();
    let mut window = Window::new(1.0, vec2(520.0, 440.0));
    // Flat, from the front: the base's print, red, fills the middle.
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
    let [r, g, b, a] = middle(&window.settle(&mut app).image);
    assert!(r > 150 && g < 40 && b < 40 && a == 255, "red print: {:?}", [r, g, b, a]);
    let state = app.run("ui.foldPreview", json!({})).unwrap();
    let img = app.addon_panels.fold_view.shown.clone().expect("a frame").image;
    assert_eq!((state["frame"]["width"].as_u64(), state["frame"]["height"].as_u64()), (Some(u64::from(img.width)), Some(u64::from(img.height))));
    // From the back, the base's inside shows in the inside color.
    app.run("ui.foldPreview", json!({"view": "back", "inside": "#00ff00"})).unwrap();
    let [r, g, b, _] = middle(&window.settle(&mut app).image);
    assert!(r < 40 && g > 150 && b < 40, "green inside: {:?}", [r, g, b]);
    // Folded and seen from the front, the walls stand edge on: the base fills the same place.
    app.run("ui.foldPreview", json!({"amount": 1, "view": "front"})).unwrap();
    let img = window.settle(&mut app).image;
    assert!(middle(&img)[0] > 150, "the base still shows its print: {:?}", middle(&img));
    // The same print is not rendered again while the document does not change.
    let before = app.addon_panels.fold_view.print.clone();
    window.frame(&mut app);
    window.frame(&mut app);
    let after = app.addon_panels.fold_view.print.clone();
    let same = matches!((&before, &after), (Some((k, p)), Some((k2, p2))) if k == k2 && Arc::ptr_eq(p, p2));
    assert!(same, "no new print without a change");
}

#[test]
fn a_wide_perspective_still_fits_the_whole_folded_blank_in_the_view() {
    let (mut app, _) = app();
    let mut window = Window::new(1.0, vec2(520.0, 440.0));
    // Without perspective, the base and the two walls behind it that face the viewer show.
    let mut flat = None;
    for perspective in [0, 60, 160] {
        app.run("ui.foldPreview", json!({"amount": 1, "view": "threeQuarter", "zoom": 1, "perspective": perspective})).unwrap();
        let img = window.settle(&mut app).image;
        let mut shown: Vec<usize> =
            (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter_map(|(x, y)| img.panel_at(x, y)).collect();
        shown.sort_unstable();
        shown.dedup();
        let flat = flat.get_or_insert_with(|| shown.clone());
        assert_eq!(flat.len(), 3);
        assert_eq!(&shown, flat, "{perspective}°: the same panels show");
        let edge = (0..img.width).flat_map(|x| [(x, 0), (x, img.height - 1)]).chain((0..img.height).flat_map(|y| [(0, y), (img.width - 1, y)]));
        assert!(edge.into_iter().all(|(x, y)| img.panel_at(x, y).is_none()), "{perspective}°: nothing reaches the edge of the view");
    }
}

#[test]
fn a_window_with_more_pixels_than_the_view_may_have_is_drawn_smaller() {
    let (mut app, _) = app();
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
    let img = Window::new(8.0, vec2(520.0, 440.0)).settle(&mut app).image;
    assert!(u64::from(img.width) * u64::from(img.height) <= u64::from(MAX_PIXELS), "{}×{}", img.width, img.height);
    assert!(img.width > 1000, "still sharper than one pixel a point: {}", img.width);
    assert!(middle(&img)[0] > 150, "the base's print shows: {:?}", middle(&img));
}

#[test]
fn a_short_wide_window_keeps_its_proportions() {
    let (mut app, _) = app();
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
    let shown = Window::new(2.0, vec2(2200.0, 260.0)).settle(&mut app);
    let (img, rect) = (&shown.image, shown.rect);
    let (image, window) = (f64::from(img.width) / f64::from(img.height), f64::from(rect.width() / rect.height()));
    assert!(img.width <= MAX_SIDE, "{}", img.width);
    assert!((image - window).abs() / window < 0.02, "image {image}, window {window}");
}

#[test]
fn the_print_leaves_out_the_fold_lines_unless_they_are_shown() {
    let (mut app, red) = app();
    // Without the red square, the base's middle shows the board with no art; with a crease drawn
    // across the middle and the fold lines shown, the crease's color shows there.
    app.run("select.set", json!({ "ids": [red] })).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    let mid = app.run("path.create", json!({ "d": "M140 190 L240 190" })).unwrap()["id"].as_u64().unwrap();
    app.run("addon.packaging.setRole", json!({ "role": "crease", "ids": [mid] })).unwrap();
    app.run("select.set", json!({ "ids": [mid] })).unwrap();
    app.run("stroke.set", json!({ "weight": 20 })).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front", "zoom": 1})).unwrap();
    let mut window = Window::new(1.0, vec2(520.0, 440.0));
    let plain = middle(&window.settle(&mut app).image);
    app.run("ui.foldPreview", json!({"showFoldLines": true})).unwrap();
    let lined = middle(&window.settle(&mut app).image);
    assert_ne!(plain, lined, "the crease shows only with Show Fold Lines on");
    assert!(plain[0] > 200 && plain[1] > 200 && plain[2] > 200, "the board is white where the print has no art: {plain:?}");
}

#[test]
fn the_settings_are_kept_with_the_ui_state() {
    let (mut app, _) = app();
    app.run("ui.foldPreview", json!({"amount": 0.25, "view": "left"})).unwrap();
    let saved: Value = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(saved["fold_preview"]["amount"], 0.25, "{}", saved["fold_preview"]);
    let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.fold_preview, app.ui.fold_preview);
}

#[test]
fn saved_settings_out_of_range_are_brought_into_range() {
    let (mut app, _) = app();
    app.ui.fold_preview = Settings {
        amount: 3.0,
        rotate_x: 200.0,
        rotate_y: 180.0,
        zoom: 0.0,
        pan: [f64::NAN, 4.0],
        perspective: -5.0,
        base: Some([f64::INFINITY, 0.0]),
        ..Settings::default()
    };
    let shown = drawn(&mut app);
    let set = &app.ui.fold_preview;
    assert_eq!(
        (set.amount, set.rotate_x, set.rotate_y, set.zoom, set.pan, set.perspective, set.base),
        (1.0, -160.0, 180.0, 0.1, [0.0, 4.0], 0.0, None)
    );
    assert!(shown.image.width > 0, "the blank folds and is drawn");
}

/// The app showing the tray flat from the front, in a [`Fixture`] 520 × 440 points big.
struct Rig {
    app: Rc<RefCell<VectorcraftApp>>,
    fx: Fixture,
}

impl Rig {
    fn front() -> Self {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let (mut app, _) = app();
        app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
        let app = Rc::new(RefCell::new(app));
        let shared = Rc::clone(&app);
        let mut fx = Fixture::new(move |ui| show(&mut shared.borrow_mut(), ui));
        fx.room(vec2(520.0, 440.0));
        let mut rig = Rig { app, fx };
        rig.settle();
        rig
    }

    /// Frames until the print of the document as it is has been rendered and drawn.
    fn settle(&mut self) {
        for _ in 0..SETTLE_FRAMES {
            self.fx.idle(1);
            if print_ready(&self.app.borrow()) {
                break;
            }
            wait_for_print(&self.app.borrow());
        }
        assert!(print_ready(&self.app.borrow()), "the print renders");
        self.fx.idle(3);
    }

    fn settings(&self) -> Settings {
        self.app.borrow().ui.fold_preview.clone()
    }

    /// The middle of the view's last image on screen.
    fn center(&self) -> Pos2 {
        self.app.borrow().addon_panels.fold_view.shown.as_ref().expect("the view is drawn").rect.center()
    }

    fn run(&mut self, command: &str, params: Value) {
        self.app.borrow_mut().run(command, params).unwrap();
    }

    /// Presses `button` at `at`, moves by `by` in four steps and lets go.
    fn drag_with(&mut self, button: PointerButton, at: Pos2, by: Vec2) {
        let mods = Modifiers::NONE;
        let event = |pos, pressed| Event::PointerButton { pos, button, pressed, modifiers: mods };
        self.fx.frame(vec![Event::PointerMoved(at)]);
        self.fx.frame(vec![event(at, true)]);
        for k in 1..=4 {
            self.fx.frame(vec![Event::PointerMoved(at + by * (k as f32 / 4.0))]);
        }
        self.fx.frame(vec![event(at + by, false)]);
        self.fx.idle(3);
    }

    /// A key event with no modifiers, as the keyboard reports a press or a release.
    fn key_event(key: Key, pressed: bool) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
    }
}

/// One way of working the view with the pointer, the wheel or the keys.
enum Gesture {
    Drag(Vec2),
    ShiftDrag(Vec2),
    MiddleDrag(Vec2),
    SpaceDrag(Vec2),
    Wheel(f32),
    Arrows(&'static [Key]),
}

#[test]
fn the_view_gestures_change_the_settings() {
    let rows: [(Gesture, fn(&Settings) -> bool, &str); 7] = [
        (
            Gesture::Drag(vec2(40.0, 20.0)),
            |s| s.rotate_y > 5.0 && s.rotate_y <= 20.0 + 1e-9 && s.rotate_x > 2.0 && s.rotate_x <= 10.0 + 1e-9,
            "a drag turns",
        ),
        (Gesture::ShiftDrag(vec2(40.0, 20.0)), |s| s.rotate_y > 5.0 && s.rotate_x == 0.0, "Shift keeps to the larger axis"),
        (
            Gesture::MiddleDrag(vec2(30.0, -10.0)),
            |s| s.pan[0] > 0.0 && s.pan[1] < 0.0 && (s.rotate_x, s.rotate_y) == (0.0, 0.0),
            "a middle drag moves and does not turn",
        ),
        (
            Gesture::SpaceDrag(vec2(30.0, -10.0)),
            |s| s.pan[0] > 0.0 && s.pan[1] < 0.0 && (s.rotate_x, s.rotate_y) == (0.0, 0.0),
            "a drag with Space held moves and does not turn",
        ),
        (Gesture::Wheel(60.0), |s| s.zoom > 1.05, "the wheel up zooms in"),
        (Gesture::Wheel(-60.0), |s| s.zoom < 0.95, "the wheel down zooms out"),
        (Gesture::Arrows(&[Key::ArrowUp, Key::ArrowRight, Key::ArrowRight]), |s| (s.rotate_x, s.rotate_y) == (-5.0, 10.0), "each press turns by 5°"),
    ];
    for (gesture, holds, why) in rows {
        let mut rig = Rig::front();
        let at = rig.center();
        match gesture {
            Gesture::Drag(by) => rig.drag_with(PointerButton::Primary, at, by),
            Gesture::ShiftDrag(by) => {
                rig.fx.hold(Modifiers::SHIFT);
                rig.fx.drag_at(at, by);
                rig.fx.hold(Modifiers::NONE);
            }
            Gesture::MiddleDrag(by) => rig.drag_with(PointerButton::Middle, at, by),
            Gesture::SpaceDrag(by) => {
                rig.fx.frame(vec![Rig::key_event(Key::Space, true)]);
                rig.drag_with(PointerButton::Primary, at, by);
                rig.fx.frame(vec![Rig::key_event(Key::Space, false)]);
            }
            Gesture::Wheel(dy) => {
                let wheel = Event::MouseWheel {
                    unit: craft::egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, dy),
                    phase: craft::egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                    source: craft::egui::MouseWheelSource::Unknown,
                };
                rig.fx.frame(vec![Event::PointerMoved(at)]);
                rig.fx.frame(vec![wheel]);
                rig.fx.idle(20);
            }
            Gesture::Arrows(arrows) => {
                rig.fx.click_at(at);
                let focused = rig.fx.focused();
                assert!(focused.is_some(), "a click gives the view the focus");
                for key in arrows {
                    rig.fx.key(*key, Modifiers::NONE);
                }
                assert_eq!(rig.fx.focused(), focused, "the arrows turn the view and leave the focus on it");
            }
        }
        assert!(holds(&rig.settings()), "{why}: {:?}", rig.settings());
    }
}

#[test]
fn a_double_click_on_a_panel_makes_it_the_base() {
    let mut rig = Rig::front();
    // The left wall is the base; the middle of the view shows the tray's base panel.
    rig.run("ui.foldPreview", json!({"base": [120, 190]}));
    rig.fx.idle(3);
    let at = rig.center();
    rig.fx.click_at(at);
    rig.fx.click_at(at);
    let [x, y] = rig.settings().base.expect("a base");
    assert!((140.0..240.0).contains(&x) && (140.0..240.0).contains(&y), "the base panel is picked: {x}, {y}");
}

#[test]
fn a_double_click_as_the_document_changes_picks_nothing_from_the_old_image() {
    let mut rig = Rig::front();
    rig.run("ui.foldPreview", json!({"base": [120, 190]}));
    rig.fx.idle(3);
    let at = rig.center();
    rig.fx.click_at(at);
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    rig.fx.frame(vec![press(true)]);
    // The document changes between the image on screen and the release that ends the double-click.
    rig.run("shape.rectangle", json!({"x": 350, "y": 350, "width": 10, "height": 10}));
    rig.fx.frame(vec![press(false)]);
    assert_eq!(rig.settings().base, Some([120.0, 190.0]), "the base is kept");
}

#[test]
fn the_view_is_folded_and_drawn_again_only_when_something_changes() {
    let mut rig = Rig::front();
    let folded = |rig: &Rig| rig.app.borrow().addon_panels.fold_view.folded.as_ref().map(|f| Arc::clone(&f.1)).expect("a fold");
    let image = |rig: &Rig| rig.app.borrow().addon_panels.fold_view.shown.as_ref().map(|s| Arc::clone(&s.image)).expect("an image");
    // Frames one at a time, each counted when it shows a newly drawn image: the store's upload count is of the frame
    // running, so it reads none between frames.
    let uploads = |rig: &mut Rig| {
        (0..5)
            .filter(|_| {
                let before = image(rig);
                rig.fx.frame(vec![]);
                !Arc::ptr_eq(&before, &image(rig))
            })
            .count()
    };
    let (fold, img) = (folded(&rig), image(&rig));
    assert_eq!(uploads(&mut rig), 0, "nothing changed: no render");
    assert!(Arc::ptr_eq(&fold, &folded(&rig)) && Arc::ptr_eq(&img, &image(&rig)), "and no new fold or image");
    rig.run("ui.foldPreview", json!({"rotateY": 10}));
    assert_eq!(uploads(&mut rig), 1, "a turn renders once");
    assert!(Arc::ptr_eq(&fold, &folded(&rig)), "and does not fold again");
    assert!(!Arc::ptr_eq(&img, &image(&rig)));
    rig.run("ui.foldPreview", json!({"amount": 0.5}));
    assert_eq!(uploads(&mut rig), 1, "a new amount renders once");
    assert!(!Arc::ptr_eq(&fold, &folded(&rig)), "and folds again");
}

#[test]
fn a_print_of_another_blank_or_setting_is_not_drawn_while_the_new_one_renders() {
    let key = PrintKey { uid: 1, revision: 5, fold_lines: false, region: [0, 0, 10, 10], width: 1, height: 1 };
    let region = vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0);
    let print = vectorcraft_three_d::fold::raster::Print::new(region, 1, 1, vec![255; 4]).expect("a print");
    let view = View { print: Some((key, Arc::new(print))), ..Default::default() };
    assert!(view.print_for(&PrintKey { revision: 6, ..key }).is_some(), "a newer revision of the same blank keeps the last print");
    for other in [PrintKey { uid: 2, ..key }, PrintKey { fold_lines: true, ..key }, PrintKey { region: [0, 0, 20, 10], ..key }] {
        assert!(view.print_for(&other).is_none(), "{other:?}");
    }
}

#[test]
fn the_status_line_reports_open_joints_crossing_panels_and_other_blanks() {
    use vectorcraft_three_d::fold::{Blank, Folded, FoldedHinge};
    let hinge = |tree, closes, gap| FoldedHinge { a: 0, b: 1, angle: 90.0, tree, closes, gap };
    let f = Folded {
        base: 0,
        panels: vec![],
        order: vec![],
        hinges: vec![hinge(true, true, 0.0), hinge(false, false, 12.34), hinge(false, false, 3.0), hinge(false, true, 0.05)],
        collisions: vec![(0, 1)],
    };
    // The summary, the fold line errors, the other blanks, the open joint, the crossing panels.
    assert_eq!(status(3, Some((&Blank::default(), &f)), true).len(), 5);
    let lines = status(3, Some((&Blank::default(), &f)), true);
    assert_eq!(lines[1], status(0, None, false)[0], "the error line comes after the summary");
    // A joint that closes and panels that do not cross add nothing.
    let f = Folded { hinges: vec![hinge(false, true, 0.05)], collisions: vec![], ..f };
    assert_eq!(status(1, Some((&Blank::default(), &f)), false).len(), 1);
    // With no fold there is the one line that says why.
    assert_eq!(status(0, None, false).len(), 1);
}
