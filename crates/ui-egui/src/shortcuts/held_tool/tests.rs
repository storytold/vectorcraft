use egui::Key;
use serde_json::json;
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::*;

/// A keyboard on one context, so egui keeps the keys held down from frame to frame and marks a
/// held key's later presses as repeats, with the frame times the test sets. A frame runs the app's own
/// `tick` and `ui`.
struct Keyboard {
    ctx: egui::Context,
}

impl Keyboard {
    fn new() -> Self {
        Self { ctx: crate::testing::ui_context() }
    }

    /// One frame at `time` seconds delivering `events`.
    fn frame(&self, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>) {
        self.frame_with_focus(app, time, events, true);
    }

    /// [`Self::frame`], with the window having the keyboard (`focused`) or not.
    fn frame_with_focus(&self, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>, focused: bool) {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        crate::testing::app_frame(app, &self.ctx, egui::RawInput { events, screen_rect, time: Some(time), focused, ..Default::default() });
    }

    /// [`Self::frame`] without the window's parts: the app's tick and its held tools' returns only, so a
    /// pointer press reaches no canvas.
    fn off_canvas(&self, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>) {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let raw = egui::RawInput { events, screen_rect, time: Some(time), focused: true, ..Default::default() };
        crate::testing::part_frame(app, &self.ctx, raw, |app, ui| {
            app.tick(ui.ctx());
            crate::shortcuts::follow_holds(app, ui.ctx());
        });
    }

    /// `key` pressed (or repeated while it is down) at `time`, typing `letter`.
    fn press(&self, app: &mut VectorcraftApp, time: f64, key: Key, letter: &str) {
        let press = egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        self.frame(app, time, vec![press, egui::Event::Text(letter.into())]);
    }

    fn release(&self, app: &mut VectorcraftApp, time: f64, key: Key) {
        let release = egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: egui::Modifiers::NONE };
        self.frame(app, time, vec![release]);
    }

    fn press_s(&self, app: &mut VectorcraftApp, time: f64) {
        self.press(app, time, Key::S, "s");
    }

    fn release_s(&self, app: &mut VectorcraftApp, time: f64) {
        self.release(app, time, Key::S);
    }
}

/// A document with a selected rectangle and the Pen tool chosen.
fn pen_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap();
    app.select_tool("pen");
    app
}

/// S let go while the update prompt is open leaves Scale chosen until the prompt closes.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_tool_key_let_go_behind_the_update_prompt_returns_after_it() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    crate::updates_ui::open_prompt_for_test(&mut app);
    keys.release_s(&mut app, 1.0);
    assert_eq!(app.session.tool_id(), "scale", "Scale stays behind the prompt");
    crate::updates_ui::close_dialogs_for_test(&mut app);
    keys.frame(&mut app, 1.1, vec![]);
    assert_eq!(app.session.tool_id(), "pen", "the Pen is back after the prompt");
}

/// S held from 0 s, its repeats at `repeats`, let go at `release`.
fn hold_s(app: &mut VectorcraftApp, keys: &Keyboard, repeats: &[f64], release: f64) {
    keys.press_s(app, 0.0);
    assert_eq!(app.session.tool_id(), "scale", "the press chooses Scale");
    for &t in repeats {
        keys.press_s(app, t);
        assert_eq!(app.session.tool_id(), "scale", "a repeat at {t} s keeps Scale");
    }
    keys.release_s(app, release);
}

/// A quick press of S chooses Scale for good.
#[test]
fn a_quick_press_of_a_tool_key_chooses_the_tool_for_good() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    hold_s(&mut app, &keys, &[], HOLD_SECONDS / 2.0);
    assert_eq!(app.session.tool_id(), "scale");
    keys.frame(&mut app, 5.0, vec![]);
    assert_eq!(app.session.tool_id(), "scale", "Scale stays chosen");
    assert!(app.held_tool.is_empty());
}

/// S held past the threshold, with its key repeats, and let go returns to the Pen.
#[test]
fn a_held_tool_key_returns_to_the_previous_tool_when_let_go() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    hold_s(&mut app, &keys, &[0.1, 0.2, 0.3, 0.4], 0.5);
    assert_eq!(app.session.tool_id(), "pen");
    assert!(app.held_tool.is_empty());
}

/// S held from the Pen and then V held: letting go of V returns to Scale, and letting go of S
/// then returns to the Pen.
#[test]
fn overlapping_held_tool_keys_return_one_at_a_time() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    keys.press(&mut app, 0.1, Key::V, "v");
    assert_eq!(app.session.tool_id(), "selection");
    keys.release(&mut app, 0.4, Key::V);
    assert_eq!(app.session.tool_id(), "scale", "letting go of V returns to Scale while S is down");
    keys.release_s(&mut app, 0.6);
    assert_eq!(app.session.tool_id(), "pen", "letting go of S returns to the Pen");
    assert!(app.held_tool.is_empty());
}

/// S let go during a Scale drag returns to the Pen once the drag ends, also when the key was let go
/// before the threshold.
#[test]
fn a_tool_key_let_go_during_a_drag_returns_after_the_drag() {
    for release in [HOLD_SECONDS / 2.0, 1.0] {
        let mut app = pen_app();
        let keys = Keyboard::new();
        keys.press_s(&mut app, 0.0);
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 150.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 260.0, 190.0), view).unwrap();
        assert!(app.session.tool_busy(), "Scale is dragging");
        keys.frame(&mut app, release / 2.0, vec![]);
        keys.release_s(&mut app, release);
        assert_eq!(app.session.tool_id(), "scale", "let go at {release} s: Scale stays during the drag");
        keys.frame(&mut app, release + 0.1, vec![]);
        assert_eq!(app.session.tool_id(), "scale", "let go at {release} s: still dragging");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 190.0), view).unwrap();
        keys.frame(&mut app, release + 0.2, vec![]);
        assert_eq!(app.session.tool_id(), "pen", "let go at {release} s: the Pen is back after the drag");
    }
}

/// A click on the canvas between two frames while S is down uses Scale, so letting go of S before
/// the threshold returns to the Pen.
#[test]
fn a_click_between_frames_counts_as_using_the_held_tool() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 150.0, 125.0), view);
    }
    keys.release_s(&mut app, HOLD_SECONDS / 2.0);
    assert_eq!(app.session.tool_id(), "pen");
}

/// A Cmd/Ctrl click while S is down goes to the selection tool lent to Scale, so it does not use
/// Scale: letting go of S before the threshold keeps Scale, whether or not a frame comes during
/// the click.
#[test]
fn a_click_lent_to_a_selection_tool_does_not_use_the_held_tool() {
    for frame_during_click in [false, true] {
        let mut app = pen_app();
        let keys = Keyboard::new();
        keys.press_s(&mut app, 0.0);
        let mods = vectorcraft_tools::Mods { cmd: true, ..Default::default() };
        assert!(app.session.lent_tool_id(&mods).is_some(), "Scale lends a selection tool to a Cmd press");
        let view = app.view_info();
        let event = |kind| PointerEvent { mods, ..PointerEvent::new(kind, 150.0, 125.0) };
        crate::canvas::dispatch(&mut app, &event(PointerKind::Down), view);
        if frame_during_click {
            keys.frame(&mut app, HOLD_SECONDS / 4.0, vec![]);
        }
        crate::canvas::dispatch(&mut app, &event(PointerKind::Up), view);
        keys.release_s(&mut app, HOLD_SECONDS / 2.0);
        assert_eq!(app.session.tool_id(), "scale", "frame during the click: {frame_during_click}");
        assert!(app.held_tool.is_empty());
    }
}

/// A pan with the Hand tool while H is down uses the Hand: letting go of H quickly after the pan
/// returns to the Pen, and letting go of H during the pan returns once the pan ends.
#[test]
fn a_view_tool_drag_counts_as_using_the_held_tool() {
    for let_go_during_pan in [false, true] {
        let mut app = pen_app();
        let keys = Keyboard::new();
        keys.frame(&mut app, 0.0, vec![]);
        let at = app.canvas_view().expect("the canvas was drawn").rect.center();
        let to = at + egui::vec2(30.0, 20.0);
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let key = |pressed| egui::Event::Key { key: Key::H, physical_key: Some(Key::H), pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        keys.frame(&mut app, 0.01, vec![egui::Event::PointerMoved(at), key(true), egui::Event::Text("h".into())]);
        assert_eq!(app.session.tool_id(), "hand", "let go during the pan: {let_go_during_pan}");
        keys.frame(&mut app, 0.02, vec![button(at, true)]);
        keys.frame(&mut app, 0.03, vec![egui::Event::PointerMoved(to)]);
        assert!(crate::canvas::in_gesture(&app, &keys.ctx), "the Hand pans");
        if let_go_during_pan {
            keys.frame(&mut app, 0.05, vec![key(false)]);
            assert_eq!(app.session.tool_id(), "hand", "the Hand stays during the pan");
            keys.frame(&mut app, 0.06, vec![button(to, false)]);
        } else {
            keys.frame(&mut app, 0.05, vec![button(to, false)]);
            keys.frame(&mut app, 0.06, vec![key(false)]);
        }
        keys.frame(&mut app, 0.07, vec![]);
        assert_eq!(app.session.tool_id(), "pen", "let go during the pan: {let_go_during_pan}");
        assert!(app.held_tool.is_empty());
    }
}

/// A pointer press in the frame that lets go of S: a press that reaches Scale on the canvas returns
/// to the Pen, and a press elsewhere (no canvas press follows) keeps Scale, also when the next frame
/// comes late.
#[test]
fn a_press_in_the_frame_that_lets_go_waits_for_the_canvas() {
    for canvas in [true, false] {
        let mut app = pen_app();
        let keys = Keyboard::new();
        keys.press_s(&mut app, 0.0);
        let at = egui::pos2(150.0, 125.0);
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        let release = egui::Event::Key { key: Key::S, physical_key: Some(Key::S), pressed: false, repeat: false, modifiers: egui::Modifiers::NONE };
        // The press reaches no canvas in these frames: the canvas's own press is dispatched here.
        keys.off_canvas(&mut app, 0.1, vec![egui::Event::PointerMoved(at), press, release]);
        assert_eq!(app.session.tool_id(), "scale", "canvas {canvas}: no decision before the canvas");
        if canvas {
            let view = app.view_info();
            for kind in [PointerKind::Down, PointerKind::Up] {
                crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 150.0, 125.0), view);
            }
        }
        keys.off_canvas(&mut app, 2.0, vec![]);
        assert_eq!(app.session.tool_id(), if canvas { "pen" } else { "scale" }, "canvas {canvas}");
    }
}

/// With S held and then V held, a click with the Selection tool uses Selection only: V let go
/// quickly returns to Scale, and S let go quickly then keeps Scale.
#[test]
fn a_click_uses_only_the_tool_of_the_key_held_last() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    keys.press(&mut app, 0.02, Key::V, "v");
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 150.0, 125.0), view);
    }
    keys.release(&mut app, 0.05, Key::V);
    assert_eq!(app.session.tool_id(), "scale", "V was used, so Scale is back");
    keys.release_s(&mut app, 0.1);
    assert_eq!(app.session.tool_id(), "scale", "Scale was not used, so it stays");
}

/// Enter with S down opens the Scale dialog, and with V down the Move dialog; the key let go quickly
/// while the dialog is open returns to the Pen once the dialog closes.
#[test]
fn a_dialog_opened_during_the_hold_counts_as_using_the_tool() {
    for (key, letter, tool, dialog) in [(Key::S, "s", "scale", "scale"), (Key::V, "v", "selection", "move")] {
        let mut app = pen_app();
        let keys = Keyboard::new();
        keys.press(&mut app, 0.0, key, letter);
        let enter =
            egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        keys.frame(&mut app, 0.05, vec![enter]);
        let top = crate::tests::picks::open_dialogs(&app).pop().and_then(|d| d.key);
        assert_eq!(top, Some(format!("object.{dialog}")), "Enter opens the {dialog} dialog");
        keys.release(&mut app, 0.1, key);
        assert_eq!(app.session.tool_id(), tool, "{tool} stays while its dialog is open");
        crate::shortcuts::tests::close_top_dialog(&mut app, false);
        keys.frame(&mut app, 0.15, vec![]);
        assert_eq!(app.session.tool_id(), "pen", "after the {dialog} dialog");
    }
}

/// Another dialog (New Document) opened while S is down and S let go quickly while it is open: Scale
/// was not used, so it stays once the dialog closes.
#[test]
fn another_dialog_opened_during_the_hold_does_not_use_the_tool() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    // PLAN: Preferences is its own viewport and `any_dialog_open()` does not count it; New Document is a dialog of the host.
    app.run("file.newDialog", json!({})).unwrap();
    keys.frame(&mut app, 0.05, vec![]);
    keys.release_s(&mut app, 0.1);
    crate::shortcuts::tests::close_top_dialog(&mut app, false);
    keys.frame(&mut app, 0.15, vec![]);
    assert_eq!(app.session.tool_id(), "scale");
}

/// With General › Hold a Tool Shortcut to Use the Tool Temporarily off, a held S chooses Scale for
/// good, as a quick press does.
#[test]
fn with_the_preference_off_a_held_tool_key_chooses_the_tool_for_good() {
    let mut app = pen_app();
    app.run("prefs.set", json!({"key": "holdToolKeys", "value": false})).unwrap();
    let keys = Keyboard::new();
    hold_s(&mut app, &keys, &[0.1, 0.2, 0.3, 0.4], 0.5);
    assert_eq!(app.session.tool_id(), "scale");
}

/// With the preference off, each repeat of a held S chooses Scale again, as before: a tool chosen in
/// between gives way to Scale. With it on, a repeat chooses nothing.
#[test]
fn with_the_preference_off_each_repeat_chooses_the_tool_again() {
    for hold in [false, true] {
        let mut app = pen_app();
        app.run("prefs.set", json!({"key": "holdToolKeys", "value": hold})).unwrap();
        let keys = Keyboard::new();
        keys.press_s(&mut app, 0.0);
        app.select_tool("rectangle");
        keys.press_s(&mut app, 0.3);
        let chosen = if hold { "rectangle" } else { "scale" };
        assert_eq!(app.session.tool_id(), chosen, "holdToolKeys: {hold}");
    }
}

/// Scale chosen again on the toolbar while S is held stays chosen when S is let go.
#[test]
fn the_held_tool_chosen_again_stays_when_the_key_is_let_go() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    app.select_tool("scale");
    // The repeats of S, still down, choose nothing.
    keys.press_s(&mut app, 0.3);
    keys.release_s(&mut app, 0.5);
    assert_eq!(app.session.tool_id(), "scale");
}

/// The window losing the keyboard while S is held, also during a drag, ends the hold: Scale stays.
#[test]
fn losing_the_keyboard_ends_the_hold() {
    for drag in [false, true] {
        let mut app = pen_app();
        let keys = Keyboard::new();
        keys.press_s(&mut app, 0.0);
        let view = app.view_info();
        if drag {
            app.session.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 150.0), view).unwrap();
            app.session.pointer(&PointerEvent::new(PointerKind::Drag, 260.0, 190.0), view).unwrap();
        }
        keys.frame_with_focus(&mut app, 0.3, vec![egui::Event::WindowFocused(false)], false);
        if drag {
            app.session.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 190.0), view).unwrap();
        }
        keys.frame(&mut app, 0.5, vec![egui::Event::WindowFocused(true)]);
        keys.release_s(&mut app, 0.6);
        assert_eq!(app.session.tool_id(), "scale", "drag {drag}: Scale stays");
    }
}

/// Another tool chosen while S is held stays chosen when S is let go.
#[test]
fn a_tool_chosen_during_the_hold_stays_when_the_key_is_let_go() {
    let mut app = pen_app();
    let keys = Keyboard::new();
    keys.press_s(&mut app, 0.0);
    app.select_tool("rectangle");
    keys.frame(&mut app, 0.3, vec![]);
    keys.release_s(&mut app, 0.5);
    assert_eq!(app.session.tool_id(), "rectangle");
}
