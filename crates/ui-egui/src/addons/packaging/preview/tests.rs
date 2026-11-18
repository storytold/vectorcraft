use serde_json::{Value, json};
use vectorcraft_engine::Session;

use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};

use super::{Cache, ID, PrintKey, Settings, inside_color, last_image, last_rect, prints_rendered, show, status, work_done};
use crate::{VectorcraftApp, menus};

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

/// Draws the window in a 520 × 440 frame, twice (egui lays out on the first).
fn frame(app: &mut VectorcraftApp) {
    frame_at(app, 1.0);
}

/// [`frame`] at `zoom` pixels per point.
fn frame_at(app: &mut VectorcraftApp, zoom: f32) {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(zoom);
    crate::theme::install_fonts(&ctx);
    for time in [0.0, 1.0] {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(520.0, 440.0));
        let input = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
    }
}

fn color_at(img: &vectorcraft_three_d::fold::raster::Image, x: u32, y: u32) -> [u8; 4] {
    let i = (y * img.width + x) as usize * 4;
    [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2], img.pixels[i + 3]]
}

/// The color at the middle of the window's last image.
fn middle(img: &vectorcraft_three_d::fold::raster::Image) -> [u8; 4] {
    color_at(img, img.width / 2, img.height / 2)
}

#[test]
fn the_menu_item_and_the_panel_button_show_the_window_and_a_second_run_keeps_it() {
    let (mut app, _) = app();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Packaging Tools"] && e.label == "Fold Preview").expect("listed");
    assert_eq!((item.command.as_deref(), &item.params), (Some("ui.foldPreview"), &json!({})));
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, "ui.foldPreview", json!({}));
    assert!(crate::panel_docking::visible(&app.ui, ID), "the window opens");
    menus::invoke(&mut app, "ui.foldPreview", json!({}));
    assert!(crate::panel_docking::visible(&app.ui, ID), "a second run leaves it open");
    let text = crate::tests_labels::painted_text(&mut app, super::super::show);
    assert!(text.contains("Fold Preview"), "the Fold Lines panel has the button: {text}");
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
    for (p, want) in [
        (json!({"amount": 2}), "`amount` must be a number from 0 to 1"),
        (json!({"amount": "half"}), "`amount` must be a number from 0 to 1"),
        (json!({"view": "sideways"}), "`view` must be one of"),
        (json!({"rotateX": 200}), "`rotateX` must be a number from -180 to 180"),
        (json!({"zoom": 0}), "`zoom` must be a number from 0.1 to 8"),
        (json!({"perspective": 170}), "`perspective` must be a number from 0 to 160"),
        (json!({"inside": "no such swatch"}), "`inside` must be #rrggbb"),
        (json!({"inside": 7}), "`inside` must be #rrggbb"),
        (json!({"showFoldLines": "yes"}), "`showFoldLines` must be true or false"),
        (json!({"base": [1, "2"]}), "`base` must be a point"),
        (json!({"base": [5, 5]}), "`base` is in no panel"),
        (json!({"amount": 0.2, "zoom": 99}), "`zoom` must be a number"),
    ] {
        let e = app.run("ui.foldPreview", p.clone()).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
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
    // Flat, from the front: the base's print, red, fills the middle.
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
    let before = prints_rendered();
    frame(&mut app);
    assert!(prints_rendered() > before, "the print is rendered on this thread without a worker");
    let img = last_image().expect("a frame");
    let [r, g, b, a] = middle(&img);
    assert!(r > 150 && g < 40 && b < 40 && a == 255, "red print: {:?}", [r, g, b, a]);
    let state = app.run("ui.foldPreview", json!({})).unwrap();
    assert_eq!((state["frame"]["width"].as_u64(), state["frame"]["height"].as_u64()), (Some(u64::from(img.width)), Some(u64::from(img.height))));
    // From the back, the base's inside shows in the inside color.
    app.run("ui.foldPreview", json!({"view": "back", "inside": "#00ff00"})).unwrap();
    frame(&mut app);
    let [r, g, b, _] = middle(&last_image().expect("a frame"));
    assert!(r < 40 && g > 150 && b < 40, "green inside: {:?}", [r, g, b]);
    // Folded and seen from the front, the walls stand edge on: the base fills the same place.
    app.run("ui.foldPreview", json!({"amount": 1, "view": "front"})).unwrap();
    frame(&mut app);
    let img = last_image().expect("a frame");
    let [r, ..] = middle(&img);
    assert!(r > 150, "the base still shows its print: {:?}", middle(&img));
    // The same print is not rendered again while the document does not change.
    let prints = prints_rendered();
    frame(&mut app);
    assert_eq!(prints_rendered(), prints);
}

#[test]
fn a_wide_perspective_still_fits_the_whole_folded_blank_in_the_view() {
    let (mut app, _) = app();
    // Without perspective, the base and the two walls behind it that face the viewer show.
    let mut flat = None;
    for perspective in [0, 60, 160] {
        app.run("ui.foldPreview", json!({"amount": 1, "view": "threeQuarter", "zoom": 1, "perspective": perspective})).unwrap();
        frame(&mut app);
        let img = last_image().expect("a frame");
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
    frame_at(&mut app, 8.0);
    let img = last_image().expect("a frame");
    assert!(u64::from(img.width) * u64::from(img.height) <= u64::from(vectorcraft_three_d::fold::raster::MAX_PIXELS), "{}×{}", img.width, img.height);
    assert!(img.width > 1000, "still sharper than one pixel a point: {}", img.width);
    let [r, ..] = middle(&img);
    assert!(r > 150, "the base's print shows: {:?}", middle(&img));
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
    frame(&mut app);
    let plain = middle(&last_image().expect("a frame"));
    app.run("ui.foldPreview", json!({"showFoldLines": true})).unwrap();
    frame(&mut app);
    let lined = middle(&last_image().expect("a frame"));
    assert_ne!(plain, lined, "the crease shows only with Show Fold Lines on");
    assert!(plain[0] > 200 && plain[1] > 200 && plain[2] > 200, "the board is white where the print has no art: {plain:?}");
}

#[test]
fn the_window_says_when_there_is_nothing_to_fold() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, show);
    assert!(text.contains("Check the fold lines first."), "{text}");
    let mut none = VectorcraftApp::new(Session::new(), Default::default());
    let text = crate::tests_labels::painted_text(&mut none, show);
    assert!(text.contains("Open a document to preview its folded shape."), "{text}");
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
    frame(&mut app);
    let set = &app.ui.fold_preview;
    assert_eq!(
        (set.amount, set.rotate_x, set.rotate_y, set.zoom, set.pan, set.perspective, set.base),
        (1.0, -160.0, 180.0, 0.1, [0.0, 4.0], 0.0, None)
    );
    let text = crate::tests_labels::painted_text(&mut app, show);
    assert!(!text.contains("Check the fold lines first."), "the blank folds: {text}");
    assert!(last_image().is_some(), "and is drawn");
}

/// A window kept open over frames, as the app draws it.
struct View {
    ctx: egui::Context,
    time: f64,
}

impl View {
    fn new(app: &mut VectorcraftApp) -> Self {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut v = View { ctx, time: 0.0 };
        v.idle(app);
        v
    }

    fn step(&mut self, app: &mut VectorcraftApp, events: Vec<Event>, modifiers: Modifiers) {
        self.time += 0.05;
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(520.0, 440.0));
        let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { time: Some(self.time), screen_rect: Some(screen), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
    }

    fn idle(&mut self, app: &mut VectorcraftApp) {
        for _ in 0..3 {
            self.step(app, vec![], Modifiers::NONE);
        }
    }

    /// Presses `button` at `at`, drags by `by` in four steps and lets go, holding `modifiers`.
    fn drag(&mut self, app: &mut VectorcraftApp, at: Pos2, by: egui::Vec2, button: PointerButton, modifiers: Modifiers) {
        let press = |pos, pressed| Event::PointerButton { pos, button, pressed, modifiers };
        self.step(app, vec![Event::PointerMoved(at)], modifiers);
        self.step(app, vec![press(at, true)], modifiers);
        for k in 1..=4 {
            self.step(app, vec![Event::PointerMoved(at + by * (k as f32 / 4.0))], modifiers);
        }
        self.step(app, vec![press(at + by, false)], modifiers);
        self.idle(app);
    }

    fn click(&mut self, app: &mut VectorcraftApp, at: Pos2) {
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.step(app, vec![Event::PointerMoved(at)], Modifiers::NONE);
        self.step(app, vec![press(true)], Modifiers::NONE);
        self.step(app, vec![press(false)], Modifiers::NONE);
    }

    fn key(&mut self, app: &mut VectorcraftApp, key: egui::Key) {
        let press = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        self.step(app, vec![press], Modifiers::NONE);
        self.step(app, vec![], Modifiers::NONE);
    }
}

/// The app showing the tray flat from the front, in a [`View`], and the middle of the view.
fn front_view() -> (VectorcraftApp, View, Pos2) {
    let (mut app, _) = app();
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
    let view = View::new(&mut app);
    let at = last_rect().expect("the view is drawn").center();
    (app, view, at)
}

#[test]
fn a_drag_turns_the_view_and_a_shift_drag_keeps_to_one_axis() {
    let (mut app, mut view, at) = front_view();
    view.drag(&mut app, at, vec2(40.0, 20.0), PointerButton::Primary, Modifiers::NONE);
    let set = app.ui.fold_preview.clone();
    assert!(set.rotate_y > 5.0 && set.rotate_y <= 20.0 + 1e-9, "{set:?}");
    assert!(set.rotate_x > 2.0 && set.rotate_x <= 10.0 + 1e-9, "{set:?}");
    app.run("ui.foldPreview", json!({"view": "front"})).unwrap();
    view.drag(&mut app, at, vec2(40.0, 20.0), PointerButton::Primary, Modifiers::SHIFT);
    let set = app.ui.fold_preview.clone();
    assert!(set.rotate_y > 5.0 && set.rotate_x == 0.0, "only the larger move turns the view: {set:?}");
}

#[test]
fn a_middle_drag_moves_the_view_and_the_wheel_zooms_it() {
    let (mut app, mut view, at) = front_view();
    view.drag(&mut app, at, vec2(30.0, -10.0), PointerButton::Middle, Modifiers::NONE);
    let set = app.ui.fold_preview.clone();
    assert!(set.pan[0] > 0.0 && set.pan[1] < 0.0, "{set:?}");
    assert_eq!((set.rotate_x, set.rotate_y), (0.0, 0.0), "the view does not turn");
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, 60.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    };
    view.step(&mut app, vec![Event::PointerMoved(at), wheel], Modifiers::NONE);
    for _ in 0..20 {
        view.step(&mut app, vec![], Modifiers::NONE);
    }
    assert!(app.ui.fold_preview.zoom > 1.05, "{:?}", app.ui.fold_preview);
}

#[test]
fn the_arrow_keys_turn_the_focused_view_and_keep_the_focus() {
    let (mut app, mut view, at) = front_view();
    view.click(&mut app, at);
    view.idle(&mut app);
    // Up first: the toolbar is above the view, where the focus would go if the view let it.
    view.key(&mut app, egui::Key::ArrowUp);
    view.key(&mut app, egui::Key::ArrowRight);
    view.key(&mut app, egui::Key::ArrowRight);
    let set = &app.ui.fold_preview;
    assert_eq!((set.rotate_x, set.rotate_y), (-5.0, 10.0), "every press turns the view by 5°");
    // A focused view seen from the back stays there until a key turns it.
    app.run("ui.foldPreview", json!({"view": "back"})).unwrap();
    view.idle(&mut app);
    assert_eq!(app.ui.fold_preview.rotate_y, 180.0);
}

#[test]
fn a_double_click_on_a_panel_makes_it_the_base() {
    let (mut app, mut view, at) = front_view();
    // The left wall is the base; the middle of the view shows the tray's base panel.
    app.run("ui.foldPreview", json!({"base": [120, 190]})).unwrap();
    view.idle(&mut app);
    view.click(&mut app, at);
    view.click(&mut app, at);
    let [x, y] = app.ui.fold_preview.base.expect("a base");
    assert!((140.0..240.0).contains(&x) && (140.0..240.0).contains(&y), "the base panel is picked: {x}, {y}");
}

#[test]
fn a_double_click_as_the_document_changes_picks_nothing_from_the_old_image() {
    let (mut app, mut view, at) = front_view();
    app.run("ui.foldPreview", json!({"base": [120, 190]})).unwrap();
    view.idle(&mut app);
    view.click(&mut app, at);
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    view.step(&mut app, vec![press(true)], Modifiers::NONE);
    // The document changes between the image on screen and the release that ends the double-click.
    app.run("shape.rectangle", json!({"x": 350, "y": 350, "width": 10, "height": 10})).unwrap();
    view.step(&mut app, vec![press(false)], Modifiers::NONE);
    assert_eq!(app.ui.fold_preview.base, Some([120.0, 190.0]), "the base is kept");
}

#[test]
fn the_view_is_folded_and_drawn_again_only_when_something_changes() {
    let (mut app, mut view, _) = front_view();
    let (folds, renders) = work_done();
    view.idle(&mut app);
    assert_eq!(work_done(), (folds, renders), "nothing changed: no fold and no render");
    app.run("ui.foldPreview", json!({"rotateY": 10})).unwrap();
    view.idle(&mut app);
    assert_eq!(work_done(), (folds, renders + 1), "a turn renders once and does not fold again");
    app.run("ui.foldPreview", json!({"amount": 0.5})).unwrap();
    view.idle(&mut app);
    assert_eq!(work_done(), (folds + 1, renders + 2), "a new amount folds and renders once");
}

#[test]
fn a_short_wide_window_keeps_its_proportions() {
    let (mut app, _) = app();
    app.run("ui.foldPreview", json!({"amount": 0, "view": "front"})).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(2.0);
    crate::theme::install_fonts(&ctx);
    for time in [0.0, 1.0] {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(2200.0, 260.0));
        let input = egui::RawInput { time: Some(time), screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(&mut app, ui));
        out.textures_delta.clear();
    }
    let (img, rect) = (last_image().expect("a frame"), last_rect().expect("a rect"));
    let (image, window) = (f64::from(img.width) / f64::from(img.height), f64::from(rect.width() / rect.height()));
    assert!(img.width <= vectorcraft_three_d::fold::raster::MAX_SIDE, "{}", img.width);
    assert!((image - window).abs() / window < 0.02, "image {image}, window {window}");
}

#[test]
fn a_print_of_another_blank_or_setting_is_not_drawn_while_the_new_one_renders() {
    let key = PrintKey { uid: 1, revision: 5, fold_lines: false, region: [0, 0, 10, 10], width: 1, height: 1 };
    let region = vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0);
    let print = vectorcraft_three_d::fold::raster::Print::new(region, 1, 1, vec![255; 4]).expect("a print");
    let cache = Cache { print: Some((key, Arc::new(print))), ..Default::default() };
    assert!(cache.print_for(&PrintKey { revision: 6, ..key }).is_some(), "a newer revision of the same blank keeps the last print");
    for other in [PrintKey { uid: 2, ..key }, PrintKey { fold_lines: true, ..key }, PrintKey { region: [0, 0, 20, 10], ..key }] {
        assert!(cache.print_for(&other).is_none(), "{other:?}");
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
    let lines = status(3, Some((&Blank::default(), &f)), true);
    assert_eq!(
        lines,
        [
            "Panels: 0, hinges: 0",
            "Check the fold lines first.",
            "Other blanks not shown: 2",
            "A joint does not close by 12.3 pt.",
            "Two panels pass through each other."
        ]
    );
    let f = Folded { hinges: vec![hinge(false, true, 0.05)], collisions: vec![], ..f };
    assert_eq!(status(1, Some((&Blank::default(), &f)), false), ["Panels: 0, hinges: 0"]);
}

#[test]
fn the_window_says_when_folded_panels_pass_through_each_other() {
    // A strip: a 120 pt lid, a 30 pt wall, the 100 pt base and a 60 pt wall, all 50 pt deep. Folded
    // about the base, the lid crosses the taller wall.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let path = |app: &mut VectorcraftApp, d: &str| app.run("path.create", json!({ "d": d })).unwrap()["id"].as_u64().unwrap();
    let outline = path(&mut app, "M0 0 L310 0 L310 50 L0 50 Z");
    let creases = [path(&mut app, "M120 0 L120 50"), path(&mut app, "M150 0 L150 50"), path(&mut app, "M250 0 L250 50")];
    app.run("addon.packaging.setRole", json!({ "role": "cut", "ids": [outline] })).unwrap();
    app.run("addon.packaging.setRole", json!({ "role": "crease", "ids": creases })).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("ui.foldPreview", json!({"base": [200, 25], "amount": 1})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, show);
    assert!(text.contains("Two panels pass through each other."), "{text}");
    assert!(!text.contains("A joint does not close"), "{text}");
}
