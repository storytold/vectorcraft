use serde_json::json;

use super::super::handle;
use super::*;

/// A keyboard on one context, so egui keeps the keys held down from frame to frame and marks a
/// held key's later presses as repeats, with the frame times the test sets.
struct Keyboard {
    ctx: egui::Context,
}

impl Keyboard {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame at `time` seconds delivering `events`, with the window having the keyboard
    /// (`focused`) or not.
    fn frame_with_focus(&self, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>, focused: bool) {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let mut out = self.ctx.run_ui(egui::RawInput { events, screen_rect, time: Some(time), focused, ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            handle(app, ui.ctx());
        });
        out.textures_delta.clear();
    }

    fn frame(&self, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>) {
        self.frame_with_focus(app, time, events, true);
    }

    /// [`Self::frame`], giving the events the shortcuts left in the first pass for what is drawn
    /// after them.
    fn frame_left(&self, app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>) -> Vec<egui::Event> {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let mut left = None;
        let mut out = self.ctx.run_ui(egui::RawInput { events, screen_rect, time: Some(time), focused: true, ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            handle(app, ui.ctx());
            if left.is_none() {
                left = Some(ui.ctx().input(|i| i.events.clone()));
            }
        });
        out.textures_delta.clear();
        left.unwrap_or_default()
    }

    /// `key` pressed (a repeat when it is already down) at `time`, typing `text`.
    fn press(&self, app: &mut VectorcraftApp, time: f64, key: Key, text: &str) {
        self.frame(app, time, press(key, text, Modifiers::NONE));
    }

    fn release(&self, app: &mut VectorcraftApp, time: f64, key: Key) {
        self.frame(app, time, vec![release(key)]);
    }
}

fn press(key: Key, text: &str, modifiers: Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }, egui::Event::Text(text.into())]
}

fn release(key: Key) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: Modifiers::NONE }
}

/// A document with a selected rectangle filled red with a blue stroke, the Pen tool chosen, and
/// Hold Shortcuts `on`.
fn app_with(on: bool) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    crate::shortcut_editor::sync(&app.ui);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("paint.setStroke", json!({"color": "#0000ff"})).unwrap();
    app.run("prefs.set", json!({"key": "holdShortcuts", "value": on})).unwrap();
    app.select_tool("pen");
    app
}

/// erolka's X: a quick press chooses the Eyedropper, and holding X swaps fill and stroke.
fn erolka_x(on: bool) -> VectorcraftApp {
    let mut app = app_with(on);
    app.run("shortcuts.set", json!({"id": "tool:eyedropper", "shortcut": "X", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
    app
}

/// The fill of the selected object, as `#rrggbb`.
fn fill(app: &VectorcraftApp) -> String {
    let d = app.session.active().unwrap();
    let id = *d.selection.objects.first().unwrap();
    d.doc.node(id).unwrap().appearance.fill_paint().color().unwrap().to_hex()
}

fn selected(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().selection.objects.len()
}

/// A quick press of X runs its tap at the release, and holding X runs the hold once at the delay,
/// also with no repeats; the repeats until the release run nothing more.
#[test]
fn a_tap_runs_the_key_and_a_hold_runs_the_hold_shortcut() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    assert_eq!(app.session.tool_id(), "pen", "the press waits");
    keys.release(&mut app, 0.1, Key::X);
    assert_eq!(app.session.tool_id(), "eyedropper", "the quick press chooses the Eyedropper");
    assert_eq!(fill(&app), "#ff0000", "and swaps nothing");

    app.select_tool("pen");
    keys.press(&mut app, 1.0, Key::X, "x");
    keys.frame(&mut app, 1.2, vec![]);
    assert_eq!(fill(&app), "#ff0000", "before the delay nothing runs");
    keys.frame(&mut app, 1.35, vec![]);
    assert_eq!(fill(&app), "#0000ff", "at the delay the hold swaps fill and stroke");
    assert_eq!(app.session.tool_id(), "pen", "and chooses no tool");
    for t in [1.5, 1.6, 1.7] {
        keys.press(&mut app, t, Key::X, "x");
    }
    keys.release(&mut app, 1.8, Key::X);
    assert_eq!(fill(&app), "#0000ff", "the repeats and the release swap nothing more");
    assert_eq!(app.session.tool_id(), "pen");
}

/// The delay is the Hold Shortcut Delay read at the press.
#[test]
fn the_hold_waits_for_the_hold_shortcut_delay() {
    let mut app = erolka_x(true);
    app.run("prefs.set", json!({"key": "keyHoldDelay", "value": 0.8})).unwrap();
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    app.run("prefs.set", json!({"key": "keyHoldDelay", "value": 0.2})).unwrap();
    keys.frame(&mut app, 0.5, vec![]);
    assert_eq!(fill(&app), "#ff0000", "a later change does not move the wait");
    keys.frame(&mut app, 0.8, vec![]);
    assert_eq!(fill(&app), "#0000ff");
}

/// erolka's `.`: a quick press keeps the key's own command (Apply Last Gradient) and keeps the
/// selection, and holding it deselects.
#[test]
fn the_period_key_taps_its_own_command_and_holds_to_deselect() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "select.none", "hold": "."})).unwrap();
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::Period, ".");
    keys.release(&mut app, 0.1, Key::Period);
    assert_eq!(selected(&app), 1, "the quick press keeps the selection");
    let d = app.session.active().unwrap();
    let id = *d.selection.objects.first().unwrap();
    let appearance = &d.doc.node(id).unwrap().appearance;
    let gradient = |p: vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Gradient(_));
    assert!(gradient(appearance.fill_paint()) || gradient(appearance.stroke_paint()), "and applies the last gradient");
    // Solid paint again, so a hold that also ran the tap would show.
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("paint.setStroke", json!({"color": "#0000ff"})).unwrap();
    keys.press(&mut app, 1.0, Key::Period, ".");
    keys.frame(&mut app, 1.4, vec![]);
    assert_eq!(selected(&app), 0, "the hold deselects");
    keys.release(&mut app, 1.5, Key::Period);
    let appearance = &app.session.active().unwrap().doc.node(id).unwrap().appearance;
    assert!(!gradient(appearance.fill_paint()) && !gradient(appearance.stroke_paint()), "and applies no gradient");
}

/// A hold on a letter alone does not catch the letter with Shift: Shift+X, held, does with Hold
/// Shortcuts on what it does with them off, and does not wait.
#[test]
fn a_hold_on_a_letter_leaves_the_letter_with_shift_alone() {
    let mut results = vec![];
    for on in [false, true] {
        let mut app = erolka_x(on);
        let keys = Keyboard::new();
        keys.frame(&mut app, 0.0, press(Key::X, "X", Modifiers::SHIFT));
        assert!(app.hold_keys.pending.is_none(), "Shift+X does not wait (on {on})");
        let pressed = (fill(&app), app.session.tool_id().to_string());
        keys.frame(&mut app, 1.0, vec![]);
        keys.frame(&mut app, 1.1, vec![release(Key::X)]);
        results.push((pressed, fill(&app), app.session.tool_id().to_string()));
    }
    assert_eq!(results[0], results[1], "holding Shift+X does not run X's hold");
}

/// A letter the active tool takes itself (E with the Extend Path tool) does not wait, also with a
/// hold shortcut on it.
#[test]
fn a_letter_the_tool_takes_does_not_wait() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "E"})).unwrap();
    app.select_tool(vectorcraft_tools::addons::extend_path::ID);
    assert_eq!(app.session.tool_id(), vectorcraft_tools::addons::extend_path::ID);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::E, "e");
    assert!(app.hold_keys.pending.is_none(), "E stays the tool's");
    keys.frame(&mut app, 1.0, vec![]);
    keys.release(&mut app, 1.1, Key::E);
    assert_eq!(fill(&app), "#ff0000", "the hold does not run");
    assert_eq!(app.session.tool_id(), vectorcraft_tools::addons::extend_path::ID);
}

/// An input method that starts composing while a key waits ends the wait without running anything.
#[test]
fn an_input_method_composition_ends_the_wait() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    assert!(app.hold_keys.pending.is_some());
    keys.frame(&mut app, 0.1, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "か".into(), active_range_chars: Some(1..1) })]);
    assert!(app.hold_keys.pending.is_none(), "the composition ends the wait");
    keys.frame(&mut app, 1.0, vec![]);
    keys.release(&mut app, 1.1, Key::X);
    assert_eq!(app.session.tool_id(), "pen", "no tap ran");
    assert_eq!(fill(&app), "#ff0000", "and no hold");
}

/// A dialog opened, or Presentation Mode entered, while a key waits ends the wait without running
/// anything.
#[test]
fn a_dialog_or_presentation_mode_ends_the_wait() {
    for what in ["dialog", "Presentation Mode"] {
        let mut app = erolka_x(true);
        let keys = Keyboard::new();
        keys.press(&mut app, 0.0, Key::X, "x");
        assert!(app.hold_keys.pending.is_some(), "{what}");
        if what == "dialog" {
            app.ui.dialog = Some(crate::state::Dialog::new("preferences", json!({})));
        } else {
            app.ui.screen_mode = 3;
        }
        keys.frame(&mut app, 0.1, vec![]);
        assert!(app.hold_keys.pending.is_none(), "{what} ends the wait");
        app.ui.dialog = None;
        app.ui.screen_mode = 0;
        keys.frame(&mut app, 1.0, vec![]);
        keys.release(&mut app, 1.1, Key::X);
        assert_eq!(app.session.tool_id(), "pen", "{what}: no tap ran");
        assert_eq!(fill(&app), "#ff0000", "{what}: and no hold");
    }
}

/// A dialog opened while a key waits does not get the key's repeat, or the text it types, from the
/// frame that also lets the key go.
#[test]
fn a_dialog_that_ends_the_wait_gets_no_repeat_from_the_frame_of_the_release() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    assert!(app.hold_keys.pending.is_some());
    app.ui.dialog = Some(crate::state::Dialog::new("preferences", json!({})));
    let mut events = press(Key::X, "x", Modifiers::NONE);
    events.push(release(Key::X));
    let left = keys.frame_left(&mut app, 0.1, events);
    assert!(app.hold_keys.pending.is_none(), "the dialog ends the wait");
    assert!(
        !left.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })),
        "neither the repeat nor its text reaches the dialog: {left:?}"
    );
}

/// A new wait does not drop a wait left in another viewport unseen: that wait ends, and its key's
/// repeats stay held back in its viewport.
#[test]
fn a_wait_in_another_viewport_ends_when_a_new_one_starts() {
    let mut app = erolka_x(true);
    let other = egui::ViewportId::from_hash_of("another window");
    app.hold_keys.pending = Some(Pending {
        entry: "paint.swap".into(),
        chords: vec![],
        key: Key::X,
        physical: Some(Key::X),
        mods: Modifiers::NONE,
        since: 0.0,
        delay: 0.35,
        viewport: other,
    });
    let keys = Keyboard::new();
    keys.press(&mut app, 0.1, Key::X, "x");
    assert!(app.hold_keys.pending.as_ref().is_some_and(|p| p.viewport == egui::ViewportId::ROOT), "the new press waits here");
    assert!(app.hold_keys.swallowed.iter().any(|s| s.viewport == other && s.key == Key::X), "the other window's key stays held back");
    assert_eq!(fill(&app), "#ff0000", "the other wait ran nothing");
}

/// With Hold Shortcuts off, a key with a hold shortcut runs on its press, as every key does, and
/// holding it runs nothing else.
#[test]
fn with_hold_shortcuts_off_every_key_runs_on_its_press() {
    let mut app = erolka_x(false);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    assert_eq!(app.session.tool_id(), "eyedropper", "the press runs at once");
    keys.frame(&mut app, 1.0, vec![]);
    keys.release(&mut app, 1.2, Key::X);
    assert_eq!(fill(&app), "#ff0000", "the hold shortcut does not act");
    assert!(app.hold_keys.pending.is_none());
}

/// Turning Hold Shortcuts off while a key waits ends the wait without running anything.
#[test]
fn turning_hold_shortcuts_off_ends_the_wait() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    assert!(app.hold_keys.pending.is_some());
    app.run("prefs.set", json!({"key": "holdShortcuts", "value": false})).unwrap();
    keys.frame(&mut app, 0.5, vec![]);
    keys.release(&mut app, 0.6, Key::X);
    assert!(app.hold_keys.pending.is_none());
    assert_eq!(fill(&app), "#ff0000", "no hold ran");
    assert_eq!(app.session.tool_id(), "pen", "and no tap");
}

/// A key without a hold shortcut runs in its press frame, and a waiting key pressed before it acts
/// first.
#[test]
fn another_key_ends_the_wait_and_runs_after_it() {
    let (swap, default) = (label("paint.swap"), label("paint.default"));
    let mut app = swap_on_x();
    app.select_tool("selection");
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::P, "p");
    assert_eq!(app.session.tool_id(), "pen", "P runs on its press");
    keys.release(&mut app, 0.05, Key::P);
    let n = steps(&app, 0).len();
    keys.press(&mut app, 0.1, Key::X, "x");
    assert_eq!(steps(&app, n), Vec::<String>::new(), "X waits");
    keys.press(&mut app, 0.2, Key::D, "d");
    assert_eq!(steps(&app, n), [swap.clone(), default.clone()], "X's tap ran, then D");
    keys.release(&mut app, 0.3, Key::X);
    keys.release(&mut app, 0.35, Key::D);
    assert_eq!(steps(&app, n), [swap, default], "the release of X runs nothing more");
    assert_eq!(selected(&app), 1, "and no hold");
}

/// The labels of the document's undo steps after the first `from`.
fn steps(app: &VectorcraftApp, from: usize) -> Vec<String> {
    app.session.doc().unwrap().history.undo.iter().skip(from).map(|e| e.label.clone()).collect()
}

/// The label of the undo step `command` records on [`app_with`]'s document.
fn label(command: &str) -> String {
    let mut app = app_with(true);
    let n = steps(&app, 0).len();
    app.run(command, json!({})).unwrap();
    steps(&app, n).pop().unwrap()
}

/// A quick press of X swaps fill and stroke, and holding X deselects; D sets the default fill and
/// stroke, as by default.
fn swap_on_x() -> VectorcraftApp {
    let mut app = app_with(true);
    assert_eq!(crate::shortcut_editor::default_of("paint.default"), Some("D"));
    app.run("shortcuts.set", json!({"id": "paint.swap", "shortcut": "X", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "select.none", "hold": "X"})).unwrap();
    app
}

/// Two keys pressed in one frame act in the order they went down, each once.
#[test]
fn keys_in_one_frame_act_in_order() {
    let (swap, default) = (label("paint.swap"), label("paint.default"));
    assert_ne!(swap, default);
    let mut app = swap_on_x();
    let keys = Keyboard::new();
    let n = steps(&app, 0).len();
    let mut events = press(Key::X, "x", Modifiers::NONE);
    events.extend(press(Key::D, "d", Modifiers::NONE));
    keys.frame(&mut app, 0.0, events);
    assert_eq!(steps(&app, n), [swap.clone(), default.clone()], "X's tap first, then D");
    keys.frame(&mut app, 1.0, vec![release(Key::X), release(Key::D)]);
    // Paint that D changes again.
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let n = steps(&app, 0).len();
    let mut events = press(Key::D, "d", Modifiers::NONE);
    events.extend(press(Key::X, "x", Modifiers::NONE));
    keys.frame(&mut app, 2.0, events);
    keys.frame(&mut app, 2.1, vec![release(Key::X)]);
    assert_eq!(steps(&app, n), [default, swap], "D first, then X's tap at its release");
    assert_eq!(selected(&app), 1, "and no hold");
}

/// A wait that starts in a frame after another key, and that a release or another key ends in the
/// same frame, runs its tap after that key.
#[test]
fn a_wait_ended_in_its_frame_runs_after_the_keys_before_it() {
    let (swap, default) = (label("paint.swap"), label("paint.default"));
    let mut app = swap_on_x();
    let keys = Keyboard::new();
    let n = steps(&app, 0).len();
    let mut events = press(Key::D, "d", Modifiers::NONE);
    events.extend(press(Key::X, "x", Modifiers::NONE));
    events.push(release(Key::X));
    keys.frame(&mut app, 0.0, events);
    assert_eq!(steps(&app, n), [default.clone(), swap.clone()], "D first, then X's tap");
    assert!(app.hold_keys.pending.is_none());
    keys.frame(&mut app, 1.0, vec![release(Key::D)]);
    assert_eq!(selected(&app), 1, "nothing waits after it");
    // Ended by V while X is still down: D, X's tap, then V.
    let n = steps(&app, 0).len();
    let mut events = press(Key::D, "d", Modifiers::NONE);
    events.extend(press(Key::X, "x", Modifiers::NONE));
    events.extend(press(Key::V, "v", Modifiers::NONE));
    keys.frame(&mut app, 2.0, events);
    assert_eq!(steps(&app, n), [default, swap], "D first, then X's tap");
    assert_eq!(app.session.tool_id(), "selection", "then V");
    keys.press(&mut app, 2.5, Key::X, "x");
    keys.frame(&mut app, 2.6, vec![release(Key::X), release(Key::D), release(Key::V)]);
    assert_eq!(steps(&app, n).len(), 2, "X's repeats run nothing");
    assert_eq!(selected(&app), 1, "and no hold");
    // Ended by a command chord (Deselect): D, X's tap, then the chord.
    assert_eq!(crate::shortcut_editor::default_of("select.none"), Some("Cmd+Shift+A"));
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let n = steps(&app, 0).len();
    let mut events = press(Key::D, "d", Modifiers::NONE);
    events.extend(press(Key::X, "x", Modifiers::NONE));
    let modifiers = Modifiers::COMMAND | Modifiers::SHIFT;
    events.push(egui::Event::Key { key: Key::A, physical_key: Some(Key::A), pressed: true, repeat: false, modifiers });
    keys.frame(&mut app, 3.0, events);
    assert_eq!(steps(&app, n), [label("paint.default"), label("paint.swap")], "D first, then X's tap");
    assert_eq!(selected(&app), 0, "then the chord");
}

/// A command chord (Undo) pressed before a wait that starts and ends in one frame runs first: the
/// key's tap does not run before it, and the frame acts as it does with Hold Shortcuts off.
#[test]
fn a_chord_before_a_wait_ended_in_its_frame_runs_first() {
    assert_eq!(crate::shortcut_editor::default_of("edit.undo"), Some("Cmd+Z"));
    let frame = |app: &mut VectorcraftApp| {
        app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
        let mut events =
            vec![egui::Event::Key { key: Key::Z, physical_key: Some(Key::Z), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }];
        events.extend(press(Key::X, "x", Modifiers::NONE));
        events.push(release(Key::X));
        Keyboard::new().frame(app, 0.0, events);
        (fill(app), selected(app))
    };
    let mut off = swap_on_x();
    off.run("prefs.set", json!({"key": "holdShortcuts", "value": false})).unwrap();
    let off = frame(&mut off);
    let mut app = swap_on_x();
    let on = frame(&mut app);
    assert_eq!(on.0, "#ff0000", "Undo restores the red fill and does not undo X's swap");
    assert_eq!(on, off);
    assert!(app.hold_keys.pending.is_none());
}

/// A key the active tool takes (E with the Extend Path tool), pressed before a wait that starts and
/// ends in one frame, reaches the tool first: the key's tap, which chooses another tool, does not
/// run before it, and the frame acts as it does with Hold Shortcuts off.
#[test]
fn a_tool_key_before_a_wait_ended_in_its_frame_reaches_the_tool_first() {
    let frame = |app: &mut VectorcraftApp| {
        app.select_tool(vectorcraft_tools::addons::extend_path::ID);
        let mut events = press(Key::E, "e", Modifiers::NONE);
        events.extend(press(Key::X, "x", Modifiers::NONE));
        events.push(release(Key::X));
        Keyboard::new().frame(app, 0.0, events);
        (app.session.tool_id().to_string(), fill(app))
    };
    let off = frame(&mut erolka_x(false));
    let mut app = erolka_x(true);
    let on = frame(&mut app);
    assert_eq!(on.0, "eyedropper", "X's tap chooses the Eyedropper after E reached the Extend Path tool");
    assert_eq!(on, off);
    assert!(app.hold_keys.pending.is_none());
}

/// After a command chord before a wait that starts and ends in one frame, the key's repeats in
/// that frame and in later ones, and the text they type, reach the frame as with Hold Shortcuts
/// off.
#[test]
fn a_chord_before_a_wait_leaves_the_key_repeats_to_the_frame() {
    let run = |app: &mut VectorcraftApp| {
        app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
        let n = steps(app, 0).len();
        let keys = Keyboard::new();
        let mut events =
            vec![egui::Event::Key { key: Key::Z, physical_key: Some(Key::Z), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }];
        events.extend(press(Key::X, "x", Modifiers::NONE));
        events.extend(press(Key::X, "x", Modifiers::NONE));
        events.extend(press(Key::D, "d", Modifiers::NONE));
        let first = keys.frame_left(app, 0.0, events);
        let mut events = press(Key::X, "x", Modifiers::NONE);
        events.extend([release(Key::X), release(Key::D)]);
        let second = keys.frame_left(app, 0.05, events);
        (steps(app, n), first, second, fill(app))
    };
    let mut off = swap_on_x();
    off.run("prefs.set", json!({"key": "holdShortcuts", "value": false})).unwrap();
    let off = run(&mut off);
    let mut app = swap_on_x();
    let on = run(&mut app);
    assert_eq!(on, off);
    assert!(app.hold_keys.pending.is_none());
    assert!(app.hold_keys.swallowed.is_empty());
}

/// The repeats of a key in the frame that starts its wait, and the text they type, stay out of the
/// frame.
#[test]
fn repeats_in_the_frame_that_starts_a_wait_stay_out_of_it() {
    let mut app = swap_on_x();
    let keys = Keyboard::new();
    let n = steps(&app, 0).len();
    let mut events = press(Key::X, "x", Modifiers::NONE);
    events.extend(press(Key::X, "x", Modifiers::NONE));
    events.extend(press(Key::X, "x", Modifiers::NONE));
    let left = keys.frame_left(&mut app, 0.0, events);
    assert!(app.hold_keys.pending.is_some(), "X waits");
    assert!(!left.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })), "{left:?}");
    assert!(steps(&app, n).is_empty(), "the repeats run nothing");
}

/// A press and its release in one frame are a tap, run in that frame.
#[test]
fn a_press_and_release_in_one_frame_is_a_tap() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    let mut events = press(Key::X, "x", Modifiers::NONE);
    events.push(release(Key::X));
    keys.frame(&mut app, 0.0, events);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert!(app.hold_keys.pending.is_none());
    keys.frame(&mut app, 5.0, vec![]);
    assert_eq!(fill(&app), "#ff0000", "nothing waits after it");
}

/// Another key pressed after the delay has passed, before a frame ran the hold, runs the hold.
#[test]
fn an_overdue_wait_ended_by_another_key_runs_the_hold() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    keys.press(&mut app, 0.5, Key::V, "v");
    assert_eq!(fill(&app), "#0000ff", "the hold ran");
    assert_eq!(app.session.tool_id(), "selection", "then V");
}

/// The window losing the keyboard ends the wait without running anything.
#[test]
fn losing_the_keyboard_ends_the_wait() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    keys.frame_with_focus(&mut app, 0.1, vec![], false);
    keys.frame(&mut app, 1.0, vec![]);
    assert_eq!(app.session.tool_id(), "pen");
    assert_eq!(fill(&app), "#ff0000");
    assert!(app.hold_keys.pending.is_none());
}

/// Cmd, Ctrl or Alt pressed while a key waits ends the wait as a tap, before the chord runs.
#[test]
fn a_command_modifier_ends_the_wait_as_a_tap() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::X, "x");
    keys.frame(&mut app, 0.1, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND)]);
    assert_eq!(app.session.tool_id(), "eyedropper");
    keys.frame(&mut app, 1.0, vec![]);
    assert_eq!(fill(&app), "#ff0000", "nothing waits after it");
}

/// A tool key with a hold shortcut runs the hold, and the tool is not chosen for the time the key
/// is held (`holdToolKeys`); a quick press chooses the tool for good.
#[test]
fn a_hold_shortcut_takes_a_tool_key_from_hold_tool_keys() {
    let mut app = app_with(true);
    assert!(app.session.prefs.hold_tool_keys);
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "S"})).unwrap();
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::S, "s");
    keys.frame(&mut app, 0.4, vec![]);
    assert_eq!(fill(&app), "#0000ff", "the hold runs");
    assert_eq!(app.session.tool_id(), "pen", "Scale is not chosen while S is held");
    keys.release(&mut app, 1.0, Key::S);
    assert_eq!(app.session.tool_id(), "pen");
    keys.press(&mut app, 2.0, Key::S, "s");
    keys.release(&mut app, 2.1, Key::S);
    assert_eq!(app.session.tool_id(), "scale", "a quick press chooses Scale");
    keys.frame(&mut app, 3.0, vec![]);
    assert_eq!(app.session.tool_id(), "scale", "for good");
    assert!(app.held_tool.is_empty());
}

/// A tool key without a hold shortcut still chooses its tool while held when another key has one.
#[test]
fn other_tool_keys_keep_hold_tool_keys() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::S, "s");
    assert_eq!(app.session.tool_id(), "scale");
    keys.frame(&mut app, 1.0, vec![]);
    keys.release(&mut app, 1.1, Key::S);
    assert_eq!(app.session.tool_id(), "pen", "the Pen comes back");
}

/// A hold on Shift and a digit recorded in the dialog (Shift+1) holds when the press types `!`.
#[test]
fn a_shifted_digit_holds_when_it_types_its_symbol() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+1"})).unwrap();
    let keys = Keyboard::new();
    keys.frame(&mut app, 0.0, press(Key::Num1, "!", Modifiers::SHIFT));
    keys.frame(&mut app, 0.4, vec![]);
    assert_eq!(fill(&app), "#0000ff");
}

/// With one entry holding Shift+1 and another `!`, which Shift+1 types, holding Shift+1 runs the
/// entry that holds the key itself.
#[test]
fn a_shifted_digit_holds_before_the_symbol_it_types() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+1"})).unwrap();
    app.run("shortcuts.set", json!({"id": "select.all", "hold": "!"})).unwrap();
    assert_eq!(app.ui.shortcut_holds.len(), 2);
    let keys = Keyboard::new();
    keys.frame(&mut app, 0.0, press(Key::Num1, "!", Modifiers::SHIFT));
    keys.frame(&mut app, 0.4, vec![]);
    assert_eq!(fill(&app), "#0000ff", "Shift+1's hold ran");
}

/// A quick press of a shifted key with a hold shortcut runs what the key runs without one: a
/// shortcut on Shift+. (typing `>`) runs, and Shift+1 (typing `!`) runs what `!` runs.
#[test]
fn a_quick_press_of_a_shifted_key_runs_what_it_runs_without_a_hold() {
    for (key, text, chord) in [(Key::Period, ">", "Shift+."), (Key::Num1, "!", "Shift+1")] {
        let mut app = app_with(true);
        app.run("shortcuts.set", json!({"id": "tool:eyedropper", "shortcut": chord, "force": true})).unwrap();
        let keys = Keyboard::new();
        keys.frame(&mut app, 0.0, press(key, text, Modifiers::SHIFT));
        keys.frame(&mut app, 0.05, vec![release(key)]);
        let without = app.session.tool_id().to_string();
        app.select_tool("pen");
        app.run("shortcuts.set", json!({"id": "paint.swap", "hold": chord})).unwrap();
        keys.frame(&mut app, 1.0, press(key, text, Modifiers::SHIFT));
        assert_eq!(app.session.tool_id(), "pen", "{chord} waits");
        keys.frame(&mut app, 1.05, vec![release(key)]);
        assert_eq!(app.session.tool_id(), without, "a quick press of {chord}");
        assert_eq!(fill(&app), "#ff0000");
    }
}

/// The Shift+. shortcut runs on its press without a hold shortcut, and at the release of a quick
/// press with one.
#[test]
fn a_quick_press_of_shift_period_runs_its_shortcut() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "tool:eyedropper", "shortcut": "Shift+.", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "Shift+."})).unwrap();
    let keys = Keyboard::new();
    keys.frame(&mut app, 0.0, press(Key::Period, ">", Modifiers::SHIFT));
    keys.frame(&mut app, 0.05, vec![release(Key::Period)]);
    assert_eq!(app.session.tool_id(), "eyedropper");
}

/// A text with no key press that typed it does not wait: it has no key to let go.
#[test]
fn a_text_without_its_key_press_does_not_wait() {
    let mut app = erolka_x(true);
    let keys = Keyboard::new();
    keys.frame(&mut app, 0.0, vec![egui::Event::Text("x".into())]);
    assert!(app.hold_keys.pending.is_none());
    assert_eq!(app.session.tool_id(), "eyedropper", "it runs as a single-key shortcut, as before");
}

/// A hold whose command opens a dialog holds back the key's repeats from it.
#[test]
fn a_hold_that_opens_a_dialog_keeps_the_repeats_from_it() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "edit.preferences", "hold": "K"})).unwrap();
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::K, "k");
    keys.frame(&mut app, 0.4, vec![]);
    assert!(app.ui.dialog.is_some(), "the hold opened Preferences");
    let opening = keys.ctx.data(|m| m.get_temp::<Key>(super::super::opening_key_id(&keys.ctx)));
    assert_eq!(opening, Some(Key::K), "K is the key that opened it");
    keys.press(&mut app, 0.5, Key::K, "k");
    assert!(app.hold_keys.swallowed.iter().any(|s| s.key == Key::K), "K's repeats are held back until it is let go");
    // A repeat in the same frame as the release is held back too.
    let mut events = press(Key::K, "k", Modifiers::NONE);
    events.push(release(Key::K));
    let left = keys.frame_left(&mut app, 0.6, events);
    assert!(!left.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })), "{left:?}");
    assert!(app.hold_keys.swallowed.is_empty());
}

/// A key that ends a wait whose tap opens a dialog does not reach the dialog: Enter does not
/// confirm it.
#[test]
fn the_key_that_ends_a_wait_does_not_reach_the_dialog_it_opened() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "edit.preferences", "shortcut": "K", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "K"})).unwrap();
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::K, "k");
    let enter = egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let left = keys.frame_left(&mut app, 0.1, vec![enter]);
    assert!(app.ui.dialog.is_some(), "the tap opened Preferences");
    assert!(!left.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, .. })), "{left:?}");
    assert_eq!(fill(&app), "#ff0000");
}

/// A key typed before a wait that starts and ends in one frame, and that opens a dialog, keeps the
/// frame's later keys from the dialog, the ones before the waiting key included.
#[test]
fn a_key_typed_before_a_wait_does_not_let_later_keys_reach_its_dialog() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "edit.preferences", "shortcut": "D", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
    let keys = Keyboard::new();
    let mut events = press(Key::D, "d", Modifiers::NONE);
    events.extend(press(Key::A, "a", Modifiers::NONE));
    events.extend(press(Key::X, "x", Modifiers::NONE));
    events.push(release(Key::X));
    let left = keys.frame_left(&mut app, 0.0, events);
    assert!(app.ui.dialog.is_some(), "D opened Preferences");
    assert!(!left.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { key: Key::A, pressed: true, .. })), "{left:?}");
    assert_eq!(fill(&app), "#ff0000");
}

/// On the web build, where each press's text comes before it, a key typed before a wait that
/// starts and ends in one frame, and that opens a dialog, keeps the frame's later keys from the
/// dialog too.
#[test]
fn a_key_typed_before_a_wait_on_the_web_build_does_not_let_later_keys_reach_its_dialog() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "edit.preferences", "shortcut": "D", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "X"})).unwrap();
    let keys = Keyboard::new();
    let web = |key: Key, text: &str| {
        [egui::Event::Text(text.into()), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }]
    };
    let mut events = vec![];
    for (key, text) in [(Key::D, "d"), (Key::A, "a"), (Key::X, "x")] {
        events.extend(web(key, text));
    }
    events.push(release(Key::X));
    let left = keys.frame_left(&mut app, 0.0, events);
    assert!(app.ui.dialog.is_some(), "D opened Preferences");
    assert!(!left.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })), "{left:?}");
    assert_eq!(fill(&app), "#ff0000");
}

/// On the web build a press's text comes before it: the text of the key that ends such a wait
/// does not reach the dialog either.
#[test]
fn the_text_before_the_key_that_ends_a_wait_does_not_reach_the_dialog() {
    let mut app = app_with(true);
    app.run("shortcuts.set", json!({"id": "edit.preferences", "shortcut": "K", "force": true})).unwrap();
    app.run("shortcuts.set", json!({"id": "paint.swap", "hold": "K"})).unwrap();
    let keys = Keyboard::new();
    keys.press(&mut app, 0.0, Key::K, "k");
    let m = egui::Event::Key { key: Key::M, physical_key: Some(Key::M), pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let left = keys.frame_left(&mut app, 0.1, vec![egui::Event::Text("m".into()), m]);
    assert!(app.ui.dialog.is_some(), "the tap opened Preferences");
    assert!(!left.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })), "{left:?}");
}
