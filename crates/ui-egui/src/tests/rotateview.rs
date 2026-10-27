//! View › Rotate View and `view.rotate`: the view turns to one of the status bar's preset angles
//! from the menu, or to any angle from the command, and the menu checks the angle the view is turned to.
//! View › Rotate View to Selection (`view.rotateToSelection`) turns the view to the angle of the
//! selection's bounding box.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, menus};

/// These are the status bar's angles and their labels, in the status bar's order.
const ANGLES: [(&str, f64); 12] = [
    ("0°", 0.0),
    ("15°", 15.0),
    ("30°", 30.0),
    ("45°", 45.0),
    ("60°", 60.0),
    ("90°", 90.0),
    ("180°", 180.0),
    ("-15°", -15.0),
    ("-30°", -30.0),
    ("-45°", -45.0),
    ("-60°", -60.0),
    ("-90°", -90.0),
];

fn with_document() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// Returns the labels of the View › Rotate View items that are checked.
fn checked(app: &VectorcraftApp) -> Vec<&'static str> {
    ANGLES.iter().filter(|(_, a)| menus::checked(app, "view.rotate", &json!({ "angle": a })) == Some(true)).map(|(l, _)| *l).collect()
}

#[test]
fn view_rotate_turns_the_view_to_any_angle() {
    let mut app = with_document();
    let center = app.view().unwrap().center;
    assert_eq!(app.run("view.rotate", json!({"angle": 30})).unwrap(), json!({"angle": 30.0}));
    let v = app.view().unwrap();
    assert_eq!((v.rotation, v.center), (30.0, center), "it turns about the center of the canvas");
    // Angles are normalized to (-180, 180], as a Rotate View drag normalizes them.
    for (angle, want) in [(390.0, 30.0), (-190.0, 170.0), (-180.0, 180.0), (12.5, 12.5)] {
        assert_eq!(app.run("view.rotate", json!({ "angle": angle })).unwrap(), json!({ "angle": want }), "{angle}");
        assert_eq!(app.view().unwrap().rotation, want, "{angle}");
    }
    for p in [json!({"angle": "x"}), json!({}), json!({"angle": null})] {
        assert_eq!(app.run("view.rotate", p.clone()).unwrap_err(), "`angle` must be a number", "{p}");
    }
    assert_eq!(app.view().unwrap().rotation, 12.5, "an error leaves the view as it was");
    let mut empty = VectorcraftApp::new(Session::new(), crate::Services::default());
    assert_eq!(empty.run("view.rotate", json!({"angle": 30})).unwrap_err(), "no document");
}

#[test]
fn the_view_menu_lists_the_status_bar_angles_and_checks_the_current_one() {
    let mut app = with_document();
    let entries = menus::menu_entries(&app);
    let at = entries.iter().position(|e| e.path == ["View", "Rotate View"]).expect("View › Rotate View");
    let (items, after) = entries[at..].split_at(ANGLES.len());
    let shown: Vec<_> = items.iter().map(|e| (e.path.join(" › "), e.label.as_str(), e.command.as_deref(), e.params.clone(), e.enabled)).collect();
    let want: Vec<_> = ANGLES.iter().map(|(l, a)| ("View › Rotate View".to_string(), *l, Some("view.rotate"), json!({ "angle": a }), true)).collect();
    assert_eq!(shown, want);
    let next: Vec<_> = after[..2].iter().map(|e| (e.path.join(" › "), e.label.as_str(), e.command.as_deref())).collect();
    let want = [
        ("View".to_string(), "Rotate View to Selection", Some("view.rotateToSelection")),
        ("View".to_string(), "Reset Rotate View", Some("view.rotateReset")),
    ];
    assert_eq!(next, want, "the submenu comes right before them");

    app.run("view.rotate", json!({"angle": 30})).unwrap();
    assert_eq!(checked(&app), ["30°"]);
    app.run("view.rotate", json!({"angle": 17})).unwrap();
    assert!(checked(&app).is_empty());
    // Every item stays a check item, unchecked, while the view is at another angle.
    assert!(ANGLES.iter().all(|(_, a)| menus::checked(&app, "view.rotate", &json!({ "angle": a })) == Some(false)));
    // A click on 180° runs its command.
    let item = &items[6];
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &id, p);
    assert_eq!(app.view().unwrap().rotation, 180.0);
    assert_eq!(checked(&app), ["180°"]);
}

/// Rotate View with no angle, as the command palette runs it, opens a dialog with the view's angle
/// filled in, and OK turns the view to the angle entered.
#[test]
fn rotate_view_without_an_angle_asks_for_one() {
    assert!(crate::palette::items().iter().any(|(label, id, _)| (label.as_str(), id.as_str()) == ("Rotate View", "view.rotate")));
    let mut app = with_document();
    app.run("view.rotate", json!({"angle": 30})).unwrap();
    menus::invoke(&mut app, "view.rotate", json!({}));
    let mut d = app.ui.dialog.clone().expect("Rotate View dialog");
    assert_eq!((d.kind.as_str(), d.str("__command"), d.str("__label")), ("command", "view.rotate".into(), "Rotate View".into()));
    assert_eq!(d.fields.get("angle"), Some(&json!(30.0)));
    d.fields.insert("angle".into(), json!(-45));
    app.ui.dialog = Some(d);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.view().unwrap().rotation, -45.0);
    // Without a document no dialog opens, and the status bar shows "no document".
    let mut empty = VectorcraftApp::new(Session::new(), crate::Services::default());
    menus::invoke(&mut empty, "view.rotate", json!({}));
    assert!(empty.ui.dialog.is_none());
    assert_eq!(empty.ui.status, "no document");
}

#[test]
fn rotate_view_needs_a_document() {
    let empty = VectorcraftApp::new(Session::new(), crate::Services::default());
    assert!(!menus::enabled(&empty, "view.rotate"));
    assert_eq!(menus::checked(&empty, "view.rotate", &json!({"angle": 0})), Some(false), "a check item without a document");
    assert!(menus::enabled(&with_document(), "view.rotate"));
}

/// Selects the object `id` alone and turns it by `angle` degrees.
fn turn(app: &mut VectorcraftApp, id: &Value, angle: f64) {
    app.run("select.set", json!({ "ids": [id] })).unwrap();
    app.run("object.rotate", json!({ "angle": angle })).unwrap();
}

/// Draws a 120 × 60 pt rectangle at `x` and returns its id.
fn rectangle(app: &mut VectorcraftApp, x: f64) -> Value {
    app.run("shape.rectangle", json!({"x": x, "y": 80, "width": 120, "height": 60})).unwrap()["id"].clone()
}

/// Says whether the sides of the selection's bounding box run across and down the window, as the
/// canvas draws the box in the active view.
fn square_on_screen(app: &VectorcraftApp) -> bool {
    let st = app.session.active().unwrap();
    let corners = st.doc.oriented_bounds(&st.selection.objects, false).unwrap().corners();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let to_screen = crate::canvas::Xf::new(rect, app.view().unwrap()).affine();
    let [a, b, c, _] = corners.map(|p| to_screen * p);
    let (top, side) = (b - a, c - b);
    top.x.abs() > 1.0 && top.y.abs() < 1e-6 && side.x.abs() < 1e-6 && side.y.abs() > 1.0
}

/// Runs Rotate View to Selection and returns the angle it reports, after checking that the view is
/// turned to that angle and that it is the selection's angle `document.inspect` reports.
fn rotate_to_selection(app: &mut VectorcraftApp) -> f64 {
    let shared = app.run("document.inspect", json!({})).unwrap()["selectionRotation"].as_f64().unwrap();
    let angle = app.run("view.rotateToSelection", json!({})).unwrap()["angle"].as_f64().unwrap();
    assert_eq!((angle, app.view().unwrap().rotation), (shared, shared));
    angle
}

#[test]
fn rotate_view_to_selection_squares_the_view_to_the_selected_object() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let mut app = with_document();
    let id = rectangle(&mut app, 40.0);
    turn(&mut app, &id, 30.0);
    let (zoom, center) = (app.view().unwrap().zoom, app.view().unwrap().center);
    assert!(!square_on_screen(&app), "the box stands at 30° before the view turns");
    let angle = rotate_to_selection(&mut app);
    assert!(near(angle, 30.0), "{angle}");
    let v = app.view().unwrap();
    assert_eq!((v.zoom, v.center), (zoom, center), "the view turns about the center of the canvas");
    assert!(square_on_screen(&app));

    // Point type turned the other way squares the view the same way.
    let text = app.run("text.create", json!({"x": 300, "y": 300, "text": "Turned"})).unwrap()["id"].clone();
    turn(&mut app, &text, -50.0);
    let angle = rotate_to_selection(&mut app);
    assert!(near(angle, -50.0), "{angle}");
    assert!(square_on_screen(&app));

    // Objects turned to different angles share no angle: the view turns back to 0.
    let other = rectangle(&mut app, 400.0);
    turn(&mut app, &other, 45.0);
    app.run("select.set", json!({ "ids": [id, other] })).unwrap();
    assert_eq!(rotate_to_selection(&mut app), 0.0);
    assert!(square_on_screen(&app));
}

#[test]
fn rotate_view_to_selection_needs_a_selected_object() {
    let mut app = with_document();
    let id = rectangle(&mut app, 40.0);
    turn(&mut app, &id, 30.0);
    assert!(menus::enabled(&app, "view.rotateToSelection"));
    app.run("select.none", json!({})).unwrap();
    assert!(!menus::enabled(&app, "view.rotateToSelection"));
    app.run("view.rotate", json!({"angle": 15})).unwrap();
    assert_eq!(app.run("view.rotateToSelection", json!({})).unwrap_err(), "nothing selected");
    assert_eq!(app.view().unwrap().rotation, 15.0, "an error leaves the view as it was");

    let mut empty = VectorcraftApp::new(Session::new(), crate::Services::default());
    assert!(!menus::enabled(&empty, "view.rotateToSelection"));
    assert_eq!(empty.run("view.rotateToSelection", json!({})).unwrap_err(), "no document");
    // The command palette lists it.
    let listed = crate::palette::items().into_iter().any(|(label, id, _)| label == "Rotate View to Selection" && id == "view.rotateToSelection");
    assert!(listed);
}

/// The strings the catalogs cover include the submenu's name and leave out the angle labels, as they
/// leave out Type › Size's point sizes.
#[test]
fn only_the_submenu_name_needs_translating() {
    let strings = menus::menu_strings();
    assert!(strings.contains("Rotate View"));
    for (label, _) in ANGLES {
        assert!(!strings.contains(label), "{label}");
    }
}

/// The window, with a 400 × 300 document whose view is turned to 30°, and the Selection tool. Each
/// frame comes the given number of seconds, zero or more, after the one before it.
struct Window {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        let mut w = Self { app: with_document(), ctx: egui::Context::default(), time: 0.0 };
        w.app.select_tool("selection");
        w.app.run("view.rotate", json!({"angle": 30})).unwrap();
        w.after(0.0, vec![]);
        w.after(0.0, vec![]);
        w
    }

    /// One headless frame of the whole window, `seconds` after the last one, with `events`.
    fn after(&mut self, seconds: f64, events: Vec<egui::Event>) {
        self.time += seconds;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let raw = egui::RawInput { screen_rect: Some(rect), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }

    /// A press and release of Escape, `seconds` after the last frame.
    fn escape(&mut self, seconds: f64) {
        self.after(seconds, key(egui::Key::Escape, false));
    }

    /// A press of `button` at `pos` and its release, a frame each, `seconds` after the last frame.
    fn click(&mut self, seconds: f64, button: egui::PointerButton, pos: egui::Pos2) {
        let press = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
        self.after(seconds, vec![egui::Event::PointerMoved(pos), press(true)]);
        self.after(0.0, vec![press(false)]);
    }

    fn rotation(&self) -> f64 {
        self.app.view().unwrap().rotation
    }
}

/// A press and release of `key`. With `repeat`, the press is a key repeat.
fn key(key: egui::Key, repeat: bool) -> Vec<egui::Event> {
    [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat, modifiers: Default::default() }).to_vec()
}

/// The second of two Escape presses less than a second apart resets a rotated view. A press of
/// another key or a pointer button between them, or a longer wait, ends the pair. Shift does not.
#[test]
fn escape_pressed_twice_resets_a_rotated_view() {
    let mut w = Window::new();
    w.escape(0.0);
    assert_eq!(w.rotation(), 30.0, "one Escape");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "a second Escape 0.5 s later");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "a third Escape leaves the view at 0°");

    let mut w = Window::new();
    let empty = w.app.canvas_rect.unwrap().left_top() + egui::vec2(20.0, 20.0);
    w.escape(0.0);
    w.click(0.2, egui::PointerButton::Primary, empty);
    w.escape(0.2);
    assert_eq!(w.rotation(), 30.0, "a click between the two");
    let mut w = Window::new();
    w.escape(0.0);
    w.after(0.2, key(egui::Key::ArrowRight, false));
    w.escape(0.2);
    assert_eq!(w.rotation(), 30.0, "an arrow key between the two");
    let mut w = Window::new();
    w.escape(0.0);
    w.after(0.2, key(egui::Key::ShiftLeft, false));
    w.escape(0.2);
    assert_eq!(w.rotation(), 0.0, "a Shift press between the two");

    let mut w = Window::new();
    w.escape(0.0);
    w.escape(2.0);
    assert_eq!(w.rotation(), 30.0, "two Escapes 2 s apart");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "the later of them starts a pair");

    // A held Escape repeats: the repeats are not a second press.
    let mut w = Window::new();
    w.after(0.0, vec![key(egui::Key::Escape, false)[0].clone()]);
    for _ in 0..5 {
        w.after(0.1, vec![key(egui::Key::Escape, true)[0].clone()]);
    }
    w.after(0.1, vec![key(egui::Key::Escape, false)[1].clone()]);
    assert_eq!(w.rotation(), 30.0, "Escape held down");
}

/// The first Escape of a pair may end something, such as isolation mode. With the Rotate View
/// tool, one Escape resets the view.
#[test]
fn escape_after_ending_isolation_or_with_the_rotate_view_tool_resets_the_view() {
    let mut w = Window::new();
    w.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    w.app.run("shape.rectangle", json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap();
    w.app.run("select.all", json!({})).unwrap();
    w.app.run("object.group", json!({})).unwrap();
    menus::invoke(&mut w.app, "object.isolate", json!({}));
    let isolated = |w: &Window| w.app.session.active().unwrap().isolation.is_some();
    assert!(isolated(&w));
    w.escape(0.0);
    assert_eq!((isolated(&w), w.rotation()), (false, 30.0), "the first Escape leaves isolation mode");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "the second resets the view");

    let mut w = Window::new();
    w.app.select_tool("rotateView");
    w.escape(0.0);
    assert_eq!(w.rotation(), 0.0, "one Escape with the Rotate View tool");
}

/// An Escape that closes a popup (the status bar's angle menu, the canvas context menu) does not
/// count, so the Escape after it leaves the view as it is. An Escape while the ruler origin is
/// dragged does not reset the view either, but it counts as the first Escape of a pair.
#[test]
fn escape_after_closing_a_popup_or_during_a_ruler_origin_drag_keeps_the_rotation() {
    let mut w = Window::new();
    // The menu's button has the id `ComboBox::from_id_salt("rot-combo")` gives it in the status bar's
    // row, which hosts other widgets too. egui 0.37 salts the Ui's stable `scope_id`, not the
    // widget rect's unstable `parent_id`; in debug builds the id's debug source names its salt.
    let widgets: Vec<egui::WidgetRect> = w.ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, ws)| ws.iter().copied()).collect());
    let combo = widgets.iter().find(|r| format!("{:?}", r.id).contains("rot-combo")).copied();
    let combo = combo.expect("the status bar's angle menu");
    w.click(0.0, egui::PointerButton::Primary, combo.rect.center());
    w.after(0.0, vec![]);
    assert!(egui::ComboBox::is_open(&w.ctx, combo.id), "the angle menu is open");
    w.escape(0.2);
    assert!(!egui::Popup::is_any_open(&w.ctx), "Escape closes it");
    w.escape(0.2);
    assert_eq!(w.rotation(), 30.0, "an Escape after the angle menu closed");

    let mut w = Window::new();
    w.click(0.0, egui::PointerButton::Secondary, w.app.canvas_rect.unwrap().center());
    w.after(0.0, vec![]);
    assert!(egui::Popup::is_any_open(&w.ctx), "a right click opens the context menu");
    w.escape(0.2);
    assert!(!egui::Popup::is_any_open(&w.ctx), "Escape closes it");
    w.escape(0.2);
    assert_eq!(w.rotation(), 30.0, "an Escape after the context menu closed");
    // With the Rotate View tool, the Escape that closes the menu leaves the view as it is too.
    let mut w = Window::new();
    w.app.select_tool("rotateView");
    w.click(0.0, egui::PointerButton::Secondary, w.app.canvas_rect.unwrap().center());
    w.after(0.0, vec![]);
    assert!(egui::Popup::is_any_open(&w.ctx), "a right click opens the context menu");
    w.escape(0.2);
    assert_eq!(w.rotation(), 30.0, "the Escape that closes the context menu");
    w.escape(0.2);
    assert_eq!(w.rotation(), 0.0, "the next Escape with the Rotate View tool");

    let mut w = Window::new();
    w.app.ui.view.rulers = true;
    w.after(0.0, vec![]);
    let corner = w.ctx.read_response(egui::Id::unique("ruler-corner")).expect("the rulers' corner").rect.center();
    let to = w.app.canvas_rect.unwrap().center();
    w.after(0.0, vec![egui::Event::PointerMoved(corner)]);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    w.after(0.0, vec![button(corner, true)]);
    w.after(0.1, vec![egui::Event::PointerMoved(to)]);
    assert!(crate::canvas::ruler_origin_dragging(&w.ctx));
    w.escape(0.2);
    w.escape(0.2);
    assert_eq!(w.rotation(), 30.0, "two Escapes while the ruler origin is dragged");
    w.after(0.1, vec![button(to, false)]);
    w.escape(0.2);
    assert_eq!(w.rotation(), 0.0, "an Escape after the drag ends pairs with the last one");
}
