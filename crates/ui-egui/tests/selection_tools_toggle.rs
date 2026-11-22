//! User-configurable persistent Selection / Direct Selection switching.
// Tests may unwrap/panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use craft::ui::shell::embed;
use egui::{Event, Key, Modifiers};
use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;
use vectorcraft_geom::PathData;
use vectorcraft_ui_egui::{UiState, VectorcraftApp, i18n};

const SWITCH: &str = "tool.switchSelectionTools";
static CASE: Mutex<()> = Mutex::new(());

// Integration binaries use a process-wide locale. Every case owns the guard and restores the
// language even if an assertion fails.
struct Globals {
    _lock: MutexGuard<'static, ()>,
    language: i18n::Lang,
}
impl Globals {
    fn new() -> Self {
        let lock = CASE.lock().unwrap_or_else(|e| e.into_inner());
        let language = i18n::current();
        i18n::set_current(i18n::Lang::EN);
        Self { _lock: lock, language }
    }
}
impl Drop for Globals {
    fn drop(&mut self) {
        i18n::set_current(self.language);
    }
}

/// The app with a rectangle, Selection chosen, shown in a window that has drawn a frame, where shortcuts are edited.
fn app() -> (VectorcraftApp, NodeId, Keyboard) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "profile": null})).unwrap();
    let shape = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap();
    let id = NodeId(shape["id"].as_u64().unwrap());
    app.select_tool("selection");
    let mut keys = Keyboard::new();
    keys.frame(&mut app, vec![]);
    (app, id, keys)
}

fn path_state(app: &VectorcraftApp, id: NodeId) -> (PathData, Vec<NodeId>, usize) {
    let state = app.session.doc().unwrap();
    (state.doc.node(id).unwrap().path_data().unwrap().clone(), state.selection.objects.clone(), state.history.undo.len())
}

struct Keyboard {
    ctx: egui::Context,
    time: f64,
}
impl Keyboard {
    fn new() -> Self {
        let ctx = egui::Context::default();
        embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
        Self { ctx, time: 0.0 }
    }

    fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<Event>) {
        self.time += 0.05;
        embed::before_input(&self.ctx, |out| app.frame_input(out));
        let mut out = self.ctx.run_ui(
            egui::RawInput {
                events,
                time: Some(self.time),
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            },
            |ui| {
                app.tick(ui.ctx());
                app.ui(ui);
            },
        );
        // The commands the pass fired run as the host's next frame begins.
        embed::before_input(&self.ctx, |out| app.frame_input(out));
        out.textures_delta.clear();
    }

    fn key(&mut self, app: &mut VectorcraftApp, key: Key, pressed: bool, modifiers: Modifiers) {
        let mut events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }];
        // Plain letter shortcuts read the text the native host pairs with the key press.
        // Modified command chords and releases produce no text.
        if pressed && modifiers == Modifiers::NONE {
            match key {
                Key::V => events.push(Event::Text("v".into())),
                Key::A => events.push(Event::Text("a".into())),
                _ => {}
            }
        }
        self.frame(app, events);
    }

    fn tap(&mut self, app: &mut VectorcraftApp, key: Key, modifiers: Modifiers) {
        self.key(app, key, true, modifiers);
        self.key(app, key, false, modifiers);
        self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }
}

fn control() -> Modifiers {
    // Physical Control is the requested chord. On Windows/Linux egui also calls it command.
    Modifiers { ctrl: true, command: !cfg!(target_os = "macos"), ..Modifiers::NONE }
}

fn configure_control_backtick(app: &mut VectorcraftApp) {
    // Main assigns this chord to Select Last Selection Tool. Free that binding explicitly,
    // as a user does in Keyboard Shortcuts, before assigning the configurable toggle.
    app.run("shortcuts.set", json!({"id": "tool.selectLastSelection", "shortcut": ""})).unwrap();
    app.run("shortcuts.set", json!({"id": SWITCH, "shortcut": "Ctrl+~"})).unwrap();
}

#[test]
fn selection_tools_existing_v_and_a_defaults_keep_their_choices() {
    let _globals = Globals::new();
    let (mut app, id, mut keys) = app();
    let before = path_state(&app, id);
    app.select_tool("pen");
    keys.tap(&mut app, Key::V, Modifiers::NONE);
    assert_eq!(app.session.tool_id(), "selection");
    keys.tap(&mut app, Key::A, Modifiers::NONE);
    assert_eq!(app.session.tool_id(), "directSelection");
    assert_eq!(path_state(&app, id), before);
}

#[test]
fn selection_tools_configured_control_backtick_toggles_both_arrows() {
    let _globals = Globals::new();
    let (mut app, id, mut keys) = app();
    let before = path_state(&app, id);
    let rows = app.run("shortcuts.list", json!({"query": SWITCH})).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1, "the configurable command is missing: {rows:?}");
    assert_eq!(rows[0]["id"], SWITCH);
    assert_eq!(rows[0]["label"], "Switch Selection Tools");
    assert!(rows[0]["default"].is_null());
    assert!(rows[0]["shortcut"].is_null());
    configure_control_backtick(&mut app);
    keys.tap(&mut app, Key::Backtick, control());
    assert_eq!(app.session.tool_id(), "directSelection");
    keys.tap(&mut app, Key::Backtick, control());
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(path_state(&app, id), before, "idle tool choices preserve path, selection and history");
}

#[test]
fn selection_tools_saved_binding_runs_and_reset_removes_it() {
    let _globals = Globals::new();
    let (mut original, _, _) = app();
    configure_control_backtick(&mut original);
    let saved = serde_json::to_string(&original.ui).unwrap();
    let (mut restored, id, mut keys) = app();
    // As the host loads saved preferences: the UI state, then what it restores.
    restored.ui = serde_json::from_str::<UiState>(&saved).unwrap();
    vectorcraft_ui_egui::prefs_dialog::restore(&mut restored);
    let before = path_state(&restored, id);
    keys.tap(&mut restored, Key::Backtick, control());
    assert_eq!(restored.session.tool_id(), "directSelection");
    restored.run("shortcuts.set", json!({"id": SWITCH, "shortcut": null})).unwrap();
    let rows = restored.run("shortcuts.list", json!({"query": SWITCH})).unwrap();
    assert!(rows[0]["shortcut"].is_null());
    assert_eq!(rows[0]["overridden"], false);
    keys.tap(&mut restored, Key::Backtick, control());
    assert_eq!(restored.session.tool_id(), "directSelection", "reset leaves this chord unbound");
    assert_eq!(path_state(&restored, id), before);
}

#[test]
fn selection_tools_other_tools_enter_selection_and_explicit_choice_survives_release() {
    let _globals = Globals::new();
    let (mut app, id, mut keys) = app();
    let before = path_state(&app, id);
    for third in ["pen", "groupSelection"] {
        app.select_tool(third);
        assert_eq!(app.run(SWITCH, json!({})).unwrap()["tool"], "selection");
        assert_eq!(app.session.tool_id(), "selection");
        assert_eq!(app.run(SWITCH, json!({})).unwrap()["tool"], "directSelection");
        assert_eq!(app.session.tool_id(), "directSelection");
    }
    configure_control_backtick(&mut app);
    app.select_tool("pen");
    keys.key(&mut app, Key::V, true, Modifiers::NONE);
    assert_eq!(app.session.tool_id(), "selection");
    keys.tap(&mut app, Key::Backtick, control());
    assert_eq!(app.session.tool_id(), "directSelection");
    // Keep V down longer than the existing 0.25s temporary-tool threshold. Without clearing
    // the held choice, its release would restore Pen instead of respecting this command.
    for _ in 0..6 {
        keys.frame(&mut app, vec![]);
    }
    keys.key(&mut app, Key::V, false, Modifiers::NONE);
    assert_eq!(app.session.tool_id(), "directSelection");
    assert_eq!(path_state(&app, id), before);
}
