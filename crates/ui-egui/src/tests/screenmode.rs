//! Screen modes and Presentation Mode (#472): Escape leaves the screen modes, and no screen mode
//! is saved with the preferences. In Presentation Mode, ← and → and a click go between the
//! artboards and no other key acts.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// One headless frame of the whole window with `events`.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), events, ..Default::default() };

    crate::testing::app_frame(app, ctx, raw)
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

/// Frames until the view stops moving: the canvas places the view a pass after it is asked to, and a view command eases
/// there under Animated Zoom.
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (mut last, mut still) = (None, 0);
    for _ in 0..120 {
        frame(app, ctx, vec![]);
        let now = view(app);
        still = if last == Some(now) { still + 1 } else { 0 };
        // An ease can leave the view where it was for its first frames.
        if still >= 5 {
            return;
        }
        last = Some(now);
    }
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

fn in_screen_mode(ctx: &egui::Context, mode: u8) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 90, "y": 40, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("selection");
    frame(&mut app, ctx, vec![]);
    app.run("view.screenMode", json!({"mode": mode})).unwrap();
    frame(&mut app, ctx, vec![]);
    app
}

#[test]
fn escape_leaves_both_full_screen_modes_without_editing() {
    for mode in [1, 2] {
        let ctx = crate::testing::ui_context();
        let mut app = in_screen_mode(&ctx, mode);
        let unchanged = snapshot(&app);
        frame(&mut app, &ctx, escape());
        assert_eq!(app.ui.screen_mode, 0, "mode {mode}");
        assert!(unchanged(&app), "mode {mode}: document, selection and undo history");
        frame(&mut app, &ctx, escape());
        assert_eq!(app.ui.screen_mode, 0, "Escape in Normal mode");
    }
}

#[test]
fn modified_escape_keeps_full_screen_mode() {
    for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::COMMAND, egui::Modifiers::ALT] {
        let ctx = crate::testing::ui_context();
        let mut app = in_screen_mode(&ctx, 2);
        let events =
            [true, false].map(|pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers }).to_vec();
        frame(&mut app, &ctx, events);
        assert_eq!(app.ui.screen_mode, 2, "{modifiers:?}");
    }
}

#[test]
fn isolation_takes_escape_before_full_screen() {
    for mode in [1, 2] {
        let ctx = crate::testing::ui_context();
        let mut app = in_screen_mode(&ctx, mode);
        app.run("shape.rectangle", json!({"x": 130, "y": 40, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("object.group", json!({})).unwrap();
        app.run("object.isolate", json!({})).unwrap();
        assert!(app.session.active().unwrap().isolation.is_some());
        frame(&mut app, &ctx, escape());
        assert!(app.session.active().unwrap().isolation.is_none());
        assert_eq!(app.ui.screen_mode, mode, "isolation");
        frame(&mut app, &ctx, escape());
        assert_eq!(app.ui.screen_mode, 0, "after isolation");
    }
}

#[test]
fn a_rotated_view_resets_before_full_screen_exits() {
    let ctx = crate::testing::ui_context();
    let mut app = in_screen_mode(&ctx, 2);
    app.select_tool("rotateView");
    app.run("view.rotate", json!({"angle": 30})).unwrap();
    frame(&mut app, &ctx, escape());
    assert_eq!(app.view().unwrap().rotation, 0.0);
    assert_eq!(app.ui.screen_mode, 2);
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0);
}

#[test]
fn pattern_edit_and_global_edit_take_escape_before_full_screen() {
    for mode in [1, 2] {
        let ctx = crate::testing::ui_context();
        let mut app = in_screen_mode(&ctx, mode);
        app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
        assert!(app.session.active().unwrap().doc.pattern_edit.is_some());
        frame(&mut app, &ctx, escape());
        assert!(app.session.active().unwrap().doc.pattern_edit.is_none());
        assert_eq!(app.ui.screen_mode, mode, "pattern edit in mode {mode}");
        frame(&mut app, &ctx, escape());
        assert_eq!(app.ui.screen_mode, 0, "after pattern edit in mode {mode}");

        let ctx = crate::testing::ui_context();
        let mut app = in_screen_mode(&ctx, mode);
        app.run("shape.rectangle", json!({"x": 130, "y": 40, "width": 20, "height": 20})).unwrap();
        app.run("select.globalEdit", json!({})).unwrap();
        assert!(app.session.active().unwrap().global_edit.is_some());
        frame(&mut app, &ctx, escape());
        assert!(app.session.active().unwrap().global_edit.is_none());
        assert_eq!(app.ui.screen_mode, mode, "Global Edit in mode {mode}");
        frame(&mut app, &ctx, escape());
        assert_eq!(app.ui.screen_mode, 0, "after Global Edit in mode {mode}");
    }
}

#[test]
fn escape_exits_full_screen_then_resets_the_rotation_with_selection_tool() {
    for mode in [1, 2] {
        let ctx = crate::testing::ui_context();
        let mut app = in_screen_mode(&ctx, mode);
        app.run("view.rotate", json!({"angle": 30})).unwrap();
        frame(&mut app, &ctx, escape());
        assert_eq!(app.ui.screen_mode, 0, "first Escape in mode {mode}");
        assert_eq!(app.view().unwrap().rotation, 30.0);
        frame(&mut app, &ctx, escape());
        assert_eq!(app.view().unwrap().rotation, 0.0, "the next Escape after mode {mode}");
    }
}

#[test]
fn escape_leaves_presentation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = crate::testing::ui_context();
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
    let ctx = crate::testing::ui_context();
    let mut app = presenting(&ctx);
    app.run("select.all", json!({})).unwrap();
    let unchanged = snapshot(&app);
    let centers: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.center()).collect();
    // All three artboards are 200 × 100, so each fits at the zoom that fits the first.
    app.run("view.fitArtboard", json!({})).unwrap();
    settle(&mut app, &ctx);
    let fit = view(&app).1;
    for (k, to) in [(ArrowRight, 1), (ArrowRight, 2), (ArrowLeft, 1), (ArrowLeft, 0)] {
        app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
        frame(&mut app, &ctx, key(k));
        settle(&mut app, &ctx);
        assert_eq!(view(&app), (to, fit, centers[to]), "{k:?}");
    }
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let first = view(&app);
    for k in [ArrowLeft, ArrowUp, ArrowDown] {
        frame(&mut app, &ctx, key(k));
        assert_eq!(view(&app), first, "{k:?} on the first artboard");
    }
    app.run("view.goToArtboard", json!({"index": "last"})).unwrap();
    settle(&mut app, &ctx);
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let last = view(&app);
    frame(&mut app, &ctx, key(ArrowRight));
    settle(&mut app, &ctx);
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
    let ctx = crate::testing::ui_context();
    let mut app = presenting(&ctx);
    app.run("select.none", json!({})).unwrap();
    let unchanged = snapshot(&app);
    // The middle of the window shows the middle of the first artboard, where the rectangle is.
    let middle = app.canvas_view().unwrap().rect.center();
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
    settle(&mut app, &ctx);
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
    assert!(!app.any_dialog_open(), "opens no Rectangle dialog");
    settle(&mut app, &ctx);
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let last = view(&app);
    click(&mut app, &ctx, middle);
    settle(&mut app, &ctx);
    assert_eq!(view(&app), last, "a click on the last artboard");
    assert!(unchanged(&app), "nothing was selected or drawn");
}

/// In Presentation Mode a tap goes to the next artboard. A tap of two fingers, which undoes in the
/// other screen modes (Enable Touch Gestures), undoes nothing and goes to no other artboard.
#[test]
fn a_tap_goes_to_the_next_artboard_in_presentation_mode() {
    let ctx = crate::testing::ui_context();
    let mut app = presenting(&ctx);
    let objects = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let c = app.canvas_view().unwrap().rect.center();
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
    let ctx = crate::testing::ui_context();
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
    let panels = crate::panel_docking::layout_of(&app.ui).clone();
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
        let state = (app.session.tool_id(), !app.any_dialog_open(), app.ui.screen_mode, crate::panel_docking::layout_of(&app.ui));
        assert_eq!(state, ("selection", true, 3, &panels), "{what}");
    }
    app.select_tool("rectangle");
    frame(&mut app, &ctx, press(Key::V, Modifiers::NONE, Some("v")));
    assert_eq!(app.session.tool_id(), "rectangle", "V");
    // The Intertwine tool claims Escape in the other screen modes.
    app.select_tool("intertwine");
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0, "Escape leaves Presentation Mode");
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

/// Presentation Mode shows the active artboard fitted to the canvas, and again when the canvas
/// changes size (the window going full screen).
#[test]
fn presentation_mode_fits_the_artboard() {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 90, "y": 40, "width": 20, "height": 20})).unwrap();
    app.select_tool("selection");
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().zoom = 0.5;
    app.run("view.presentation", json!({})).unwrap();
    settle(&mut app, &ctx);
    let canvas = app.canvas_view().unwrap().rect;
    let zoom = app.view().unwrap().zoom;
    // 200 × 100 pt fitted to the canvas: one side reaches across most of it.
    assert!(200.0 * zoom >= f64::from(canvas.width()) * 0.8 || 100.0 * zoom >= f64::from(canvas.height()) * 0.8, "fitted: {zoom} in {canvas:?}");
    // A bigger canvas (the window gone full screen) fits it again.
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))), ..Default::default() };
    // The canvas takes its new size in the first pass and places the view in the next.
    for _ in 0..3 {
        crate::testing::app_frame(&mut app, &ctx, raw.clone());
    }
    assert!(app.view().unwrap().zoom > zoom * 1.5, "fitted again to the larger canvas");
}
