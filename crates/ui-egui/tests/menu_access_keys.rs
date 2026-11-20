//! Main-menu access keys use the real App input and existing dialog routes.
#![cfg(not(target_os = "macos"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, i18n};

// These cases share the library's process-global locale. A failing baseline must not poison
// the passing pointer control, and Drop restores the previous locale even after an assertion.
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

fn text_points(shape: &Shape, clip: Rect, out: &mut Vec<(String, Pos2)>) {
    match shape {
        Shape::Text(t) => {
            let point = t.pos + vec2(4.0, 6.0);
            if clip.contains(point) {
                out.push((t.galley.text().to_string(), point));
            }
        }
        Shape::Vec(children) => children.iter().for_each(|shape| text_points(shape, clip, out)),
        _ => {}
    }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    shown: Vec<(String, Pos2)>,
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
        vectorcraft_ui_egui::theme::install_fonts(&ctx);
        vectorcraft_ui_egui::theme::apply(&ctx, Default::default());
        let mut s = Self { app, ctx, time: 0.0, shown: vec![] };
        for _ in 0..4 {
            s.frame(vec![]);
        }
        assert!(s.app.ui.dialog.is_none());
        assert!(!s.ctx.egui_wants_keyboard_input(), "the initial idle canvas owns input");
        s
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let mut raw = RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 1000.0))),
            ..Default::default()
        };
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut out = self.ctx.run_ui(raw, |ui| self.app.ui(ui));
        self.shown.clear();
        for clipped in &out.shapes {
            text_points(&clipped.shape, clipped.clip_rect, &mut self.shown);
        }
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

    fn arrow(&mut self, key_id: Key) {
        self.frame(vec![key(key_id, true, Modifiers::NONE)]);
        self.frame(vec![key(key_id, false, Modifiers::NONE)]);
    }

    fn point(&self, label: &str, popup: bool) -> Pos2 {
        let points: Vec<_> = self
            .shown
            .iter()
            .filter(|(text, point)| {
                text.trim() == label && (!popup || self.ctx.layer_id_at(*point).is_some_and(|layer| layer.order == egui::Order::Foreground))
            })
            .map(|(_, point)| *point)
            .collect();
        assert_eq!(points.len(), 1, "one visible {label:?} label required; found {points:?}");
        points[0]
    }

    fn click(&mut self, pos: Pos2) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        self.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        self.settle();
    }

    fn assert_offset_dialog(&self) {
        let d = self.app.ui.dialog.as_ref().expect("menu navigation must open a dialog");
        assert_eq!(d.kind, "offsetPath", "must invoke the existing Offset Path dialog, not another command");
        assert!(!d.bool("preview"), "a fresh Offset Path dialog opens with Preview off");
        assert!(self.shown.iter().any(|(text, _)| text == "Offset Path"), "the actual existing dialog must be drawn");
    }
}

#[test]
fn menu_access_baseline_alt_sequence_opens_offset_path_without_editing() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let state = s.app.session.active().unwrap();
    let (document, selection) = (state.doc.clone(), state.selection.clone());
    let history = (state.history.undo.len(), state.history.redo.len());
    s.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
    s.letter(Key::O, "o", Modifiers::ALT);
    s.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    s.letter(Key::P, "p", Modifiers::NONE);
    s.arrow(Key::ArrowRight);
    s.letter(Key::O, "o", Modifiers::NONE);
    s.settle();
    s.assert_offset_dialog();
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, document, "opening the menu's dialog must not edit the art");
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
}

#[test]
fn menu_access_baseline_pointer_route_opens_same_offset_path_dialog() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let state = s.app.session.active().unwrap();
    let (document, selection) = (state.doc.clone(), state.selection.clone());
    let history = (state.history.undo.len(), state.history.redo.len());
    s.click(s.point("Object", false));
    let path = s.point("Path", true);
    s.frame(vec![Event::PointerMoved(path)]);
    s.settle();
    s.click(s.point("Offset Path\u{2026}", true));
    s.assert_offset_dialog();
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
}

// Prospective full-contract cases. The baseline above stays byte-for-byte unchanged.
#[derive(Debug)]
struct Cue {
    label: String,
    character: char,
    popup: bool,
}

fn clean_label(text: &str) -> &str {
    text.trim().trim_start_matches('\u{2713}').trim_start()
}

fn painted_cues(shape: &Shape, clip: Rect, ctx: &egui::Context, out: &mut Vec<Cue>) {
    match shape {
        Shape::Text(t) => {
            let point = t.pos + vec2(4.0, 6.0);
            if !clip.contains(point) {
                return;
            }
            for section in &t.galley.job.sections {
                if section.format.underline.width <= 0.0 {
                    continue;
                }
                let marked =
                    t.galley.job.text.get(section.byte_range.start.0..section.byte_range.end.0).expect("underline uses valid UTF-8 boundaries");
                let mut chars = marked.chars();
                let Some(character) = chars.next() else { continue };
                // Other widgets may underline whole strings. Only a single-character menu cue
                // satisfies the assertions below; do not mistake unrelated styling for a cue.
                if chars.next().is_some() {
                    continue;
                }
                out.push(Cue {
                    label: clean_label(t.galley.text()).into(),
                    character,
                    popup: ctx.layer_id_at(point).is_some_and(|layer| layer.order == egui::Order::Foreground),
                });
            }
        }
        Shape::Vec(children) => children.iter().for_each(|shape| painted_cues(shape, clip, ctx, out)),
        _ => {}
    }
}

impl Scene {
    fn cues(&mut self) -> Vec<Cue> {
        self.at_size(vec![], vec2(1440.0, 1000.0))
    }

    fn at_size(&mut self, events: Vec<Event>, size: egui::Vec2) -> Vec<Cue> {
        self.at_size_focused(events, size, true)
    }

    fn at_size_focused(&mut self, events: Vec<Event>, size: egui::Vec2, focused: bool) -> Vec<Cue> {
        self.time += 1.0 / 30.0;
        let mut raw =
            RawInput { events, focused, time: Some(self.time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut out = self.ctx.run_ui(raw, |ui| self.app.ui(ui));
        self.shown.clear();
        let mut cues = vec![];
        for clipped in &out.shapes {
            text_points(&clipped.shape, clipped.clip_rect, &mut self.shown);
            painted_cues(&clipped.shape, clipped.clip_rect, &self.ctx, &mut cues);
        }
        out.textures_delta.clear();
        cues
    }

    fn window_focus(&mut self, focused: bool) {
        // egui-winit supplies both the RawInput field and the event on native focus changes.
        self.at_size_focused(vec![Event::WindowFocused(focused)], vec2(1440.0, 1000.0), focused);
    }

    fn menu_diagnostic(&self) -> String {
        let shown: Vec<_> = self
            .shown
            .iter()
            .filter(|(_, point)| point.y < 40.0 || self.ctx.layer_id_at(*point).is_some_and(|layer| layer.order == egui::Order::Foreground))
            .take(36)
            .collect();
        let focus = self.ctx.memory(|m| m.focused()).and_then(|id| self.ctx.read_response(id)).map(|r| r.rect);
        format!(
            "pref={} ui={} global={} input_focused={} focus_rect={focus:?} shown={shown:?}",
            self.app.session.prefs.interface_language,
            self.app.ui_language().code(),
            i18n::current().code(),
            self.ctx.input(|i| i.focused)
        )
    }

    fn bare_alt(&mut self) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }

    fn root(&mut self, title: &str) {
        let (key, text) = match title {
            "File" => (Key::F, "f"),
            "Edit" => (Key::E, "e"),
            "Object" => (Key::O, "o"),
            "Type" => (Key::T, "t"),
            "Select" => (Key::S, "s"),
            "Effect" => (Key::C, "c"),
            "View" => (Key::V, "v"),
            "Window" => (Key::W, "w"),
            "Help" => (Key::H, "h"),
            _ => panic!("unexpected root {title}"),
        };
        self.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        self.letter(key, text, Modifiers::ALT);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.settle();
    }

    fn end_access(&mut self) {
        for _ in 0..12 {
            let active = egui::Popup::is_any_open(&self.ctx)
                || self.cues().iter().any(|c| {
                    !c.popup && ["File", "Edit", "Object", "Type", "Select", "Effect", "View", "Window", "Help"].contains(&c.label.as_str())
                });
            if !active {
                return;
            }
            self.arrow(Key::Escape);
            self.settle();
        }
        panic!("menu navigation did not close after bounded Escape presses");
    }

    fn advertised(&mut self, label: &str, invoke: bool) {
        let cues = self.cues();
        let matches: Vec<_> = cues.iter().filter(|c| c.popup && c.label == label).collect();
        assert_eq!(matches.len(), 1, "one painted access cue for {label:?}: {cues:?}; {}", self.menu_diagnostic());
        let c = matches[0].character;
        let text = c.to_lowercase().collect::<String>();
        let key = Key::from_name(&c.to_uppercase().collect::<String>()).expect("this English menu has a native letter key");
        // A painted letter need not be unique. Follow the advertised collision interaction,
        // checking the real focused row, instead of assuming the first press names this label.
        let bound = cues.iter().filter(|cue| cue.popup && cue.character.to_lowercase().eq(c.to_lowercase())).count() + 1;
        for _ in 0..bound {
            self.letter(key, &text, Modifiers::NONE);
            self.settle();
            if !egui::Popup::is_any_open(&self.ctx) {
                assert!(invoke, "submenu {label:?} must focus rather than invoke another command; {}", self.menu_diagnostic());
                return;
            }
            if focused_label(self, &[label]).as_deref() == Some(label) {
                if invoke {
                    self.arrow(Key::Enter);
                    self.settle();
                }
                return;
            }
        }
        panic!("painted access key could not focus {label:?}; {}", self.menu_diagnostic());
    }

    fn command_from_menu(&mut self, id: &str) {
        let entries = vectorcraft_ui_egui::menus::menu_entries(&self.app);
        let matches: Vec<_> = entries.iter().filter(|e| e.command.as_deref() == Some(id)).collect();
        assert_eq!(matches.len(), 1, "unambiguous existing menu command {id}");
        let row = matches[0];
        let (path, label) = (row.path.clone(), row.label.clone());
        assert!(row.enabled);
        self.root(&path[0]);
        for submenu in path.iter().skip(1) {
            self.advertised(submenu, false);
            self.arrow(Key::ArrowRight);
            self.settle();
        }
        self.advertised(&label, true);
    }

    fn assert_root_contents(&self, root: &str) {
        use vectorcraft_ui_egui::menus::{self, Entry};
        let tree = menus::menu_tree_named(self.app.session.prefs.font_names_in_english);
        let (_, items) = tree.iter().find(|(title, _)| *title == root).unwrap();
        let labels: Vec<_> = items
            .iter()
            .filter_map(|item| match menus::entry(&self.app, item)? {
                Entry::Sub(label, _) => Some(label.to_string()),
                Entry::Item(row) => Some(row.label.to_string()),
                _ => None,
            })
            .take(3)
            .collect();
        assert!(!labels.is_empty());
        for label in labels {
            assert!(
                self.shown.iter().any(|(text, point)| clean_label(text) == label
                    && self.ctx.layer_id_at(*point).is_some_and(|layer| layer.order == egui::Order::Foreground)),
                "visible {root} popup must contain {label:?}"
            );
        }
    }
}

#[test]
fn menu_access_all_roots_bare_alt_and_one_existing_command() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let before = s.app.session.active().unwrap();
    let (doc, selection, undo) = (before.doc.clone(), before.selection.clone(), before.history.undo.len());
    s.bare_alt();
    let roots = [
        ("File", 'F'),
        ("Edit", 'E'),
        ("Object", 'O'),
        ("Type", 'T'),
        ("Select", 'S'),
        ("Effect", 'C'),
        ("View", 'V'),
        ("Window", 'W'),
        ("Help", 'H'),
    ];
    let cues = s.cues();
    for (label, letter) in roots {
        assert!(cues.iter().any(|c| !c.popup && c.label == label && c.character.eq_ignore_ascii_case(&letter)), "painted root cue {label}/{letter}");
    }
    s.bare_alt();
    assert!(!s.cues().iter().any(|c| roots.iter().any(|(title, _)| c.label == *title)));
    for (title, _) in roots {
        s.root(title);
        s.assert_root_contents(title);
        s.end_access();
    }
    let rulers = s.app.ui.view.rulers;
    s.command_from_menu("view.rulers");
    assert_eq!(s.app.ui.view.rulers, !rulers, "one menu activation toggles once");
    s.settle();
    assert_eq!(s.app.ui.view.rulers, !rulers, "idle passes cannot repeat the command");
    let after = s.app.session.active().unwrap();
    assert_eq!(after.doc, doc);
    assert_eq!(after.selection, selection);
    assert_eq!(after.history.undo.len(), undo);
}

#[test]
fn menu_access_localized_latin_and_cyrillic_painted_text_pairs() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    for (locale, expected) in [("fr", "Fichier"), ("ru", "Файл")] {
        s.app.run("app.language", json!({"lang": locale})).unwrap();
        i18n::set_current(i18n::Lang::from_code(locale).unwrap());
        s.settle();
        assert_eq!(s.app.session.prefs.interface_language, locale);
        assert_eq!(s.app.ui_language().code(), locale);
        s.bare_alt();
        let cues = s.cues();
        let cue = cues
            .iter()
            .find(|c| !c.popup && c.label == expected)
            .unwrap_or_else(|| panic!("translated {locale} File root {expected:?} has no visible cue: {cues:?}; {}", s.menu_diagnostic()));
        assert!(expected.chars().any(|c| c.to_lowercase().eq(cue.character.to_lowercase())), "cue is an actual translated character");
        let text = cue.character.to_lowercase().collect::<String>();
        // Standard Russian physical positions send these Latin Key values with Cyrillic Text.
        // Inter's bundled cmap contains every glyph of Файл; no substituted ASCII label is used.
        let key = if locale == "ru" {
            match text.as_str() {
                "ф" => Key::A,
                "а" => Key::F,
                "й" => Key::Q,
                "л" => Key::K,
                _ => panic!("unexpected File cue {text}"),
            }
        } else {
            Key::from_name(&text.to_uppercase()).unwrap()
        };
        s.letter(key, &text, Modifiers::NONE);
        s.settle();
        s.assert_root_contents("File");
        // Escape must end localized navigation too, independently of English labels.
        for _ in 0..3 {
            s.arrow(Key::Escape);
        }
        s.settle();
        assert!(!egui::Popup::is_any_open(&s.ctx));
        assert!(s.app.ui.dialog.is_none(), "one Text/Key pair opens only the root");
    }
}

fn focused_label(s: &Scene, labels: &[&str]) -> Option<String> {
    let focused = s.ctx.memory(|m| m.focused())?;
    let response = s.ctx.read_response(focused)?;
    s.shown
        .iter()
        .find(|(text, point)| labels.contains(&clean_label(text)) && response.rect.contains(*point))
        .map(|(text, _)| clean_label(text).into())
}

#[test]
fn menu_access_disabled_rows_dynamic_collisions_and_deleted_focus() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    s.app.run("select.set", json!({"ids": []})).unwrap();
    let doc = s.app.session.active().unwrap().doc.clone();
    s.root("Object");
    s.letter(Key::P, "p", Modifiers::NONE);
    s.arrow(Key::ArrowRight);
    s.letter(Key::O, "o", Modifiers::NONE);
    s.settle();
    assert!(s.app.ui.dialog.is_none(), "disabled Offset Path cannot be invoked by its access key");
    assert_eq!(s.app.session.active().unwrap().doc, doc);
    s.end_access();

    let first = s.app.run("shape.rectangle", json!({"x": 220, "y": 70, "width": 30, "height": 40})).unwrap()["id"].clone();
    s.app.run("select.set", json!({"ids": [first]})).unwrap();
    let selection_a = s.app.session.active().unwrap().selection.clone();
    s.app.run("select.save", json!({"name": "QQQQ"})).unwrap();
    let second = s.app.run("shape.rectangle", json!({"x": 290, "y": 90, "width": 40, "height": 30})).unwrap()["id"].clone();
    s.app.run("select.set", json!({"ids": [second]})).unwrap();
    let selection_b = s.app.session.active().unwrap().selection.clone();
    s.app.run("select.save", json!({"name": "QQQ"})).unwrap();
    s.app.run("select.set", json!({"ids": []})).unwrap();
    let empty = s.app.session.active().unwrap().selection.clone();
    s.root("Select");
    let cues = s.cues();
    for label in ["QQQQ", "QQQ"] {
        assert!(
            cues.iter().any(|c| c.popup && c.label == label && c.character.eq_ignore_ascii_case(&'Q')),
            "the unavoidable {label} collision is visibly advertised"
        );
    }
    s.letter(Key::Q, "q", Modifiers::NONE);
    s.settle();
    let first_focus = focused_label(&s, &["QQQQ", "QQQ"]).expect("collision focuses one saved selection");
    assert_eq!(s.app.session.active().unwrap().selection, empty, "a collision letter does not execute");
    s.letter(Key::Q, "q", Modifiers::NONE);
    s.settle();
    let next_focus = focused_label(&s, &["QQQQ", "QQQ"]).unwrap();
    assert_ne!(first_focus, next_focus, "repeated letter cycles the two real dynamic rows");
    s.arrow(Key::Enter);
    s.settle();
    assert_eq!(s.app.session.active().unwrap().selection, if next_focus == "QQQQ" { selection_a } else { selection_b });

    s.app.run("select.set", json!({"ids": []})).unwrap();
    s.root("Select");
    for _ in 0..2 {
        s.letter(Key::Q, "q", Modifiers::NONE);
        s.settle();
        if focused_label(&s, &["QQQQ", "QQQ"]).as_deref() == Some("QQQQ") {
            break;
        }
    }
    assert_eq!(focused_label(&s, &["QQQQ", "QQQ"]).as_deref(), Some("QQQQ"));
    s.app.run("select.editSaved", json!({"name": "QQQQ", "delete": true})).unwrap();
    s.settle();
    assert!(!s.shown.iter().any(|(text, _)| clean_label(text) == "QQQQ"));
    s.arrow(Key::Enter);
    s.settle();
    assert_eq!(s.app.session.active().unwrap().selection, empty, "deleting the focused row cannot invoke the next saved-selection slot");
    assert!(s.app.ui.dialog.is_none());
}

fn field_range(s: &Scene, id: egui::Id) -> egui::text::CCursorRange {
    egui::TextEdit::load_state(&s.ctx, id).unwrap().cursor.char_range().unwrap()
}

impl Scene {
    fn focus_numeric(&mut self) -> egui::Id {
        // Unit::format includes its suffix; choose an actual original rectangle field by its number.
        let mut candidates: Vec<_> = self
            .shown
            .iter()
            .filter(|(text, _)| text.split_whitespace().next().is_some_and(|number| ["70", "60", "120", "80"].contains(&number)))
            .map(|(_, p)| *p)
            .collect();
        candidates.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        candidates.dedup();
        for point in candidates {
            self.click(point);
            if let Some(field) = self.ctx.memory(|m| m.focused()).filter(|id| egui::TextEdit::load_state(&self.ctx, *id).is_some()) {
                return field;
            }
        }
        panic!("an actual selected-rectangle numeric field must be focusable");
    }
}

#[test]
#[allow(deprecated)] // ImeEvent::Disabled is a supported host cancellation event in this regression.
fn menu_access_real_numeric_field_cancel_and_edit_cut_keep_art_unchanged() {
    let _global = GlobalState::new();
    {
        let mut ime = Scene::new();
        ime.focus_numeric();
        ime.frame(vec![Event::Ime(egui::ImeEvent::Preedit { text: "ф".into(), active_range_chars: Some(0..1) })]);
        ime.bare_alt();
        assert!(!ime.cues().iter().any(|c| !c.popup && c.label == "File"), "active composition keeps Alt ownership");
        ime.frame(vec![Event::Ime(egui::ImeEvent::Disabled)]);
        ime.settle();
        ime.root("Object");
        ime.letter(Key::P, "p", Modifiers::NONE);
        ime.arrow(Key::ArrowRight);
        ime.letter(Key::O, "o", Modifiers::NONE);
        ime.settle();
        ime.assert_offset_dialog(); // No Commit was sent: terminal cancellation really releases the latch.
    }
    let mut s = Scene::new();
    // These are the original rectangle's actual Transform/Control numeric fields, not a test UI.
    let field = s.focus_numeric();
    s.arrow(Key::Home);
    s.arrow(Key::ArrowRight);
    let caret = field_range(&s, field);
    assert!(caret.is_empty());
    let state = s.app.session.active().unwrap();
    let (doc, selection, undo) = (state.doc.clone(), state.selection.clone(), state.history.undo.len());
    s.bare_alt();
    s.arrow(Key::Escape);
    s.settle();
    assert_eq!(s.ctx.memory(|m| m.focused()), Some(field), "cancelling access mode restores the former field");
    assert_eq!(field_range(&s, field), caret, "the original caret is restored too");
    s.bare_alt();
    let altgr = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    s.frame(vec![Event::ModifiersChanged(altgr), Event::Text("ø".into())]);
    s.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    s.settle();
    assert_eq!(s.ctx.memory(|m| m.focused()), Some(field), "Text-only AltGr restores the original field before delivery");
    assert_eq!(field_range(&s, field).primary.index, caret.primary.index + 1, "the actual field receives the Unicode Text event without a Key event");
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"), "AltGr cancels active menu mode");
    s.arrow(Key::Home);
    s.frame(vec![Event::ModifiersChanged(Modifiers::SHIFT), key(Key::End, true, Modifiers::SHIFT)]);
    s.frame(vec![key(Key::End, false, Modifiers::SHIFT), Event::ModifiersChanged(Modifiers::NONE)]);
    assert!(!field_range(&s, field).is_empty());
    s.command_from_menu("edit.cut");
    s.settle();
    assert_eq!(s.ctx.memory(|m| m.focused()), Some(field), "Edit Cut is delivered to the original field");
    let cut = field_range(&s, field);
    assert_eq!(
        (cut.primary.index, cut.secondary.index),
        (egui::text::CharIndex::ZERO, egui::text::CharIndex::ZERO),
        "the selected numeric text was cut"
    );
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, doc, "cutting uncommitted field text must not cut selected artwork");
    assert_eq!(state.selection, selection);
    assert_eq!(state.history.undo.len(), undo);
    s.letter(Key::Num9, "9", Modifiers::NONE);
    s.letter(Key::Num5, "5", Modifiers::NONE);
    assert_eq!(field_range(&s, field).primary.index, egui::text::CharIndex(2), "normal Text/Key input resumes in the same field after menu editing");
}

struct KeymapRestore(vectorcraft_ui_egui::UiState);

impl Drop for KeymapRestore {
    fn drop(&mut self) {
        vectorcraft_ui_egui::shortcut_editor::sync(&self.0);
    }
}

#[test]
fn menu_access_custom_shortcut_typing_tool_altgr_and_cancel_ownership() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let _restore = KeymapRestore(s.app.ui.clone());
    let rulers = s.app.ui.view.rulers;
    s.app.run("shortcuts.set", json!({"id": "view.rulers", "shortcut": "Alt+O"})).unwrap();
    s.root("Object");
    let once = s.app.ui.view.rulers;
    let open = egui::Popup::is_any_open(&s.ctx);
    s.app.run("shortcuts.set", json!({"id": "view.rulers", "shortcut": null})).unwrap();
    assert_eq!(once, !rulers, "the user's existing Alt+O shortcut wins");
    assert!(!open, "the same chord must not also arm Object");

    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "pen", "ordinary typing tool shortcut still works");
    s.app.select_tool("selection");
    let altgr = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    s.frame(vec![Event::ModifiersChanged(altgr)]);
    s.letter(Key::O, "ø", altgr);
    s.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    s.settle();
    assert!(!egui::Popup::is_any_open(&s.ctx));
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "Object"), "Ctrl+Alt/AltGr does not become bare Alt navigation");

    s.bare_alt();
    s.window_focus(false);
    s.window_focus(true);
    s.settle();
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"), "window focus loss cancels access mode");
    s.bare_alt();
    s.click(Pos2::new(1000.0, 850.0));
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"), "an outside pointer click cancels access mode");
    for key_id in [Key::Tab, Key::Space] {
        s.frame(vec![Event::ModifiersChanged(Modifiers::ALT), key(key_id, true, Modifiers::ALT)]);
        s.frame(vec![key(key_id, false, Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)]);
        s.settle();
        assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"), "Alt+Tab/Space does not become an unused Alt release");
    }
    s.app.select_tool("rectangle");
    // Ordinary points inside the wide fixture's real central canvas: use the actual App pointer
    // route, with a busy-tool precondition, rather than merely selecting an idle tool.
    let (start, end) = (Pos2::new(600.0, 500.0), Pos2::new(650.0, 540.0));
    s.frame(vec![
        Event::ModifiersChanged(Modifiers::ALT),
        Event::PointerMoved(start),
        Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT },
    ]);
    s.frame(vec![Event::PointerMoved(end)]);
    assert!(s.app.session.tool_busy(), "the actual rectangle drag must have started");
    s.bare_alt();
    assert_eq!(s.app.session.tool_id(), "rectangle");
    assert!(s.app.session.tool_busy(), "Alt access cannot finish or replace the active gesture");
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"));
    s.frame(vec![Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    s.settle();
    assert!(!s.app.session.tool_busy(), "normal pointer release completes the same gesture");
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"), "using Alt with the pointer cannot arm a later unused-Alt release");
    s.app.select_tool("selection");
    s.app.run("select.all", json!({})).unwrap();
    s.command_from_menu("object.path.offsetPath");
    s.assert_offset_dialog();
    s.root("Edit");
    assert_eq!(s.app.ui.dialog.as_ref().unwrap().kind, "offsetPath", "an existing modal dialog keeps keyboard ownership");
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "Edit"));
}

#[test]
fn menu_access_stationary_pointer_repeat_and_rerun_do_not_retarget_or_double_invoke() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let object = s.point("Object", false);
    s.frame(vec![Event::PointerMoved(object)]);
    s.root("File");
    s.assert_root_contents("File");
    s.settle();
    s.assert_root_contents("File");
    s.frame(vec![Event::PointerMoved(s.point("Edit", false))]);
    s.settle();
    s.assert_root_contents("Edit");
    s.end_access();
    s.frame(vec![Event::PointerGone]);
    s.root("Object");
    s.frame(vec![
        Event::ModifiersChanged(Modifiers::ALT),
        Event::Key { key: Key::O, physical_key: Some(Key::O), pressed: true, repeat: true, modifiers: Modifiers::ALT },
        Event::Text("o".into()),
    ]);
    s.frame(vec![key(Key::O, false, Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)]);
    s.settle();
    s.assert_root_contents("Object");
    s.end_access();

    let entries = vectorcraft_ui_egui::menus::menu_entries(&s.app);
    let row = entries.iter().find(|e| e.command.as_deref() == Some("view.rulers")).unwrap();
    let (path, label) = (row.path.clone(), row.label.clone());
    s.root(&path[0]);
    for submenu in path.iter().skip(1) {
        s.advertised(submenu, false);
        s.arrow(Key::ArrowRight);
        s.settle();
    }
    let cues = s.cues();
    let cue = cues
        .iter()
        .find(|c| c.popup && c.label == label)
        .unwrap_or_else(|| panic!("rerun command {label:?} has no painted cue: {cues:?}; {}", s.menu_diagnostic()));
    let text = cue.character.to_lowercase().collect::<String>();
    let key_id = Key::from_name(&text.to_uppercase()).unwrap();
    let before = s.app.ui.view.rulers;
    s.time += 1.0 / 30.0;
    let mut raw = RawInput {
        time: Some(s.time),
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 1000.0))),
        events: vec![Event::ModifiersChanged(Modifiers::NONE), key(key_id, true, Modifiers::NONE), Event::Text(text)],
        ..Default::default()
    };
    s.app.raw_input_hook(&s.ctx, &mut raw);
    let mut passes = 0;
    let mut out = s.ctx.run_ui(raw, |ui| {
        passes += 1;
        s.app.ui(ui);
        if passes == 1 {
            ui.ctx().request_discard("exercise the real App's once-per-frame menu input");
        }
    });
    out.textures_delta.clear();
    s.frame(vec![key(key_id, false, Modifiers::NONE)]);
    s.settle();
    assert!(passes >= 2, "the test must actually exercise a UI rerun");
    assert_eq!(s.app.ui.view.rulers, !before, "one Text/Key pair invokes exactly once across passes");

    s.end_access();
    // Wait for any background font scan and ensure > 20 families even on minimal runners.
    let db = vectorcraft_text::FontDb::global();
    let _ = db.families();
    for i in 0..24 {
        if db.menu_family_list_now().len() > 20 {
            break;
        }
        // `renamed` needs a family name as long as "Source Sans 3" (13 bytes).
        db.add_font(vectorcraft_testkit::fonts::renamed(&format!("Menu Test F{i:02}")));
    }
    let text = s.app.run("text.create", json!({"x": 250, "y": 200, "text": "Menu", "font": "Inter", "size": 12})).unwrap()["id"].clone();
    s.app.run("select.set", json!({"ids": [text]})).unwrap();
    let size = vec2(640.0, 420.0);
    for _ in 0..4 {
        s.at_size(vec![], size);
    }
    s.at_size(vec![Event::ModifiersChanged(Modifiers::ALT), key(Key::T, true, Modifiers::ALT), Event::Text("t".into())], size);
    s.at_size(vec![key(Key::T, false, Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)], size);
    let cues = s.at_size(vec![], size);
    let font = cues.iter().find(|c| c.popup && c.label == "Font").expect("narrow Type menu exposes its Font submenu");
    let letter = font.character.to_lowercase().collect::<String>();
    let font_key = Key::from_name(&letter.to_uppercase()).unwrap();
    s.at_size(vec![key(font_key, true, Modifiers::NONE), Event::Text(letter)], size);
    s.at_size(vec![key(font_key, false, Modifiers::NONE)], size);
    s.at_size(vec![key(Key::ArrowRight, true, Modifiers::NONE)], size);
    s.at_size(vec![key(Key::ArrowRight, false, Modifiers::NONE)], size);
    let entries = vectorcraft_ui_egui::menus::menu_entries(&s.app);
    let font_labels: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.path.first().is_some_and(|p| p == "Type") && e.path.iter().any(|p| p == "Font") && e.command.as_deref() == Some("text.setStyle")
        })
        .map(|e| e.label.clone())
        .collect();
    assert!(font_labels.len() > 20, "the inherited bundled font fixture must produce a genuinely long submenu");
    let first = s.ctx.memory(|m| m.focused()).expect("Font navigation has a genuine focused menu response");
    let viewport = Rect::from_min_size(Pos2::ZERO, size);
    // Right can leave global focus on the Font parent until native Down navigation enters its
    // children. Locate the pointer from an actually painted family cue instead of that parent.
    let family_cues = s.at_size(vec![], size);
    let (pointer_family, pointer, wheel_layer) = s
        .shown
        .iter()
        .filter_map(|(text, point)| {
            let label = clean_label(text);
            if !font_labels.iter().any(|family| family == label) || !family_cues.iter().any(|cue| cue.popup && cue.label == label) {
                return None;
            }
            let layer = s.ctx.layer_id_at(*point)?;
            (layer.order == egui::Order::Foreground).then_some((label.to_owned(), *point, layer))
        })
        .min_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)))
        .unwrap_or_else(|| panic!("wheel precondition requires an actual painted Font family: {family_cues:?}; {}", s.menu_diagnostic()));
    assert!(
        viewport.contains(pointer) && s.ctx.layer_id_at(pointer) == Some(wheel_layer),
        "wheel point for {pointer_family:?} is inside the actual visible Font popup"
    );
    s.at_size(vec![Event::PointerMoved(pointer)], size);
    let focus_before_rearm = s.ctx.memory(|m| m.focused());
    // Movement alone already gives the old implementation pointer ownership. A real navigation
    // key must re-arm keyboard ownership with that pointer stationary INSIDE the same popup.
    s.at_size(vec![key(Key::ArrowDown, true, Modifiers::NONE)], size);
    s.at_size(vec![key(Key::ArrowDown, false, Modifiers::NONE)], size);
    s.at_size(vec![], size);
    let rearmed_focus = s.ctx.memory(|m| m.focused()).expect("real Down navigation retains menu focus");
    assert_ne!(Some(rearmed_focus), focus_before_rearm, "the real re-arming Down press advances Font focus before the wheel");
    let rearmed_row = s.ctx.read_response(rearmed_focus).unwrap();
    assert!(
        s.shown.iter().any(|(text, point)| font_labels.contains(&clean_label(text).to_owned())
            && rearmed_row.rect.contains(*point)
            && s.ctx.layer_id_at(*point) == Some(wheel_layer)),
        "re-armed keyboard focus remains on a visible Font family in the same popup"
    );
    assert_eq!(s.ctx.pointer_hover_pos(), Some(pointer));
    assert_eq!(s.ctx.layer_id_at(pointer), Some(wheel_layer));
    let top_family = |scene: &Scene, labels: &[String]| {
        scene
            .shown
            .iter()
            .filter(|(text, point)| labels.contains(&clean_label(text).to_owned()) && scene.ctx.layer_id_at(*point) == Some(wheel_layer))
            .map(|(text, point)| (clean_label(text).to_owned(), *point))
            .min_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)))
            .expect("the real Font popup has a visible top family")
    };
    let wheel_top_before = top_family(&s, &font_labels);
    let wheel_state = s.app.session.active().unwrap();
    let (wheel_doc, wheel_selection, wheel_history) =
        (wheel_state.doc.clone(), wheel_state.selection.clone(), (wheel_state.history.undo.len(), wheel_state.history.redo.len()));
    s.at_size(
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -80.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        }],
        size,
    );
    for _ in 0..10 {
        s.at_size(vec![], size);
    }
    let wheel_top_after = top_family(&s, &font_labels);
    let before_index = font_labels.iter().position(|label| label == &wheel_top_before.0).unwrap();
    let after_index = font_labels.iter().position(|label| label == &wheel_top_after.0).unwrap();
    assert!(
        after_index > before_index || (wheel_top_after.0 == wheel_top_before.0 && wheel_top_after.1.y < wheel_top_before.1.y - 1.0),
        "stationary raw wheel must scroll the real Font list after keyboard navigation: before={wheel_top_before:?}, after={wheel_top_after:?}"
    );
    assert!(s.app.ui.dialog.is_none(), "wheel scroll cannot invoke a menu command");
    let wheel_state = s.app.session.active().unwrap();
    assert_eq!(wheel_state.doc, wheel_doc);
    assert_eq!(wheel_state.selection, wheel_selection);
    assert_eq!((wheel_state.history.undo.len(), wheel_state.history.redo.len()), wheel_history);
    for _ in 0..20 {
        s.at_size(vec![key(Key::ArrowDown, true, Modifiers::NONE)], size);
        s.at_size(vec![key(Key::ArrowDown, false, Modifiers::NONE)], size);
    }
    let focus_before_settle = s.ctx.memory(|m| m.focused()).unwrap();
    // The normal scroll animation can take 0.3s. Keep the actual narrow viewport while it settles.
    for _ in 0..10 {
        s.at_size(vec![], size);
    }
    let cues = s.at_size(vec![], size);
    let focus = s.ctx.memory(|m| m.focused()).unwrap();
    assert_eq!(focus, focus_before_settle, "settling scrolling must not change the chosen Font row");
    assert_ne!(focus, first, "normal Down navigation advances through the long submenu");
    // An installed-font scan can finish while the menu is open; compare with its current rows.
    let entries = vectorcraft_ui_egui::menus::menu_entries(&s.app);
    let font_labels: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.path.first().is_some_and(|p| p == "Type") && e.path.iter().any(|p| p == "Font") && e.command.as_deref() == Some("text.setStyle")
        })
        .map(|e| e.label.clone())
        .collect();
    let response = s.ctx.read_response(focus).unwrap();
    let viewport = Rect::from_min_size(Pos2::ZERO, size);
    assert!(
        viewport.contains(response.rect.center()),
        "the focused row must scroll into the actual narrow viewport: focus={focus:?}, rect={:?}, viewport={viewport:?}, visible_fonts={:?}",
        response.rect,
        s.shown.iter().filter(|(text, _)| font_labels.contains(&clean_label(text).to_owned())).take(8).collect::<Vec<_>>()
    );
    assert!(
        cues.iter().any(|c| c.popup
            && font_labels.contains(&c.label)
            && s.shown.iter().any(|(text, point)| clean_label(text) == c.label && response.rect.contains(*point))),
        "the scrolled focused Font row retains a visible painted access cue"
    );
}

/// The reported Select menu letters use the actual name/list dialog routes,
/// with no document edit before a normal dialog confirmation.
#[test]
fn menu_access_select_save_and_edit_use_reported_letters() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let before = s.app.session.active().unwrap();
    let (doc, selection) = (before.doc.clone(), before.selection.clone());
    let history = (before.history.undo.len(), before.history.redo.len());
    s.root("Select");
    s.letter(Key::S, "s", Modifiers::NONE);
    s.settle();
    let dialog = s.app.ui.dialog.as_ref().expect("Alt+S then S must open Save Selection, not focus Same");
    assert_eq!(dialog.kind, "command");
    assert_eq!(dialog.str("__command"), "select.save");
    assert_eq!(dialog.str("__label"), "Save Selection");
    assert_eq!(dialog.str("name"), "Selection 1");
    assert!(s.shown.iter().any(|(text, _)| text == "Save Selection"), "the real naming dialog must be drawn");
    let after = s.app.session.active().unwrap();
    assert_eq!(after.doc, doc, "opening Save Selection does not edit art or saved selections");
    assert_eq!(after.selection, selection);
    assert_eq!((after.history.undo.len(), after.history.redo.len()), history);
    vectorcraft_ui_egui::dialogs::cancel(&mut s.app);
    s.settle();
    s.end_access();
    assert!(s.app.ui.dialog.is_none());
    let after_cancel = s.app.session.active().unwrap();
    assert_eq!(after_cancel.doc, doc);
    assert_eq!(after_cancel.selection, selection);
    assert_eq!((after_cancel.history.undo.len(), after_cancel.history.redo.len()), history);

    // Edit Selection is enabled only after a real saved selection exists.
    // This command is setup, never a substitute for the observed E invocation.
    s.app.run("select.save", json!({"name": "Access Keys Selection"})).unwrap();
    s.settle();
    let before = s.app.session.active().unwrap();
    let (doc, selection) = (before.doc.clone(), before.selection.clone());
    let history = (before.history.undo.len(), before.history.redo.len());
    s.root("Select");
    s.letter(Key::E, "e", Modifiers::NONE);
    s.settle();
    let dialog = s.app.ui.dialog.as_ref().expect("Alt+S then E must open Edit Selection");
    assert_eq!(dialog.kind, "editSelection");
    assert_eq!(dialog.fields["count"].as_u64(), Some(1));
    assert_eq!(dialog.str("orig0"), "Access Keys Selection");
    assert!(s.shown.iter().any(|(text, _)| text == "Edit Selection"), "the real saved-selection list must be drawn");
    let after = s.app.session.active().unwrap();
    assert_eq!(after.doc, doc, "opening Edit Selection does not rename or delete a saved selection");
    assert_eq!(after.selection, selection);
    assert_eq!((after.history.undo.len(), after.history.redo.len()), history);
    vectorcraft_ui_egui::dialogs::cancel(&mut s.app);
    s.settle();
    s.end_access();
    assert!(s.app.ui.dialog.is_none());
    let after_cancel = s.app.session.active().unwrap();
    assert_eq!(after_cancel.doc, doc);
    assert_eq!(after_cancel.selection, selection);
    assert_eq!((after_cancel.history.undo.len(), after_cancel.history.redo.len()), history);
}

// Review regressions use real App input and painted menu targets, including the native parent
// focus left by Right. The original nine cases above remain unchanged.
#[test]
fn menu_access_review_bare_alt_outside_panel_click_releases_tool_shortcuts() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let state = s.app.session.active().unwrap();
    let (doc, selection, history) = (state.doc.clone(), state.selection.clone(), (state.history.undo.len(), state.history.redo.len()));
    s.bare_alt();
    assert!(s.cues().iter().any(|c| !c.popup && c.label == "File"), "bare Alt must first expose the File root cue");
    assert!(!egui::Popup::is_any_open(&s.ctx), "this is closed access mode, not an open popup");
    // The actual Layers tab/header is panel chrome. Unlike the earlier canvas click, this
    // cannot accidentally satisfy cancellation by starting a busy drawing/selection gesture.
    let header = s.point("Layers", false);
    assert!(header.y > 40.0, "the actual Layers header is outside the main menu bar");
    s.click(header);
    assert!(!s.app.session.tool_busy(), "the non-canvas header click must leave the tool idle");
    assert!(!s.cues().iter().any(|c| !c.popup && c.label == "File"), "an outside panel click must cancel even closed access mode");
    assert!(!egui::Popup::is_any_open(&s.ctx));
    assert!(s.app.ui.dialog.is_none());
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, doc);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    s.letter(Key::P, "p", Modifiers::NONE);
    assert_eq!(s.app.session.tool_id(), "pen", "ordinary P must reach the tool shortcut after panel dismissal");
    assert!(!egui::Popup::is_any_open(&s.ctx), "the same P must not navigate a menu");
    assert!(s.app.ui.dialog.is_none());
}

#[test]
fn menu_access_review_font_mnemonic_after_right_uses_open_child_level() {
    let _global = GlobalState::new();
    let mut s = Scene::new();
    let id = s.app.run("text.create", json!({"x": 250, "y": 200, "text": "Menu", "font": "Inter", "size": 12})).unwrap()["id"].as_u64().unwrap();
    s.app.run("select.set", json!({"ids": [id]})).unwrap();
    s.settle();
    let font_rows = |scene: &Scene| {
        vectorcraft_ui_egui::menus::menu_entries(&scene.app)
            .into_iter()
            .filter(|e| {
                e.path.first().is_some_and(|p| p == "Type") && e.path.iter().any(|p| p == "Font") && e.command.as_deref() == Some("text.setStyle")
            })
            .map(|e| (e.label, e.params["font"].as_str().expect("Font family rows supply the canonical family").to_owned()))
            .collect::<Vec<_>>()
    };
    let family = |scene: &Scene| {
        let node = scene.app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        let vectorcraft_doc::NodeKind::Text(text) = &node.kind else { panic!("the selected original object remains text") };
        text.runs.first().unwrap().style.font_family.clone()
    };
    let initial_rows = font_rows(&s);
    assert!(initial_rows.len() > 1, "the inherited font fixture has distinct real families");
    // Set a real family from the end of the same public menu, so its first row can make a
    // meaningful font change without guessing a host-installed family or a private font API.
    let initial = initial_rows.last().unwrap().1.clone();
    s.app.run("text.setStyle", json!({"font": initial})).unwrap();
    s.settle();
    let before = family(&s);
    let selection = s.app.session.active().unwrap().selection.clone();
    let history = s.app.session.active().unwrap().history.undo.len();
    let size = vec2(640.0, 420.0);
    for _ in 0..4 {
        s.at_size(vec![], size);
    }
    let pointer = s.ctx.pointer_hover_pos();
    // Match the existing narrow-menu witness exactly; the wide Scene convenience methods
    // would reset the viewport and could change egui's Right/focus transition.
    s.at_size(vec![Event::ModifiersChanged(Modifiers::ALT), key(Key::T, true, Modifiers::ALT), Event::Text("t".into())], size);
    s.at_size(vec![key(Key::T, false, Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)], size);
    let type_cues = s.at_size(vec![], size);
    let font_cue = type_cues.iter().find(|c| c.popup && c.label == "Font").expect("the actual narrow Type menu exposes Font");
    let font_text = font_cue.character.to_lowercase().collect::<String>();
    let font_key = Key::from_name(&font_cue.character.to_uppercase().collect::<String>()).unwrap();
    s.at_size(vec![key(font_key, true, Modifiers::NONE), Event::Text(font_text)], size);
    s.at_size(vec![key(font_key, false, Modifiers::NONE)], size);
    s.at_size(vec![key(Key::ArrowRight, true, Modifiers::NONE)], size);
    s.at_size(vec![key(Key::ArrowRight, false, Modifiers::NONE)], size);
    // No Down and no pointer movement: this is the actual parent-focus state the former
    // long-menu test skipped before switching to pointer/wheel interaction.
    let cues = s.at_size(vec![], size);
    // Exercise the actual Right transition without forcing global focus onto its parent.
    // The unchanged implementation already focuses a family in this native App route.
    assert_eq!(s.ctx.pointer_hover_pos(), pointer, "opening Font and choosing its letter use keyboard input only");
    let rows = font_rows(&s);
    let (label, expected) = rows.first().expect("the open Font list has an actual first family").clone();
    assert_ne!(before, expected, "the chosen family must actually change the text's font");
    let target = s.point(&label, true);
    assert!(Rect::from_min_size(Pos2::ZERO, size).contains(target), "the first real family is visible");
    let cue = cues.iter().find(|c| c.popup && c.label == label).expect("the actual visible family advertises its mnemonic");
    let text = cue.character.to_lowercase().collect::<String>();
    let key_id = Key::from_name(&cue.character.to_uppercase().collect::<String>()).expect("the English family cue has a native key");
    s.at_size(vec![Event::ModifiersChanged(Modifiers::NONE), key(key_id, true, Modifiers::NONE), Event::Text(text)], size);
    s.at_size(vec![key(key_id, false, Modifiers::NONE)], size);
    for _ in 0..3 {
        s.at_size(vec![], size);
    }
    assert!(s.app.ui.dialog.is_none(), "a family letter cannot invoke a command from the parent Type level");
    if family(&s) != expected {
        // A shared cue selects the first matching row and requires Enter. The first family
        // is also the first possible match; never substitute Down or pointer selection.
        assert_eq!(
            focused_label(&s, &[label.as_str()]),
            Some(label.clone()),
            "the delivered family mnemonic must focus inside Font, not Type: {}",
            s.menu_diagnostic()
        );
        s.at_size(vec![key(Key::Enter, true, Modifiers::NONE)], size);
        s.at_size(vec![key(Key::Enter, false, Modifiers::NONE)], size);
        for _ in 0..3 {
            s.at_size(vec![], size);
        }
    }
    assert_eq!(family(&s), expected, "the advertised child mnemonic must apply its real text.setStyle target");
    assert_eq!(s.app.session.active().unwrap().selection, selection);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1, "one family choice makes one font edit");
    assert!(!egui::Popup::is_any_open(&s.ctx));
    assert!(s.app.ui.dialog.is_none());
}

#[test]
fn menu_access_review_fresh_alt_reselects_root_from_open_submenu() {
    let _global = GlobalState::new();
    for pointer_movement in [true, false] {
        let mut s = Scene::new();
        if pointer_movement {
            // Establish a real previous position before opening the menus, so the later movement
            // is nonzero rather than the pointer's first appearance.
            s.frame(vec![Event::PointerMoved(s.point("Layers", false))]);
            s.settle();
        }
        let state = s.app.session.active().unwrap();
        let (doc, selection, history) = (state.doc.clone(), state.selection.clone(), (state.history.undo.len(), state.history.redo.len()));
        s.root("Object");
        s.letter(Key::P, "p", Modifiers::NONE);
        s.arrow(Key::ArrowRight);
        s.settle();
        assert!(s.cues().iter().any(|c| c.popup && c.label == "Offset Path\u{2026}"), "the actual Object > Path submenu must be open first");
        assert!(s.app.ui.dialog.is_none());
        // A fresh physical Alt chord names a top-level root even while a descendant popup is
        // open; unmodified letters below still address the selected submenu normally.
        if pointer_movement {
            let title = s.point("Object", false);
            assert_ne!(s.ctx.pointer_hover_pos(), Some(title), "the same-batch pointer must actually move");
            s.frame(vec![
                Event::ModifiersChanged(Modifiers::ALT),
                Event::PointerMoved(title),
                key(Key::O, true, Modifiers::ALT),
                Event::Text("o".into()),
            ]);
            s.frame(vec![key(Key::O, false, Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)]);
            s.settle();
        } else {
            s.root("Object");
        }
        assert!(s.app.ui.dialog.is_none(), "fresh Alt+O must select Object, not invoke Offset Path");
        s.assert_root_contents("Object");
        assert!(
            !s.shown.iter().any(|(text, point)| clean_label(text) == "Offset Path\u{2026}"
                && s.ctx.layer_id_at(*point).is_some_and(|layer| layer.order == egui::Order::Foreground)),
            "reselecting Object closes its old Path descendant"
        );
        s.letter(Key::P, "p", Modifiers::NONE);
        s.arrow(Key::ArrowRight);
        s.letter(Key::O, "o", Modifiers::NONE);
        s.settle();
        s.assert_offset_dialog();
        let state = s.app.session.active().unwrap();
        assert_eq!(state.doc, doc);
        assert_eq!(state.selection, selection);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
    }
}
