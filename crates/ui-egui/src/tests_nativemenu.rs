//! The macOS menu bar's model ([`crate::native_menu`]), tested on every platform with a fake
//! backend: the Mac layout of the in-window menus, the keys it hands back to egui, the clicks it
//! runs and when it reads the menus again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use egui::{Event, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES;

use crate::i18n::Lang;
use crate::native_menu::{self, Backend, MenuBar, MenuRole, NativeMenu, Node, Standard};
use crate::{VectorcraftApp, menus};

fn app(doc: bool) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    if doc {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 37, "height": 40})).unwrap();
    }
    app
}

fn bar(app: &VectorcraftApp) -> MenuBar {
    native_menu::mac_layout(app, &native_menu::from_tree(app, &menus::menu_tree()), Lang::EN).bar
}

/// A level's nodes as short names: an item's command (`---` a separator, `[label]` a submenu,
/// `<Standard>` an AppKit item).
fn ids(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Item(it) => it.command.unwrap_or("").to_string(),
            Node::Separator => "---".into(),
            Node::Header(h) => format!("#{h}"),
            Node::Submenu { label, .. } => format!("[{label}]"),
            Node::Standard(s) => format!("<{s:?}>"),
        })
        .collect()
}

fn menu<'a>(bar: &'a MenuBar, title: &str) -> &'a [Node] {
    &bar.menus.iter().find(|m| m.title == title).unwrap_or_else(|| panic!("no {title} menu")).children
}

fn submenu<'a>(nodes: &'a [Node], name: &str) -> &'a [Node] {
    nodes
        .iter()
        .find_map(|n| match n {
            Node::Submenu { label, children } if label == name => Some(children.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {name} submenu in {:?}", ids(nodes)))
}

fn items(nodes: &[Node]) -> Vec<&native_menu::Item> {
    nodes.iter().filter_map(|n| if let Node::Item(it) = n { Some(it) } else { None }).collect()
}

fn find<'a>(bar: &'a MenuBar, command: &str) -> &'a native_menu::Item {
    bar.items().into_iter().find(|it| it.command == Some(command)).unwrap_or_else(|| panic!("no {command} item"))
}

#[test]
fn vectorcraft_gets_a_mac_app_menu() {
    let bar = bar(&app(true));
    let titles: Vec<&str> = bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["VectorCraft", "File", "Edit", "Object", "Type", "Select", "Effect", "View", "Window", "Help"]);
    let app_menu = &bar.menus[0];
    assert_eq!(app_menu.role, MenuRole::App);
    assert_eq!(
        ids(&app_menu.children),
        [
            "help.about",
            "---",
            "[Settings]",
            "[Language]",
            "[Appearance]",
            "---",
            "<Services>",
            "---",
            native_menu::HIDE,
            native_menu::HIDE_OTHERS,
            "<ShowAll>",
            "---",
            "app.quit"
        ]
    );
    let shown = |command: &str| {
        let it = find(&bar, command);
        (it.label.as_str(), it.shortcut)
    };
    assert_eq!(shown("help.about"), ("About VectorCraft", None));
    assert_eq!(shown("app.quit"), ("Quit VectorCraft", Some("Cmd+Q")), "Quit is the app's own, so unsaved documents are asked about");
    // ⌘H stays View › Hide Edges (Illustrator's); Hide takes ⌃⌘H.
    assert_eq!(shown(native_menu::HIDE), ("Hide VectorCraft", Some("Ctrl+Cmd+H")));
    assert_eq!(shown(native_menu::HIDE_OTHERS), ("Hide Others", Some("Cmd+Alt+H")));
    assert_eq!(find(&bar, "view.edges").shortcut, Some("Cmd+H"));
    assert_eq!(bar.menu(MenuRole::Help).map(|m| m.title.as_str()), Some("Help"));
}

/// Settings ▸ lists the Preferences dialog's own pages, General… with ⌘K, and each opens the
/// dialog on its page.
#[test]
fn settings_lists_every_preferences_page() {
    let mut app = app(true);
    let bar = bar(&app);
    let pages = items(submenu(&bar.menus[0].children, "Settings"));
    assert_eq!(pages.len(), PREF_CATEGORIES.len());
    assert_eq!((pages[0].label.as_str(), pages[0].shortcut), ("General…", Some("Cmd+K")));
    assert!(pages[1..].iter().all(|it| it.shortcut.is_none()));
    for (it, page) in pages.iter().zip(PREF_CATEGORIES) {
        assert_eq!((it.command, &it.params, it.label.clone()), (Some("edit.preferences"), &json!({"category": page}), format!("{page}…")));
    }
    let units = pages.iter().find(|it| it.source == "Units").unwrap();
    let (id, p) = menus::click_target(units.source, units.command.unwrap(), &units.params);
    assert_eq!(id, "edit.preferences", "a page opens the dialog, not a parameter form");
    menus::invoke(&mut app, &id, p);
    let dialog = app.ui.dialog.as_ref().expect("Preferences open");
    assert_eq!((dialog.kind.as_str(), dialog.fields.get("__category")), ("preferences", Some(&json!("Units"))));
}

/// Language ▸: Automatic, then each language in its own name (in any UI language), the
/// preference checked; Appearance ▸: the UI brightness choices, the current one checked.
#[test]
fn language_and_appearance_are_in_the_app_menu() {
    let mut app = app(false);
    app.session.prefs.interface_language = "ja".into();
    let bar = bar(&app);
    let app_menu = &bar.menus[0].children;
    assert_eq!(ids(submenu(app_menu, "Language"))[..2], ["app.language", "---"], "Automatic, then the languages");
    let langs: Vec<(String, Option<bool>)> = items(submenu(app_menu, "Language")).iter().map(|it| (it.label.clone(), it.checked)).collect();
    let mut want = vec![("Automatic".to_string(), Some(false))];
    want.extend(Lang::all().map(|l| (l.name().to_string(), Some(l.code() == "ja"))));
    assert_eq!(langs, want);
    // In every UI language the languages keep their own names.
    for ui in Lang::all() {
        for l in Lang::all() {
            assert_eq!(crate::i18n::tr_id(ui, "app.language", l.name()), l.name(), "{} in {}", l.name(), ui.code());
        }
    }
    let appearance: Vec<(&'static str, Option<bool>)> = items(submenu(app_menu, "Appearance")).iter().map(|it| (it.source, it.checked)).collect();
    assert_eq!(appearance.len(), crate::theme::Brightness::ALL.len());
    assert_eq!(appearance.iter().filter(|(_, c)| *c == Some(true)).count(), 1, "{appearance:?}");
    assert!(appearance.iter().all(|(_, c)| c.is_some()));
    assert_eq!(crate::i18n::tr_ctx(Lang::from_code("ja").unwrap(), "theme", "Appearance"), "外観", "the theme, not the Appearance panel");
}

/// Everything the in-window bar lists is in the Mac menu bar, once: what moved to the app menu
/// (About, Settings, Quit, Language, UI Brightness) leaves its other places, and Join Our Discord
/// stays under Help.
#[test]
fn every_in_window_item_is_in_the_mac_menu_bar_once() {
    let app = app(true);
    let count = |bar: &MenuBar| {
        let mut n: HashMap<(String, String), usize> = HashMap::new();
        for it in bar.items() {
            if let Some(c) = it.command {
                // Settings ▸ opens Preferences on each page: one item per page instead of one.
                let params = if c == "edit.preferences" { String::new() } else { it.params.to_string() };
                *n.entry((c.to_string(), params)).or_default() += 1;
            }
        }
        n
    };
    let in_window = native_menu::from_tree(&app, &menus::menu_tree());
    let mac = bar(&app);
    let (a, b) = (count(&in_window), count(&mac));
    // The in-window application menu's commands, its submenus' included.
    let moved: Vec<&str> = MenuBar { menus: vec![in_window.menus[0].clone()] }.items().iter().filter_map(|it| it.command).collect();
    for (key, n) in &a {
        let m = b.get(key).copied().unwrap_or(0);
        let want = match key.0.as_str() {
            "edit.preferences" => PREF_CATEGORIES.len(),
            c if moved.contains(&c) => 1,
            _ => *n,
        };
        assert_eq!(m, want, "{key:?} is listed {m} times on the Mac, {n} in the window");
    }
    let system = [native_menu::HIDE, native_menu::HIDE_OTHERS, native_menu::MINIMIZE];
    // Language and Appearance have no in-window menu (synthesized in the App menu, as in PhotoCraft).
    let synthesized = ["app.language", "window.brightness"];
    for key in b.keys() {
        assert!(a.contains_key(key) || system.contains(&key.0.as_str()) || synthesized.contains(&key.0.as_str()), "{key:?} isn't an in-window item");
    }
    let help = ids(menu(&mac, "Help"));
    assert!(help.contains(&"help.discord".to_string()) && !help.contains(&"help.about".to_string()), "{help:?}");
    let edit = ids(menu(&mac, "Edit"));
    assert!(!edit.contains(&"edit.preferences".to_string()) && edit.contains(&"edit.keyboardShortcuts".to_string()), "{edit:?}");
    assert!(ids(menu(&mac, "Window")).contains(&"[Workspace]".to_string()));
}

#[test]
fn the_window_menu_has_the_system_items() {
    let bar = bar(&app(true));
    let window = ids(&bar.menu(MenuRole::Window).unwrap().children);
    assert_eq!(window[..3], [native_menu::MINIMIZE.to_string(), "<Zoom>".into(), "---".into()]);
    assert_eq!(window[window.len() - 2..], ["---".to_string(), "<BringAllToFront>".into()]);
    assert_eq!(window[3], "window.newWindow", "then VectorCraft's own items");
    assert_eq!(find(&bar, native_menu::MINIMIZE).shortcut, Some("Ctrl+Cmd+M"));
    assert!(!window.iter().any(|i| i == "window.brightness"), "the theme is only in the app menu");
    assert_eq!(Standard::Zoom.label(), "Zoom");
}

/// A system key a command already has stays the command's: the system item goes without. The
/// user's shortcuts show in the menus.
#[test]
fn a_command_keeps_a_system_key_it_is_bound_to() {
    let mut app = app(true);
    app.ui.shortcut_overrides.insert("view.grid".into(), "Ctrl+Cmd+H".into());
    crate::shortcut_editor::sync(&app.ui);
    let layout = native_menu::mac_layout(&app, &native_menu::from_tree(&app, &menus::menu_tree()), Lang::EN);
    app.ui.shortcut_overrides.clear();
    crate::shortcut_editor::sync(&app.ui);
    assert_eq!(find(&layout.bar, native_menu::HIDE).shortcut, None);
    assert_eq!(find(&layout.bar, "view.grid").shortcut, Some("Ctrl+Cmd+H"), "the user's override shows in the menu");
    assert_eq!(layout.clashes.iter().map(|c| (c.shortcut, c.command)).collect::<Vec<_>>(), [("Ctrl+Cmd+H", "view.grid")]);
}

/// A native menu can't turn a plain item into a check item in place: opening a document or
/// selecting must not change the menus' structure, or every change rebuilds them.
#[test]
fn documents_and_selections_change_state_not_structure() {
    let empty = native_menu::structure_key(&bar(&app(false)));
    let mut a = app(true);
    assert_eq!(native_menu::structure_key(&bar(&a)), empty, "a document with a selected rectangle");
    a.run("select.none", json!({})).unwrap();
    a.run("text.create", json!({"x": 50, "y": 50, "text": "Hi"})).unwrap();
    assert!(!a.session.active().unwrap().selection.objects.is_empty());
    assert_eq!(native_menu::structure_key(&bar(&a)), empty, "type selected");
    // A recent file is a new item: the menus are rebuilt.
    crate::io::note_recent(&mut a, "/art/a.svg");
    assert_ne!(native_menu::structure_key(&bar(&a)), empty);
}

#[test]
fn key_equivalents_are_command_and_function_keys_only() {
    let ok = |s| crate::shortcuts::parse(s).is_some_and(|k| native_menu::native_ok(&k));
    assert!(ok("Cmd+B") && ok("F7") && ok("Ctrl+Cmd+M") && ok("Shift+F7"));
    assert!(!ok("B") && !ok("Alt+F") && !ok("Shift+X"));
}

/// What a key equivalent becomes: the press and release egui's window integration sends, or for
/// ⌘X, ⌘C and ⌘V its clipboard events (a paste's release comes from the window).
#[test]
fn key_equivalents_become_the_input_their_keys_make() {
    let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
    let key = |key, pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::COMMAND };
    assert_eq!(native_menu::key_events(cmd(Key::A), || None), [key(Key::A, true), key(Key::A, false)]);
    assert_eq!(native_menu::key_events(cmd(Key::C), || None), [Event::Copy, key(Key::C, false)]);
    assert_eq!(native_menu::key_events(cmd(Key::X), || None), [Event::Cut, key(Key::X, false)]);
    assert_eq!(native_menu::key_events(cmd(Key::V), || Some("a\r\nb".into())), [Event::Paste("a\nb".into())]);
    assert_eq!(native_menu::key_events(cmd(Key::V), || None), [], "a picture: the release tells the paste");
}

/// The fake backend: the menus the app synced, and events to report.
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<(Vec<MenuBar>, Vec<native_menu::Event>)>>);

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.0.borrow_mut().0.push(bar.clone());
    }
    fn drain(&mut self) -> Vec<native_menu::Event> {
        std::mem::take(&mut self.0.borrow_mut().1)
    }
}

impl Fake {
    fn synced(&self) -> usize {
        self.0.borrow().0.len()
    }
    fn last(&self) -> MenuBar {
        self.0.borrow().0.last().cloned().unwrap()
    }
    fn report(&self, e: native_menu::Event) {
        self.0.borrow_mut().1.push(e);
    }
}

/// The app with the fake native menu (or none), driven a whole frame at a time like the desktop
/// app.
struct Mac {
    app: VectorcraftApp,
    fake: Fake,
    ctx: egui::Context,
    time: f64,
}

impl Mac {
    fn new(native: bool) -> Self {
        let mut app = app(true);
        let fake = Fake::default();
        if native {
            app.services.native_menu = Some(NativeMenu::new(Box::new(fake.clone())));
        }
        let mut m = Mac { app, fake, ctx: egui::Context::default(), time: 0.0 };
        for _ in 0..3 {
            m.frame(vec![]);
        }
        m
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.1;
        let mut raw = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0))),
            ..Default::default()
        };
        self.app.raw_input_hook(&mut raw);
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out
    }

    /// AppKit chose an item by its key equivalent `sc` (⌘ held).
    fn key_equivalent(&mut self, sc: &str) -> egui::FullOutput {
        let mut k = crate::shortcuts::parse(sc).unwrap();
        k.modifiers.mac_cmd = k.modifiers.command;
        self.fake.report(native_menu::Event::Key(k));
        self.frame(vec![])
    }

    fn selected(&self) -> usize {
        self.app.session.active().unwrap().selection.objects.len()
    }
}

/// A key equivalent runs through the app's own key handling, once: ⌘A selects every object, ⌘Z
/// undoes one step; and frames where nothing changed don't read the menus again.
#[test]
fn key_equivalents_run_through_the_normal_shortcut_path_once() {
    let mut m = Mac::new(true);
    m.app.run("shape.rectangle", json!({"x": 200, "y": 150, "width": 20, "height": 20})).unwrap();
    m.app.run("select.none", json!({})).unwrap();
    m.frame(vec![]);
    m.key_equivalent("Cmd+A");
    assert_eq!(m.selected(), 2, "⌘A selected every object");
    let undo = |m: &Mac| m.app.session.active().unwrap().history.undo.len();
    let before = undo(&m);
    m.key_equivalent("Cmd+Z");
    assert_eq!(undo(&m), before - 1, "⌘Z undid one step");
    assert!(m.fake.synced() >= 1, "the menus were built");
    // Idle frames don't read the menus again. The shortcut and plug-in generations are
    // process-wide, and tests running alongside with other shortcuts bump them (rightly reading
    // the menus again), so judge two idle frames in which they held still.
    let globals = || (crate::shortcut_editor::GENERATION.load(std::sync::atomic::Ordering::Relaxed), crate::menus::plugin_revision());
    let quiet = (0..50).any(|_| {
        let (before, synced) = (globals(), m.fake.synced());
        m.frame(vec![]);
        m.frame(vec![]);
        globals() == before && m.fake.synced() == synced
    });
    assert!(quiet, "nothing changed: the menus weren't read again");
    assert_eq!(crate::control::inspect(&m.app, &m.ctx)["nativeMenuBar"], json!(true));
}

/// A click runs its command like an in-window click, and the menus follow: View › Show Rulers
/// reads Hide Rulers afterwards; the app menu's Appearance items run with their params.
#[test]
fn clicks_run_their_command_and_the_menus_follow() {
    let mut m = Mac::new(true);
    let rulers = find(&m.fake.last(), "view.rulers").clone();
    let before = m.app.ui.view.rulers;
    m.fake.report(native_menu::Event::Click(rulers.clone()));
    m.frame(vec![]);
    assert_eq!(m.app.ui.view.rulers, !before);
    assert_ne!(find(&m.fake.last(), "view.rulers").label, rulers.label, "the label flipped");
    let other = m.fake.last().items().into_iter().find(|it| it.command == Some("window.brightness") && it.checked == Some(false)).cloned().unwrap();
    m.fake.report(native_menu::Event::Click(other.clone()));
    m.frame(vec![]);
    assert_eq!(Some(m.app.ui.brightness.id()), other.params.get("brightness").and_then(Value::as_str));
    let now = m.fake.last().items().into_iter().find(|it| it.params == other.params).cloned().unwrap();
    assert_eq!(now.checked, Some(true), "the checkmark moved");
}

/// With a text field focused, the menu's ⌘A and ⌘C act on the field, as the keys do without the
/// native menu.
#[test]
fn a_focused_field_keeps_select_all_and_copy() {
    let mut m = Mac::new(true);
    let (id, rect) = m
        .ctx
        .viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).map(|w| (w.id, w.rect)).collect::<Vec<_>>())
        .into_iter()
        .find(|&(id, _)| m.ctx.data(|d| d.get_temp::<String>(id)).is_some_and(|t| t == "37 pt") && egui::TextEdit::load_state(&m.ctx, id).is_some())
        .expect("the W field");
    let click = |pressed| Event::PointerButton { pos: rect.center(), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    m.frame(vec![Event::PointerMoved(rect.center()), click(true)]);
    m.frame(vec![click(false)]);
    m.frame(vec![Event::Key { key: Key::End, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    assert!(m.ctx.text_edit_focused());
    m.key_equivalent("Cmd+A");
    let r = egui::TextEdit::load_state(&m.ctx, id).and_then(|s| s.cursor.char_range()).unwrap();
    let (a, b): (usize, usize) = (r.primary.index.into(), r.secondary.index.into());
    assert_eq!((a.min(b), a.max(b)), (0, 5), "⌘A selected the field's text");
    assert_eq!(m.selected(), 1, "not the art");
    let out = m.key_equivalent("Cmd+C");
    let copied: Vec<_> =
        out.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.as_str()) } else { None }).collect();
    assert_eq!(copied, ["37 pt"]);
    assert!(m.app.session.clipboard.is_empty(), "no art copied");
}

/// With the macOS menu bar the app bar draws no menu titles: each shows once fewer.
#[test]
fn the_app_bar_hides_its_menus_with_the_mac_menu_bar() {
    let texts = |native: bool| {
        let mut m = Mac::new(native);
        let out = m.frame(vec![]);
        out.shapes
            .iter()
            .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t.galley.text().to_string()) } else { None })
            .collect::<Vec<_>>()
    };
    let (in_window, native) = (texts(false), texts(true));
    for (title, _) in menus::menu_tree().iter().skip(1) {
        let n = |t: &[String]| t.iter().filter(|s| s == title).count();
        assert_eq!(n(&native) + 1, n(&in_window), "{title}");
    }
}

/// While a file dialog's sheet is on the window the menu bar does nothing, as under the modal
/// dialog before: every item is disabled but Hide, Hide Others and Minimize, a click that raced it
/// isn't run (Minimize still is), and a key equivalent doesn't reach the app. Once the dialog
/// answers, the menus are as before and work again.
#[test]
fn the_menu_bar_is_inert_while_a_file_dialog_is_open() {
    let mut m = Mac::new(true);
    m.app.services.dialogs_are_sheets = true;
    let (shown, _) = crate::tests_picks::off_the_ui_thread(&mut m.app.services);
    m.app.run("shape.rectangle", json!({"x": 200, "y": 150, "width": 20, "height": 20})).unwrap();
    m.app.run("select.none", json!({})).unwrap();
    m.frame(vec![]);
    let rows = |bar: &MenuBar| bar.items().iter().map(|it| (it.command, it.params.clone(), it.enabled)).collect::<Vec<_>>();
    let before = m.fake.last();
    let system = [native_menu::HIDE, native_menu::HIDE_OTHERS, native_menu::MINIMIZE];
    assert!(system.iter().all(|c| find(&before, c).enabled));
    assert!(m.app.run("file.open", json!({})).is_err());
    assert!(m.app.file_dialog_open());
    m.frame(vec![]);
    let inert = m.fake.last();
    for it in inert.items() {
        let kept = it.command.is_some_and(|c| system.contains(&c));
        assert_eq!(it.enabled, kept, "{:?} {} while the dialog is open", it.command, it.label);
    }
    // A click that raced the menu going inert isn't run; Minimize is.
    let rulers = (m.app.ui.view.rulers, find(&before, "view.rulers").clone());
    m.fake.report(native_menu::Event::Click(rulers.1.clone()));
    m.frame(vec![]);
    assert_eq!(m.app.ui.view.rulers, rulers.0);
    m.fake.report(native_menu::Event::Click(find(&inert, native_menu::MINIMIZE).clone()));
    assert!(m.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::Minimized(true)));
    // A key equivalent that raced it doesn't reach egui.
    let mut k = crate::shortcuts::parse("Cmd+A").unwrap();
    k.modifiers.mac_cmd = k.modifiers.command;
    m.fake.report(native_menu::Event::Key(k));
    let mut raw = egui::RawInput::default();
    m.app.raw_input_hook(&mut raw);
    assert!(!raw.events.iter().any(|e| matches!(e, Event::Key { .. })), "{:?}", raw.events);
    m.key_equivalent("Cmd+A");
    assert_eq!(m.selected(), 0, "⌘A selected nothing");
    // Cancelled: the menus are as before, and work again.
    shown.answer(&[]);
    m.frame(vec![]);
    m.frame(vec![]);
    assert!(!m.app.file_dialog_open());
    // Every row from before is back as it was (tests running alongside may add plug-in rows).
    let after = rows(&m.fake.last());
    assert!(rows(&before).iter().all(|r| after.contains(r)), "the menus are as before");
    m.fake.report(native_menu::Event::Click(rulers.1));
    m.frame(vec![]);
    assert_eq!(m.app.ui.view.rulers, !rulers.0);
    m.key_equivalent("Cmd+A");
    assert_eq!(m.selected(), 2, "⌘A selected every object");
}
