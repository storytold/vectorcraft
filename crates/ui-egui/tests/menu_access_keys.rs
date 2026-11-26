//! Main-menu access keys: the app's typed tool letters wait while a bar menu is open or the bar
//! holds the keys, through the real App input. The bar's own key rules (mnemonics, Alt and F10)
//! are the framework's and have trials there.
#![cfg(not(target_os = "macos"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use craft::behavior::MotionMode;
use craft::ui::menu::bar::{is_focused, is_open};
use craft::ui::runtime::env::WindowKey;
use craft::ui::shell::{WindowDialogs, embed};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, i18n};

// These cases share the library's process-global locale. A failing case must not poison the
// others, and Drop restores the previous locale even after an assertion.
static UI_STATE: Mutex<()> = Mutex::new(());

struct GlobalState {
    language: i18n::Lang,
    _lock: MutexGuard<'static, ()>,
}

impl GlobalState {
    fn new() -> Self {
        let lock = UI_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let language = i18n::current();
        i18n::set_current(i18n::Lang::EN);
        Self { language, _lock: lock }
    }
}

impl Drop for GlobalState {
    fn drop(&mut self) {
        i18n::set_current(self.language);
    }
}

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
}

impl Scene {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("app.language", json!({"lang": "en"})).unwrap();
        i18n::set_current(i18n::Lang::EN);
        app.run("file.new", json!({"width": 500, "height": 350})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 70, "y": 60, "width": 120, "height": 80})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
        let mut s = Self { app, ctx, time: 0.0 };
        for _ in 0..4 {
            s.frame(vec![]);
        }
        let dialogs = WindowDialogs::of(&s.ctx, &WindowKey::main()).with(|host| host.open_dialogs());
        assert!(dialogs.is_empty(), "no dialog is open");
        assert!(!s.ctx.egui_wants_keyboard_input(), "the initial idle canvas owns input");
        s
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let raw = RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 1000.0))),
            ..Default::default()
        };
        let app = &mut self.app;
        // Motion off: a closed menu leaves at once instead of keeping focus through its exit fade.
        embed::before_input(&self.ctx, |out| {
            let mut input = app.frame_input(out);
            input.root = std::mem::take(&mut input.root).motion(MotionMode::Off);
            input
        });
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.tick(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }

    fn letter(&mut self, key_id: Key, text: &str, modifiers: Modifiers) {
        self.frame(vec![Event::ModifiersChanged(modifiers), key(key_id, true, modifiers), Event::Text(text.into())]);
        self.frame(vec![key(key_id, false, modifiers)]);
    }

    fn click(&mut self, pos: Pos2) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        self.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        self.settle();
    }

    /// A lone Alt: pressed and released with nothing between.
    fn bare_alt(&mut self) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    /// Alt held with `letter`, then released: opens the menu whose mnemonic it is.
    fn alt_letter(&mut self, key_id: Key, text: &str) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        self.letter(key_id, text, Modifiers::ALT);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }
}

#[test]
fn menu_access_custom_shortcut_typing_tool_altgr_and_cancel_ownership() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let outside = Pos2::new(1000.0, 850.0);

    // The user's own Alt+O runs once and does not arm Object.
    let rulers = s.app.ui.view.rulers;
    s.app.run("shortcuts.set", json!({"id": "view.rulers", "shortcut": "Alt+O"})).unwrap();
    s.alt_letter(Key::O, "o");
    assert_eq!(s.app.ui.view.rulers, !rulers, "the user's existing Alt+O shortcut wins, once");
    assert!(!is_open(&s.ctx), "the same chord must not also arm Object");
    s.app.run("shortcuts.set", json!({"id": "view.rulers", "shortcut": null})).unwrap();

    // With the bar idle, a typed letter selects its tool.
    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "pen", "ordinary typing tool shortcut still works");
    s.app.select_tool("selection");

    // Ctrl+Alt (AltGr) is not a bare Alt: it opens and focuses nothing.
    let altgr = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    s.frame(vec![Event::ModifiersChanged(altgr)]);
    s.letter(Key::O, "ø", altgr);
    s.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    s.settle();
    assert!(!is_open(&s.ctx) && !is_focused(&s.ctx), "Ctrl+Alt/AltGr does not become bare Alt navigation");

    // While a title is open, the letter belongs to the menu.
    s.alt_letter(Key::O, "o");
    assert!(is_open(&s.ctx), "Alt+O opens Object");
    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "selection", "an open menu takes the letter");
    // An outside click closes it, and the letter reaches the tool again.
    s.click(outside);
    assert!(!is_open(&s.ctx) && !is_focused(&s.ctx), "an outside pointer click ends menu navigation");
    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "pen", "after the dismissal the letter selects its tool");
    s.app.select_tool("selection");

    // While the bar holds the keys after a lone Alt, the letter does not reach the tool either.
    s.bare_alt();
    s.settle();
    assert!(is_focused(&s.ctx), "a lone Alt gives the bar the keys");
    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "selection", "a focused bar takes the letter");
    s.click(outside);
    assert!(!is_focused(&s.ctx), "an outside pointer click releases the bar");
    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "pen");
}
