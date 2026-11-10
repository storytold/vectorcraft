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

/// Send a control request and keep its success/error value for protocol assertions.
fn control_response(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _) = crate::control::ControlRequest::new(method, params);
    let crate::control::Outcome::Done(reply) = crate::control::handle(app, ctx, &request) else { panic!("not done") };
    reply
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

/// MCP's `pointer_gesture` reaches `ui.pointer`: view tools must turn the canvas there too,
/// rather than silently sending the gesture to the engine's no-op tool.
#[test]
fn control_rotate_view_gesture_turns_the_canvas_and_reset_restores_it() {
    let mut app = with_document();
    let ctx = egui::Context::default();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let view = app.view_mut().unwrap();
    view.center = vectorcraft_geom::Point::new(200.0, 150.0);
    view.zoom = 1.0;
    let revision = app.session.active().unwrap().revision;
    let send = super::synthetic::control;
    send(&mut app, &ctx, "ui.tool.select", json!({"tool": "rotateView"}));
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 300, "y": 150}]}));
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();
    send(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({"events": [
            {"kind": "drag", "x": 200, "y": 250},
            {"kind": "up", "x": 200, "y": 250}
        ]}),
    );
    let view = app.view().unwrap();
    assert!((view.rotation - 90.0).abs() < 1e-3, "{}", view.rotation);
    assert_eq!(view.center, vectorcraft_geom::Point::new(200.0, 150.0));
    assert_eq!(view.zoom, 1.0);
    assert_eq!(app.session.active().unwrap().revision, revision, "navigation does not edit the document");
    let reset = control_response(&mut app, &ctx, "engine.execute", json!({"command": "view.rotateReset", "params": {}}));
    assert_eq!(reset["ok"], true);
    assert!(reset["result"].is_null());
    assert_eq!(app.view().unwrap().rotation, 0.0);
}

/// Repeated document points stay fixed across requests, including Shift snapping from a turned view.
#[test]
fn control_rotate_view_keeps_press_coordinates_and_shift_snapping_across_requests() {
    let mut app = with_document();
    let ctx = egui::Context::default();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let view = app.view_mut().unwrap();
    view.center = vectorcraft_geom::Point::new(200.0, 150.0);
    view.zoom = 1.0;
    view.rotation = 30.0;
    let send = super::synthetic::control;
    send(&mut app, &ctx, "ui.tool.select", json!({"tool": "rotateView"}));
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 300, "y": 150}]}));
    let angle = 20_f64.to_radians();
    let (x, y) = (200.0 + 100.0 * angle.cos(), 150.0 + 100.0 * angle.sin());
    for _ in 0..2 {
        send(&mut app, &ctx, "ui.pointer", json!({"mods": {"shift": true}, "events": [{"kind": "drag", "x": x, "y": y}]}));
        assert_eq!(app.view().unwrap().rotation, 45.0, "the same document point does not turn again as the view changes");
    }
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "up", "x": x, "y": y}]}));
    assert_eq!(app.view().unwrap().rotation, 45.0);
}

/// Invalid coordinates and document changes cannot turn a different view.
#[test]
fn control_rotate_view_rejects_nonfinite_positions_and_a_changed_document() {
    let mut app = with_document();
    let ctx = egui::Context::default();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let send = super::synthetic::control;
    send(&mut app, &ctx, "ui.tool.select", json!({"tool": "rotateView"}));
    let (req, _) = crate::control::ControlRequest::new("ui.pointer", json!({"events": [{"kind": "down", "x": 1e300, "y": 0}]}));
    let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(reply["ok"], false);
    assert_eq!(app.view().unwrap().rotation, 0.0);
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 300, "y": 150}]}));
    app.run("file.new", json!({})).unwrap();
    let (req, _) = crate::control::ControlRequest::new("ui.pointer", json!({"events": [{"kind": "drag", "x": 200, "y": 250}]}));
    let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
    assert_eq!(reply["ok"], false);
    assert_eq!(app.view().unwrap().rotation, 0.0);
}

/// An invalid packet leaves the angle unchanged and requires a fresh press, including errors
/// after an otherwise valid drag and malformed releases or hovers.
#[test]
fn control_rotate_view_rejects_malformed_packets_without_leaving_a_press() {
    let drag = json!({"kind": "drag", "x": 200, "y": 250});
    for packet in [
        json!({"events": [drag.clone(), {"kind": "drag", "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "drag", "x": "200", "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "drag", "x": 200, "y": 250, "mods": {"shift": "yes"}}]}),
        json!({"mods": "shift", "events": [drag.clone()]}),
        json!({"events": [drag.clone(), {"kind": "drag", "x": 200, "y": 250, "mods": {"unknown": true}}]}),
        json!({"events": [drag.clone(), {"kind": "drag", "x": 1e300, "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "up", "x": 1e300, "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "move", "x": 1e300, "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "doubleclick", "x": 1e300, "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "wiggle", "x": 200, "y": 250}]}),
        json!({"events": [drag.clone(), {"kind": "drag", "x": 200, "y": 250, "space": "window"}]}),
        json!({"events": null}),
    ] {
        let mut app = with_document();
        let ctx = egui::Context::default();
        app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let view = app.view_mut().unwrap();
        view.center = vectorcraft_geom::Point::new(200.0, 150.0);
        view.zoom = 1.0;
        super::synthetic::control(&mut app, &ctx, "ui.tool.select", json!({"tool": "rotateView"}));
        super::synthetic::control(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 300, "y": 150}]}));
        let reply = control_response(&mut app, &ctx, "ui.pointer", packet.clone());
        assert_eq!(reply["ok"], false, "{packet}: {reply}");
        assert_eq!(app.view().unwrap().rotation, 0.0, "{packet}");
        let resume = control_response(&mut app, &ctx, "ui.pointer", json!({"events": [drag.clone()]}));
        assert_eq!(resume["ok"], false, "a failed packet cancels its press: {packet}");
        assert_eq!(app.view().unwrap().rotation, 0.0);
    }
}

/// Screen-space gestures survive frames, stop on release or a tool change, and start afresh
/// from the current angle. A press at the pivot uses the same finite convention as the mouse.
#[test]
fn control_rotate_view_screen_events_require_a_live_press() {
    let mut app = with_document();
    let ctx = egui::Context::default();
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let view = app.view_mut().unwrap();
    view.center = vectorcraft_geom::Point::new(1200.0, 900.0);
    view.zoom = 2.0;
    let send = super::synthetic::control;
    send(&mut app, &ctx, "ui.tool.select", json!({"tool": "rotateView"}));
    let down = json!({"events": [{"kind": "down", "x": 500, "y": 300, "space": "screen"}]});
    let drag = json!({"events": [{"kind": "drag", "x": 400, "y": 400, "space": "screen"}]});
    assert_eq!(control_response(&mut app, &ctx, "ui.pointer", drag.clone())["ok"], false);
    send(&mut app, &ctx, "ui.pointer", down.clone());
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();
    send(&mut app, &ctx, "ui.pointer", drag.clone());
    assert!((app.view().unwrap().rotation - 90.0).abs() < 1e-3);
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "up", "x": 400, "y": 400, "space": "screen"}]}));
    assert_eq!(control_response(&mut app, &ctx, "ui.pointer", drag.clone())["ok"], false, "release ends the press");
    send(&mut app, &ctx, "ui.pointer", down.clone());
    send(&mut app, &ctx, "ui.tool.select", json!({"tool": "selection"}));
    send(&mut app, &ctx, "ui.tool.select", json!({"tool": "rotateView"}));
    assert_eq!(control_response(&mut app, &ctx, "ui.pointer", drag.clone())["ok"], false, "a tool change ends the press");
    send(&mut app, &ctx, "ui.pointer", down);
    send(&mut app, &ctx, "ui.pointer", drag);
    assert!((app.view().unwrap().rotation.abs() - 180.0).abs() < 1e-3, "a new press starts at the current angle");
    let before = app.view().unwrap().rotation;
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": 400, "y": 300, "space": "screen"}]}));
    send(&mut app, &ctx, "ui.pointer", json!({"events": [{"kind": "drag", "x": 500, "y": 300, "space": "screen"}]}));
    assert!(app.view().unwrap().rotation.is_finite());
    assert!((app.view().unwrap().rotation - before).abs() < 1e-3);
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
    fn after(&mut self, seconds: f64, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += seconds;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let raw = egui::RawInput { screen_rect: Some(rect), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out
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

fn selected(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().selection.objects.len()
}

/// One Escape resets a rotated view when it has nothing else to end, with or without a selected
/// object, which stays selected. Shift held down with it (as after a Space+Shift drag) does not
/// change that, and the repeats of a held Escape do nothing more.
#[test]
fn one_escape_resets_a_rotated_view() {
    let mut w = Window::new();
    w.escape(0.0);
    assert_eq!(w.rotation(), 0.0, "one Escape with nothing selected");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "a second Escape leaves the view at 0°");

    let mut w = Window::new();
    w.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    w.after(0.0, vec![]);
    assert_eq!(selected(&w.app), 1);
    w.escape(0.0);
    assert_eq!((w.rotation(), selected(&w.app)), (0.0, 1), "one Escape with a selected object, which stays selected");

    let mut w = Window::new();
    let shifted = [true, false].map(|pressed| egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::SHIFT,
    });
    w.after(0.0, shifted.to_vec());
    assert_eq!(w.rotation(), 0.0, "Shift+Escape");

    let mut w = Window::new();
    w.after(0.0, vec![key(egui::Key::Escape, false)[0].clone()]);
    assert_eq!(w.rotation(), 0.0, "the press of a held Escape");
    w.app.run("view.rotate", json!({"angle": 30})).unwrap();
    for _ in 0..5 {
        w.after(0.1, vec![key(egui::Key::Escape, true)[0].clone()]);
    }
    w.after(0.1, vec![key(egui::Key::Escape, false)[1].clone()]);
    assert_eq!(w.rotation(), 30.0, "its repeats");

    let mut w = Window::new();
    w.app.select_tool("rotateView");
    w.escape(0.0);
    assert_eq!(w.rotation(), 0.0, "one Escape with the Rotate View tool");

    // A hover highlight that the Escape clears does not count as something it ends.
    let mut w = Window::new();
    w.app.run("shape.rectangle", json!({"x": 150, "y": 100, "width": 120, "height": 80})).unwrap();
    // The new object is the selection, to be put on the path under the pointer.
    w.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    w.app.select_tool("objectsOnPath");
    let view = w.app.view_info();
    let over = vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Move, 150.0, 140.0);
    w.app.session.pointer(&over, view).unwrap();
    assert!(!w.app.session.overlays(view).is_empty(), "Objects on Path highlights the path under the pointer");
    w.escape(0.0);
    assert_eq!(w.rotation(), 0.0, "one Escape with Objects on Path hovering");
}

/// An Escape that ends something, such as isolation mode, leaves the view as it is. The next
/// Escape resets it, however long after it comes and whatever is clicked or pressed in between.
#[test]
fn escape_after_ending_isolation_resets_the_view() {
    let isolated_window = || {
        let mut w = Window::new();
        w.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
        w.app.run("shape.rectangle", json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap();
        w.app.run("select.all", json!({})).unwrap();
        w.app.run("object.group", json!({})).unwrap();
        menus::invoke(&mut w.app, "object.isolate", json!({}));
        w.after(0.0, vec![]);
        w
    };
    let isolated = |w: &Window| w.app.session.active().unwrap().isolation.is_some();
    let mut w = isolated_window();
    assert!(isolated(&w));
    w.escape(0.0);
    assert_eq!((isolated(&w), w.rotation()), (false, 30.0), "the first Escape leaves isolation mode");
    w.escape(5.0);
    assert_eq!(w.rotation(), 0.0, "the second resets the view 5 s later");

    let mut w = isolated_window();
    let empty = w.app.canvas_rect.unwrap().left_top() + egui::vec2(20.0, 20.0);
    w.escape(0.0);
    assert_eq!((isolated(&w), w.rotation()), (false, 30.0));
    w.click(2.0, egui::PointerButton::Primary, empty);
    w.after(0.5, key(egui::Key::ArrowRight, false));
    w.escape(3.0);
    assert_eq!(w.rotation(), 0.0, "after a click and an arrow key between the two");
}

/// An Escape that ends the path the Pen, the Smart Pen or the Curvature tool is drawing leaves the
/// view as it is, with the rubber band shown or not; the next Escape resets it.
#[test]
fn escape_that_ends_a_path_being_drawn_keeps_the_rotation() {
    for (tool, rubber_band) in [("pen", true), ("pen", false), ("smartPen", true), ("smartPen", false), ("curvature", true)] {
        let mut w = Window::new();
        w.app.session.prefs.pen_rubber_band = rubber_band;
        w.app.select_tool(tool);
        let c = w.app.canvas_rect.unwrap().center();
        w.click(0.0, egui::PointerButton::Primary, c);
        w.click(0.1, egui::PointerButton::Primary, c + egui::vec2(60.0, 10.0));
        w.after(0.1, vec![egui::Event::PointerMoved(c + egui::vec2(60.0, 80.0))]);
        assert!(w.app.session.tool_ends_on_escape(w.app.view_info()), "{tool}: a path is being drawn");
        w.escape(0.2);
        assert_eq!(w.rotation(), 30.0, "{tool}, rubber band {rubber_band}: the Escape that ends the path");
        assert!(!w.app.session.tool_ends_on_escape(w.app.view_info()), "{tool}: the path is ended");
        w.escape(3.0);
        assert_eq!(w.rotation(), 0.0, "{tool}, rubber band {rubber_band}: the next Escape");
    }
}

/// In isolation mode, an Escape that ends the Pen's path ends only the path: the next Escape leaves
/// isolation mode, and the one after it resets the view.
#[test]
fn escape_ends_one_thing_at_a_time() {
    let mut w = Window::new();
    w.app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    w.app.run("shape.rectangle", json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap();
    w.app.run("select.all", json!({})).unwrap();
    w.app.run("object.group", json!({})).unwrap();
    menus::invoke(&mut w.app, "object.isolate", json!({}));
    w.after(0.0, vec![]);
    let isolated = |w: &Window| w.app.session.active().unwrap().isolation.is_some();
    assert!(isolated(&w));
    w.app.select_tool("pen");
    let c = w.app.canvas_rect.unwrap().center();
    w.click(0.0, egui::PointerButton::Primary, c);
    w.click(0.1, egui::PointerButton::Primary, c + egui::vec2(60.0, 10.0));
    assert!(w.app.session.tool_ends_on_escape(w.app.view_info()), "a path is being drawn");
    w.escape(0.2);
    assert!(!w.app.session.tool_ends_on_escape(w.app.view_info()), "the first Escape ends the path");
    assert_eq!((isolated(&w), w.rotation()), (true, 30.0), "and only the path");
    w.escape(0.2);
    assert_eq!((isolated(&w), w.rotation()), (false, 30.0), "the second leaves isolation mode");
    w.escape(0.2);
    assert_eq!(w.rotation(), 0.0, "the third resets the view");
}

/// The Escape that leaves the Artboard tool leaves the view as it is; the next resets it.
#[test]
fn escape_after_leaving_the_artboard_tool_resets_the_view() {
    let mut w = Window::new();
    w.app.select_tool("artboard");
    w.escape(0.0);
    assert_eq!((w.app.session.tool_id(), w.rotation()), ("selection", 30.0), "the first Escape leaves the Artboard tool");
    w.escape(5.0);
    assert_eq!(w.rotation(), 0.0, "the next resets the view");
}

/// An Escape that a dialog or a full screen mode takes leaves the view as it is; the next resets it.
#[test]
fn escape_in_a_dialog_or_a_full_screen_mode_keeps_the_rotation() {
    let mut w = Window::new();
    menus::invoke(&mut w.app, "view.rotate", json!({}));
    w.after(0.0, vec![]);
    assert!(w.app.ui.dialog.is_some(), "Rotate View asks for an angle");
    w.escape(0.0);
    assert_eq!((w.app.ui.dialog.is_none(), w.rotation()), (true, 30.0), "Escape closes the dialog");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "the next Escape");

    for mode in [1, 2] {
        let mut w = Window::new();
        w.app.ui.screen_mode = mode;
        w.after(0.0, vec![]);
        w.escape(0.0);
        assert_eq!((w.app.ui.screen_mode, w.rotation()), (0, 30.0), "Escape leaves screen mode {mode}");
        w.escape(0.5);
        assert_eq!(w.rotation(), 0.0, "the next Escape after screen mode {mode}");

        // Shift+Escape does not leave a full screen mode, so it resets the view there.
        let mut w = Window::new();
        w.app.ui.screen_mode = mode;
        w.after(0.0, vec![]);
        let shifted = [true, false].map(|pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        });
        w.after(0.0, shifted.to_vec());
        assert_eq!((w.app.ui.screen_mode, w.rotation()), (mode, 0.0), "Shift+Escape in screen mode {mode}");
    }

    let mut w = Window::new();
    w.app.run("view.presentation", json!({})).unwrap();
    w.after(0.0, vec![]);
    assert_eq!(w.app.ui.screen_mode, 3, "Presentation Mode");
    w.escape(0.0);
    assert_eq!((w.app.ui.screen_mode != 3, w.rotation()), (true, 30.0), "Escape leaves Presentation Mode");
    w.escape(0.5);
    assert_eq!(w.rotation(), 0.0, "the next Escape after Presentation Mode");
}

/// An Escape while About or an Appearance row's popup is open, or while a pointer button is held
/// outside the canvas (a scroll bar, a tab or a panel being dragged) or let go in the same frame,
/// leaves the view as it is.
#[test]
fn escape_with_a_window_open_or_a_button_held_keeps_the_rotation() {
    let mut w = Window::new();
    w.app.ui.about = true;
    w.after(0.0, vec![]);
    w.escape(0.0);
    assert_eq!(w.rotation(), 30.0, "an Escape with About open");

    // An Appearance row's popup (here, the Stroke panel's) closes on Escape as the frame is drawn.
    let mut w = Window::new();
    w.app.ui.transient_panel = Some("stroke".into());
    w.escape(0.0);
    assert_eq!(w.rotation(), 30.0, "an Escape with an Appearance row's popup open");

    let mut w = Window::new();
    let bar = w.app.canvas_rect.unwrap().right_center() + egui::vec2(4.0, 0.0);
    let button = |pressed| egui::Event::PointerButton { pos: bar, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    w.after(0.0, vec![egui::Event::PointerMoved(bar), button(true)]);
    w.escape(0.1);
    assert_eq!(w.rotation(), 30.0, "an Escape with the button held");
    let mut release = vec![button(false)];
    release.extend(key(egui::Key::Escape, false));
    w.after(0.1, release);
    assert_eq!(w.rotation(), 30.0, "an Escape in the frame the button is let go");
    w.escape(0.1);
    assert_eq!(w.rotation(), 0.0, "an Escape after the release");
}

/// An Escape in a text field ends the editing and leaves the view as it is.
#[test]
fn escape_in_a_text_field_keeps_the_rotation() {
    let mut w = Window::new();
    w.ctx.enable_accesskit();
    let out = w.after(0.0, vec![]);
    let tree = out.platform_output.accesskit_update.expect("the window's accessibility tree");
    let field = tree.nodes.iter().find(|(_, node)| node.role() == egui::accesskit::Role::TextInput).map(|(id, _)| *id).expect("a text field");
    let field = w
        .ctx
        .viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, ws)| ws.iter()).find(|r| r.id.accesskit_id() == field).copied())
        .expect("the text field's widget");
    w.ctx.memory_mut(|m| m.request_focus(field.id));
    w.after(0.0, vec![]);
    assert!(w.ctx.egui_wants_keyboard_input(), "the field has the keyboard");
    w.escape(0.0);
    assert_eq!(w.rotation(), 30.0, "the Escape in the field");
}

/// An Escape that closes a popup (the status bar's angle menu, the canvas context menu) or ends a
/// drag of the ruler origin leaves the view as it is; the next Escape resets it.
#[test]
fn escape_that_closes_a_popup_or_ends_a_ruler_origin_drag_keeps_the_rotation() {
    let mut w = Window::new();
    // Find the actual combo by its accessible name, which is present in both build profiles;
    // `Id`'s debug source (including "rot-combo") only exists with debug assertions enabled.
    w.ctx.enable_accesskit();
    let out = w.after(0.0, vec![]);
    let tree = out.platform_output.accesskit_update.expect("the window's accessibility tree");
    let combos: Vec<_> =
        tree.nodes.iter().filter(|(_, node)| node.role() == egui::accesskit::Role::ComboBox && node.label() == Some("Rotation")).collect();
    assert_eq!(combos.len(), 1, "the status bar's angle menu");
    let combo = w
        .ctx
        .viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, ws)| ws.iter()).find(|r| r.id.accesskit_id() == combos[0].0).copied())
        .expect("the angle menu's widget");
    w.click(0.0, egui::PointerButton::Primary, combo.rect.center());
    w.after(0.0, vec![]);
    assert!(craft_ui::ComboBox::is_open(&w.ctx, combo.id), "the angle menu is open");
    w.escape(0.2);
    assert!(!egui::Popup::is_any_open(&w.ctx), "Escape closes it");
    assert_eq!(w.rotation(), 30.0, "the Escape that closes the angle menu");
    w.escape(0.2);
    assert_eq!(w.rotation(), 0.0, "the next Escape");

    let mut w = Window::new();
    w.click(0.0, egui::PointerButton::Secondary, w.app.canvas_rect.unwrap().center());
    w.after(0.0, vec![]);
    assert!(egui::Popup::is_any_open(&w.ctx), "a right click opens the context menu");
    w.escape(0.2);
    assert!(!egui::Popup::is_any_open(&w.ctx), "Escape closes it");
    assert_eq!(w.rotation(), 30.0, "the Escape that closes the context menu");
    w.escape(0.2);
    assert_eq!(w.rotation(), 0.0, "the next Escape");
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
    assert_eq!(w.rotation(), 0.0, "an Escape after the drag ends");
}
