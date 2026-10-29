//! Screen modes and Presentation Mode (#472): Escape leaves Presentation Mode, F cycles the three
//! screen modes, and no screen mode is saved with the preferences. In Presentation Mode, ← and →
//! and a click go between the artboards, no other key acts, a right click opens no context menu and
//! the pointer is the default one.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// One headless frame of the whole window with `events`.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out
}

fn escape() -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() })
        .to_vec()
}

/// A press and release of `key`.
fn key(key: egui::Key) -> Vec<egui::Event> {
    [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }).to_vec()
}

/// A click of the primary button at `pos`, one event a frame.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, pos: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
        frame(app, ctx, vec![e]);
    }
}

/// A document with three 200 × 100 artboards and a rectangle in the middle of the first, in
/// Presentation Mode with the Selection tool, after a frame laid the window out.
fn presenting(ctx: &egui::Context) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.run("shape.rectangle", json!({"x": 90, "y": 40, "width": 20, "height": 20})).unwrap();
    app.select_tool("selection");
    frame(&mut app, ctx, vec![]);
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, ctx, vec![]);
    app
}

/// The artboard the view is on, its zoom and the document point at its center.
fn view(app: &VectorcraftApp) -> (usize, f64, vectorcraft_geom::Point) {
    let v = app.view().unwrap();
    (v.artboard, v.zoom, v.center)
}

/// A check that the document, the selection and the undo steps are still as they are now.
fn snapshot(app: &VectorcraftApp) -> impl Fn(&VectorcraftApp) -> bool + use<> {
    let st = app.session.active().unwrap();
    let (doc, selection, undo) = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
    move |app| {
        let st = app.session.active().unwrap();
        st.doc == doc && st.selection == selection && st.history.undo.len() == undo
    }
}

#[test]
fn escape_leaves_presentation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.run("view.presentation", json!({})).unwrap(), json!(3));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(crate::control::inspect(&app, &ctx)["screenMode"], json!(3), "agents see the screen mode");
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0, "Escape is back in Normal Screen Mode");
    // Escape in Normal Screen Mode leaves it so.
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0);
}

/// In Presentation Mode → and ← go to the next and the previous artboard and fit it in the
/// window, as the status bar navigator's Next and Previous do. On the last artboard → leaves the
/// view as it is, and on the first ←, ↑ and ↓ leave it as it is. No arrow key moves the selection.
#[test]
fn arrow_keys_go_between_artboards_in_presentation_mode() {
    use egui::Key::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp};
    let ctx = egui::Context::default();
    let mut app = presenting(&ctx);
    app.run("select.all", json!({})).unwrap();
    let unchanged = snapshot(&app);
    let centers: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.center()).collect();
    // All three artboards are 200 × 100, so each fits at the zoom that fits the first.
    app.run("view.fitArtboard", json!({})).unwrap();
    let fit = view(&app).1;
    for (k, to) in [(ArrowRight, 1), (ArrowRight, 2), (ArrowLeft, 1), (ArrowLeft, 0)] {
        app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
        frame(&mut app, &ctx, key(k));
        assert_eq!(view(&app), (to, fit, centers[to]), "{k:?}");
    }
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let first = view(&app);
    for k in [ArrowLeft, ArrowUp, ArrowDown] {
        frame(&mut app, &ctx, key(k));
        assert_eq!(view(&app), first, "{k:?} on the first artboard");
    }
    app.run("view.goToArtboard", json!({"index": "last"})).unwrap();
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let last = view(&app);
    frame(&mut app, &ctx, key(ArrowRight));
    assert_eq!(view(&app), last, "→ on the last artboard");
    assert!(unchanged(&app), "no arrow key moved the selected rectangle");
}

/// In Presentation Mode a click anywhere goes to the next artboard, as the status bar navigator's
/// Next does, and the tool gets no press: a click on the rectangle with the Selection tool selects
/// nothing, and with the Rectangle tool a drag draws nothing and a click pressed and released in
/// one frame opens no dialog. A drag, also with the middle button or with Space held, leaves the
/// view as it is, a right click opens no context menu and goes to no other artboard, and on the
/// last artboard a click leaves the view as it is.
#[test]
fn a_click_goes_to_the_next_artboard_in_presentation_mode() {
    let ctx = egui::Context::default();
    let mut app = presenting(&ctx);
    app.run("select.none", json!({})).unwrap();
    let unchanged = snapshot(&app);
    // The middle of the window shows the middle of the first artboard, where the rectangle is.
    let middle = app.canvas_rect.unwrap().center();
    click(&mut app, &ctx, middle);
    assert_eq!(view(&app).0, 1, "the click on the rectangle went to the second artboard");
    assert!(unchanged(&app), "and selected nothing");
    let right = |pressed| egui::Event::PointerButton { pos: middle, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    for e in [vec![right(true)], vec![right(false)], vec![]] {
        frame(&mut app, &ctx, e);
    }
    assert!(!egui::Popup::is_any_open(&ctx), "a right click opens no context menu");
    assert_eq!(view(&app).0, 1, "a right click");
    app.select_tool("rectangle");
    let button = |button, pos, pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
    let to = middle + egui::vec2(80.0, 60.0);
    let second = view(&app);
    let space = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    for (what, b, with_space) in [
        ("a drag", egui::PointerButton::Primary, false),
        ("a middle-button drag", egui::PointerButton::Middle, false),
        ("a Space drag", egui::PointerButton::Primary, true),
    ] {
        let mut events = vec![egui::Event::PointerMoved(middle), button(b, middle, true), egui::Event::PointerMoved(to), button(b, to, false)];
        if with_space {
            events.insert(0, space(true));
            events.push(space(false));
        }
        for e in events {
            frame(&mut app, &ctx, vec![e]);
        }
        assert_eq!(view(&app), second, "{what}");
    }
    let left = |pos, pressed| button(egui::PointerButton::Primary, pos, pressed);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(middle), left(middle, true), left(middle, false)]);
    assert_eq!(view(&app).0, 2, "a click with the Rectangle tool, pressed and released in one frame");
    assert!(app.ui.dialog.is_none(), "opens no Rectangle dialog");
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let last = view(&app);
    click(&mut app, &ctx, middle);
    assert_eq!(view(&app), last, "a click on the last artboard");
    assert!(unchanged(&app), "nothing was selected or drawn");
}

/// In Presentation Mode a tap goes to the next artboard. A tap of two fingers, which undoes in the
/// other screen modes (Enable Touch Gestures), undoes nothing and goes to no other artboard.
#[test]
fn a_tap_goes_to_the_next_artboard_in_presentation_mode() {
    let ctx = egui::Context::default();
    let mut app = presenting(&ctx);
    let objects = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let c = app.canvas_rect.unwrap().center();
    let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
    let button = |pressed| egui::Event::PointerButton { pos: c, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // `fingers` fingers tap 50 points apart, the first also pressing and releasing as the pointer.
    let tap = |app: &mut VectorcraftApp, fingers: u64| {
        let at = |n: u64| c + egui::vec2(n as f32 * 50.0, 0.0);
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::Start, c), egui::Event::PointerMoved(c), button(true)]);
        frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::Start, at(n))).collect());
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::End, c), button(false), egui::Event::PointerGone]);
        frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::End, at(n))).collect());
    };
    tap(&mut app, 1);
    assert_eq!((view(&app).0, objects(&app)), (1, 1), "a tap");
    tap(&mut app, 2);
    assert_eq!((view(&app).0, objects(&app)), (1, 1), "two fingers undo nothing");
}

/// In Presentation Mode no key but ←, → and Escape acts: Cmd/Ctrl+A selects nothing, P and V pick
/// no tool, Delete and Backspace delete nothing, Cmd/Ctrl+Z undoes nothing, Enter opens no dialog,
/// Tab hides no panel and F keeps the screen mode. Escape still leaves the mode, also with a tool
/// that claims Escape.
#[test]
fn no_other_key_works_in_presentation_mode() {
    use egui::{Key, Modifiers};
    let ctx = egui::Context::default();
    let mut app = presenting(&ctx);
    // A press and release of `key` with `modifiers` held, and the text it types.
    let press = |key, modifiers: Modifiers, text: Option<&str>| {
        let k = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        let mut events = vec![egui::Event::ModifiersChanged(modifiers), k(true)];
        events.extend(text.map(|t| egui::Event::Text(t.into())));
        events.extend([k(false), egui::Event::ModifiersChanged(Modifiers::NONE)]);
        events
    };
    app.run("select.none", json!({})).unwrap();
    frame(&mut app, &ctx, press(Key::A, Modifiers::COMMAND, None));
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "Cmd/Ctrl+A selected the rectangle");
    app.run("select.all", json!({})).unwrap();
    let unchanged = snapshot(&app);
    let panels = (app.ui.dock, app.ui.toolbar);
    for (what, events) in [
        ("Delete", press(Key::Delete, Modifiers::NONE, None)),
        ("Backspace", press(Key::Backspace, Modifiers::NONE, None)),
        ("Cmd/Ctrl+Z", press(Key::Z, Modifiers::COMMAND, None)),
        ("Enter", press(Key::Enter, Modifiers::NONE, None)),
        ("Tab", press(Key::Tab, Modifiers::NONE, None)),
        ("F", press(Key::F, Modifiers::NONE, Some("f"))),
        ("P", press(Key::P, Modifiers::NONE, Some("p"))),
    ] {
        frame(&mut app, &ctx, events);
        assert!(unchanged(&app), "{what} changed the document, the selection or the undo steps");
        let state = (app.session.tool_id(), app.ui.dialog.is_none(), app.ui.screen_mode, (app.ui.dock, app.ui.toolbar));
        assert_eq!(state, ("selection", true, 3, panels), "{what}");
    }
    app.select_tool("rectangle");
    frame(&mut app, &ctx, press(Key::V, Modifiers::NONE, Some("v")));
    assert_eq!(app.session.tool_id(), "rectangle", "V");
    // The Intertwine tool claims Escape in the other screen modes.
    app.select_tool("intertwine");
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0, "Escape leaves Presentation Mode");
}

/// In Presentation Mode the pointer over the canvas is the system's default pointer, not the
/// tool's cursor.
#[test]
fn the_pointer_is_the_default_one_in_presentation_mode() {
    let ctx = egui::Context::default();
    let mut app = presenting(&ctx);
    app.select_tool("rectangle");
    let middle = app.canvas_rect.unwrap().center();
    let out = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(middle)]);
    assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::Default);
    assert!(out.platform_output.cursor_image.is_none(), "no tool cursor bitmap");
}

#[test]
fn f_cycles_the_screen_modes_and_the_command_leaves_presentation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    let modes: Vec<u64> = (0..4).map(|_| app.run("view.screenMode", json!({})).unwrap().as_u64().unwrap()).collect();
    assert_eq!(modes, [1, 2, 0, 1]);
    app.run("view.presentation", json!({})).unwrap();
    assert_eq!(app.run("view.screenMode", json!({})).unwrap(), json!(0), "from Presentation Mode, view.screenMode goes back to Normal");
}

#[test]
fn the_screen_mode_is_not_saved() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("view.presentation", json!({})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert!(saved.get("screen_mode").is_none(), "{saved}");
    // Preferences an older version saved in Presentation Mode start in Normal Screen Mode.
    let old: crate::state::UiState = serde_json::from_value(json!({ "screen_mode": 3 })).unwrap();
    assert_eq!(old.screen_mode, 0);
}
