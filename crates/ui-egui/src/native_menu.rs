//! The macOS menu bar: the menus of the in-window bar ([`crate::menus::menu_tree`], each item as
//! [`crate::menus::entry`] shows it), laid out the way a Mac app's are and handed to a platform
//! [`Backend`] (the desktop app's, through muda).
//!
//! Pure and platform-free, so it is tested everywhere and builds for the web; only the backend
//! touches AppKit. What the layout does ([`mac_layout`], Illustrator on the Mac as the reference):
//! - an **app menu** named VectorCraft: About; Settings, a submenu with one item per Preferences
//!   page (General… ⌘K first); Language; Appearance (the UI brightness); Services; Hide VectorCraft
//!   ⌃⌘H (⌘H stays View › Hide Edges), Hide Others ⌥⌘H, Show All; and Quit ⌘Q, still the app's own
//!   `app.quit`, so documents with unsaved changes are asked about. Language and Appearance have
//!   no in-window menu (as in PhotoCraft) and are synthesized here; Quit is File › Exit in the
//!   window and reads as Quit in the App menu. These leave the other menus (Edit › Preferences…,
//!   Help › About), and Join Our Discord stays under Help only;
//! - **Window** gets Minimize ⌃⌘M, Zoom and Bring All to Front, and **Help** the system's search
//!   field;
//! - a system key already bound to a command keeps the command's: the system item goes without.
//!
//! **Keys stay with egui.** AppKit runs a menu's key equivalents before the window sees the key,
//! which would skip VectorCraft's key rules (a focused text field takes ⌘A, ⌘C, ⌘V and ⌘Z; dialogs
//! and the Type tool's input method have the keyboard). So the backend reports a key equivalent as
//! [`Event::Key`], and [`key_events`] feeds it to egui as the key press it was (the clipboard
//! events egui's window integration makes of ⌘X, ⌘C and ⌘V): the shortcut runs through
//! [`crate::shortcuts::handle`] exactly as without a native menu, user overrides included. A click
//! is [`Event::Click`] and runs like an in-window menu click.
//!
//! **Updates are cheap.** The rows are read again only when something they show may have changed
//! ([`sync`]); labels, enabled and checked are then updated in place, and the menus are rebuilt
//! only when their structure changes ([`structure_key`]: a recent file added, the language or a
//! shortcut changed).

use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Key, KeyboardShortcut};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::i18n::{Lang, tr, tr_ctx};
use crate::menus::{self, Entry};
use crate::theme::Brightness;

pub const APP_NAME: &str = "VectorCraft";
/// Hide VectorCraft: an item of ours (AppKit's always takes ⌘H, which is View › Hide Edges).
pub const HIDE: &str = "app.hide";
/// Hide Others: an item of ours, so a command bound to ⌥⌘H keeps it.
pub const HIDE_OTHERS: &str = "app.hideOthers";
/// Minimize: an item of ours with ⌃⌘M (AppKit's always takes ⌘M).
pub const MINIMIZE: &str = "window.minimize";

/// The labels the Mac layout adds to the menus (translated like every menu string:
/// [`crate::menus::menu_strings`]).
pub const MAC_LABELS: &[&str] =
    &["Settings", "Appearance", "Services", "Hide VectorCraft", "Hide Others", "Show All", "Minimize", "Zoom", "Bring All to Front"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    /// In the UI language (the app menu is the app's name).
    pub title: String,
    pub role: MenuRole,
    pub children: Vec<Node>,
}

/// What a top-level menu is for: macOS treats the app, Window and Help menus specially.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuRole {
    #[default]
    Normal,
    App,
    Window,
    Help,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Item(Item),
    Separator,
    /// A section header (a disabled label).
    Header(String),
    Submenu {
        label: String,
        children: Vec<Node>,
    },
    /// An item AppKit provides and runs itself.
    Standard(Standard),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The command id, one of [`HIDE`], [`HIDE_OTHERS`], [`MINIMIZE`], or `None` for an item not
    /// implemented yet.
    pub command: Option<&'static str>,
    pub params: Value,
    /// The menu tree's English label ([`crate::menus::click_target`] reads it).
    pub source: &'static str,
    /// In the UI language, as it reads now.
    pub label: String,
    /// As the registry writes it (`Cmd+Shift+Z`): `Cmd` is ⌘ and `Ctrl` the Control key.
    pub shortcut: Option<&'static str>,
    pub enabled: bool,
    /// `Some` for an item that is on or off.
    pub checked: Option<bool>,
}

/// Items AppKit provides and runs: none has a key equivalent, so none can take a command's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Standard {
    Services,
    ShowAll,
    Zoom,
    BringAllToFront,
}

impl Standard {
    /// The English label ([`MAC_LABELS`]).
    pub fn label(self) -> &'static str {
        match self {
            Standard::Services => "Services",
            Standard::ShowAll => "Show All",
            Standard::Zoom => "Zoom",
            Standard::BringAllToFront => "Bring All to Front",
        }
    }
}

impl Item {
    /// An item of the layout's own (Hide, Minimize…): always enabled, `shortcut` unless a command
    /// has it.
    fn system(command: &'static str, source: &'static str, shortcut: Option<&'static str>, lang: Lang) -> Item {
        Item { command: Some(command), params: Value::Null, source, label: tr(lang, source).into(), shortcut, enabled: true, checked: None }
    }
}

impl MenuBar {
    /// Every item, depth first.
    pub fn items(&self) -> Vec<&Item> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Item>) {
            for n in nodes {
                match n {
                    Node::Item(it) => out.push(it),
                    Node::Submenu { children, .. } => walk(children, out),
                    Node::Separator | Node::Header(_) | Node::Standard(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            walk(&m.children, &mut out);
        }
        out
    }

    pub fn menu(&self, role: MenuRole) -> Option<&Menu> {
        self.menus.iter().find(|m| m.role == role)
    }
}

/// The menus as the in-window bar shows them now, as a tree: `tree`'s items through
/// [`crate::menus::entry`] (labels in the UI language, live state, empty slots left out).
pub fn from_tree(app: &VectorcraftApp, tree: &[(&'static str, Vec<menus::Item>)]) -> MenuBar {
    fn level(app: &VectorcraftApp, items: &[menus::Item]) -> Vec<Node> {
        let nodes = items
            .iter()
            .filter_map(|it| menus::entry(app, it))
            .map(|e| match e {
                Entry::Sep => Node::Separator,
                Entry::Header(h) => Node::Header(h.to_string()),
                Entry::Sub(label, children) => Node::Submenu { label: label.to_string(), children: level(app, children) },
                Entry::Item(row) => Node::Item(Item {
                    command: row.command.map(|(id, _)| id),
                    params: row.command.map_or(Value::Null, |(_, p)| p.clone()),
                    source: row.source,
                    label: row.label.into_owned(),
                    shortcut: row.shortcut,
                    enabled: row.enabled,
                    checked: row.checked,
                }),
            })
            .collect();
        tidy(nodes)
    }
    let menus = tree
        .iter()
        .enumerate()
        .map(|(i, (title, items))| {
            // The first menu is the in-window bar's application menu.
            let role = match *title {
                _ if i == 0 => MenuRole::App,
                "Window" => MenuRole::Window,
                "Help" => MenuRole::Help,
                _ => MenuRole::Normal,
            };
            Menu { title: crate::i18n::t(title).to_string(), role, children: level(app, items) }
        })
        .collect();
    MenuBar { menus }
}

/// Drop separators at the start or end of a level and collapse runs of them, at every depth.
fn tidy(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let n = match n {
            Node::Submenu { label, children } => Node::Submenu { label, children: tidy(children) },
            other => other,
        };
        if matches!(n, Node::Separator) && matches!(out.last(), None | Some(Node::Separator)) {
            continue;
        }
        out.push(n);
    }
    while matches!(out.last(), Some(Node::Separator)) {
        out.pop();
    }
    out
}

/// A hash of everything a native menu can only change by rebuilding: menus, items (command,
/// params, kind: plain or check, shortcut), submenu and header labels. Item labels, enabled and
/// checked are updated in place.
pub fn structure_key(bar: &MenuBar) -> u64 {
    fn walk(nodes: &[Node], h: &mut DefaultHasher) {
        for n in nodes {
            match n {
                Node::Item(it) => ('i', it.command, it.params.to_string(), it.checked.is_some(), it.shortcut).hash(h),
                Node::Separator => '-'.hash(h),
                Node::Header(label) => ('h', label).hash(h),
                Node::Submenu { label, children } => {
                    ('[', label).hash(h);
                    walk(children, h);
                    ']'.hash(h);
                }
                Node::Standard(s) => ('s', s).hash(h),
            }
        }
    }
    let mut h = DefaultHasher::new();
    for m in &bar.menus {
        (&m.title, m.role).hash(&mut h);
        walk(&m.children, &mut h);
    }
    h.finish()
}

// ----------------------------------------------------------------------------- shortcuts

/// Does a shortcut make a key equivalent in the native menu? Only with ⌘ or ⌃, or a function key:
/// bare keys and ⌥/⇧ chords are typing (⌥ types accented letters), which AppKit would otherwise
/// take from a text field.
pub fn native_ok(sc: &KeyboardShortcut) -> bool {
    sc.modifiers.command || sc.modifiers.mac_cmd || sc.modifiers.ctrl || is_function_key(sc.logical_key)
}

pub fn is_function_key(key: Key) -> bool {
    key.name().strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n))
}

/// The input egui's window integration would have made of a press of `sc` (key and modifiers
/// held): the key press and its release, but for ⌘X, ⌘C and ⌘V the Cut, Copy and Paste events
/// (Paste with the clipboard's text, from `paste_text`). A paste's release is left to the window's
/// own key-up: one without a press or a Paste event is how a paste of a picture or a PDF is told
/// apart ([`crate::shortcuts`]).
pub fn key_events(sc: KeyboardShortcut, paste_text: impl FnOnce() -> Option<String>) -> Vec<egui::Event> {
    let key = |pressed| egui::Event::Key { key: sc.logical_key, physical_key: None, pressed, repeat: false, modifiers: sc.modifiers };
    if sc.modifiers.command {
        match sc.logical_key {
            Key::X => return vec![egui::Event::Cut, key(false)],
            Key::C => return vec![egui::Event::Copy, key(false)],
            Key::V => return paste_text().map(|t| t.replace("\r\n", "\n")).filter(|t| !t.is_empty()).map(egui::Event::Paste).into_iter().collect(),
            _ => {}
        }
    }
    vec![key(true), key(false)]
}

// ----------------------------------------------------------------------------- macOS layout

/// A system key equivalent VectorCraft already uses for a command.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub shortcut: &'static str,
    /// What macOS uses it for.
    pub system: &'static str,
    /// The command bound to it.
    pub command: &'static str,
}

pub struct Layout {
    pub bar: MenuBar,
    /// System keys left to commands (logged once).
    pub clashes: Vec<Clash>,
}

/// The Mac menu bar of `bar` (the in-window menus, [`from_tree`]): see the module docs.
/// Language and Appearance have no in-window menu (as in PhotoCraft); when the tree doesn't
/// carry them they are synthesized below, so the App menu keeps them.
pub fn mac_layout(app: &VectorcraftApp, bar: &MenuBar, lang: Lang) -> Layout {
    let mut bar = bar.clone();
    let mut clashes = Vec::new();
    let bound = crate::shortcuts::all_shortcuts();
    // A system item's key, unless a command has it.
    let mut key = |sc: &'static str, system: &'static str| {
        let chord = crate::shortcuts::parse(sc)?;
        match bound.iter().find(|(b, ..)| *b == chord) {
            Some((_, command, _)) => {
                clashes.push(Clash { shortcut: sc, system, command });
                None
            }
            None => Some(sc),
        }
    };
    let hide = Item::system(HIDE, "Hide VectorCraft", key("Ctrl+Cmd+H", "Hide"), lang);
    let hide_others = Item::system(HIDE_OTHERS, "Hide Others", key("Cmd+Alt+H", "Hide Others"), lang);
    let minimize = Item::system(MINIMIZE, "Minimize", key("Ctrl+Cmd+M", "Minimize"), lang);

    // What the application menu takes from the others (every copy: About is in Help too, Settings
    // in Edit as Preferences…).
    let about = take_items(&mut bar, "help.about").into_iter().next();
    let settings = take_items(&mut bar, "edit.preferences").into_iter().next();
    let quit = take_items(&mut bar, "app.quit").into_iter().next();
    // Language and UI Brightness live only in the App menu: taken from the tree when it carries
    // them, else synthesized here (the in-window bar has neither).
    let language = take_submenu(&mut bar, "app.language").or_else(|| Some(language_node(app, lang)));
    let appearance = take_submenu(&mut bar, "window.brightness").or_else(|| Some(appearance_node(app, lang)));
    // Its other items (Join Our Discord) stay where the in-window bar also has them: Help.
    bar.menus.retain(|m| m.role != MenuRole::App);

    let mut app = Vec::new();
    app.extend(about.map(Node::Item));
    app.push(Node::Separator);
    app.extend(settings.map(|s| Node::Submenu { label: tr(lang, "Settings").into(), children: settings_pages(&s, lang) }));
    app.extend(language);
    app.extend(appearance.map(|n| match n {
        // The theme, as Mac apps call it.
        Node::Submenu { children, .. } => Node::Submenu { label: tr_ctx(lang, "theme", "Appearance").into(), children },
        other => other,
    }));
    app.extend([Node::Separator, Node::Standard(Standard::Services), Node::Separator]);
    app.extend([Node::Item(hide), Node::Item(hide_others), Node::Standard(Standard::ShowAll), Node::Separator]);
    // Quit is File › Exit in the window; in the App menu it reads as a Mac Quit item.
    app.extend(quit.map(|mut q| {
        q.label = tr(lang, "Quit VectorCraft").into();
        Node::Item(q)
    }));
    bar.menus.insert(0, Menu { title: APP_NAME.into(), role: MenuRole::App, children: app });

    if let Some(window) = bar.menus.iter_mut().find(|m| m.role == MenuRole::Window) {
        let mut children = vec![Node::Item(minimize), Node::Standard(Standard::Zoom), Node::Separator];
        children.append(&mut window.children);
        children.extend([Node::Separator, Node::Standard(Standard::BringAllToFront)]);
        window.children = children;
    }
    for m in &mut bar.menus {
        m.children = tidy(std::mem::take(&mut m.children));
    }
    // A menu the moves emptied goes away (Help stays: it has the search field).
    bar.menus.retain(|m| !m.children.is_empty() || m.role == MenuRole::Help);
    Layout { bar, clashes }
}

/// Language in the app menu, after Settings: Automatic, then every UI language in its own name,
/// with the `interfaceLanguage` preference checked.
fn language_node(app: &VectorcraftApp, lang: Lang) -> Node {
    let mut children = vec![Node::Item(native_item(app, "Automatic", "app.language", json!({"lang": "auto"}))), Node::Separator];
    children.extend(Lang::all().map(|l| Node::Item(native_item(app, l.name(), "app.language", json!({"lang": l.code()})))));
    Node::Submenu { label: tr(lang, "Language").into(), children }
}

/// Appearance in the app menu (the UI-brightness theme, as Mac apps call it).
fn appearance_node(app: &VectorcraftApp, lang: Lang) -> Node {
    let children =
        Brightness::ALL.iter().map(|b| Node::Item(native_item(app, b.label(), "window.brightness", json!({"brightness": b.id()})))).collect();
    Node::Submenu { label: tr_ctx(lang, "theme", "Appearance").into(), children }
}

/// A native item running `command`, built like [`from_tree`] builds it (translated label,
/// shortcut, enabled and checked as they read now).
fn native_item(app: &VectorcraftApp, label: &'static str, command: &'static str, params: Value) -> Item {
    let item = menus::Item::Cmd(label, command, params);
    match menus::entry(app, &item) {
        Some(Entry::Item(row)) => Item {
            command: row.command.map(|(id, _)| id),
            params: row.command.map_or(Value::Null, |(_, p)| p.clone()),
            source: row.source,
            label: row.label.into_owned(),
            shortcut: row.shortcut,
            enabled: row.enabled,
            checked: row.checked,
        },
        // The commands always show; the fallback keeps the structure.
        _ => Item { command: Some(command), params: Value::Null, source: label, label: label.into(), shortcut: None, enabled: true, checked: None },
    }
}

/// Settings ▸: one item per page of the Preferences dialog (its own list), each opening it there;
/// the first (General…) has Preferences' shortcut (⌘K), as in Illustrator.
fn settings_pages(preferences: &Item, lang: Lang) -> Vec<Node> {
    vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, page)| {
            Node::Item(Item {
                command: preferences.command,
                params: json!({ "category": page }),
                // The page's name, not a "…" label: it runs, rather than asking for its params.
                source: page,
                label: format!("{}…", tr(lang, page)),
                shortcut: preferences.shortcut.filter(|_| i == 0),
                enabled: preferences.enabled,
                checked: None,
            })
        })
        .collect()
}

/// Remove every item running `command`, at any depth; returns them in order.
fn take_items(bar: &mut MenuBar, command: &str) -> Vec<Item> {
    fn walk(nodes: &mut Vec<Node>, command: &str, out: &mut Vec<Item>) {
        let mut kept = Vec::with_capacity(nodes.len());
        for n in std::mem::take(nodes) {
            match n {
                Node::Item(it) if it.command == Some(command) => out.push(it),
                Node::Submenu { label, mut children } => {
                    walk(&mut children, command, out);
                    kept.push(Node::Submenu { label, children });
                }
                other => kept.push(other),
            }
        }
        *nodes = kept;
    }
    let mut out = Vec::new();
    for m in &mut bar.menus {
        walk(&mut m.children, command, &mut out);
    }
    out
}

/// Remove the first submenu that lists `command`, and any other item running it.
fn take_submenu(bar: &mut MenuBar, command: &str) -> Option<Node> {
    fn find(nodes: &mut Vec<Node>, command: &str) -> Option<Node> {
        let lists = |n: &Node| matches!(n, Node::Submenu { children, .. } if children.iter().any(|c| matches!(c, Node::Item(it) if it.command == Some(command))));
        if let Some(i) = nodes.iter().position(lists) {
            return Some(nodes.remove(i));
        }
        nodes.iter_mut().find_map(|n| match n {
            Node::Submenu { children, .. } => find(children, command),
            _ => None,
        })
    }
    let sub = bar.menus.iter_mut().find_map(|m| find(&mut m.children, command));
    take_items(bar, command);
    sub
}

/// VectorCraft's menus as a Mac menu bar, as they show now.
pub fn layout(app: &VectorcraftApp) -> Layout {
    let mut layout = mac_layout(app, &from_tree(app, &menus::menu_tree_named(app.session.prefs.font_names_in_english)), crate::i18n::current());
    if app.file_dialog_open() {
        for menu in &mut layout.bar.menus {
            inert(&mut menu.children);
        }
    }
    layout
}

/// While a file dialog is open (a sheet on the window, [`crate::picks`]) the menu bar does
/// nothing, as under the modal dialog before: what asked for the dialog runs again when it
/// answers, on the document as it was. Hide and Minimize still work.
fn modal_allows(item: &Item) -> bool {
    matches!(item.command, Some(HIDE | HIDE_OTHERS | MINIMIZE))
}

/// Disable every item of `nodes` (and their submenus) [`modal_allows`] doesn't keep.
fn inert(nodes: &mut [Node]) {
    for node in nodes {
        match node {
            Node::Item(item) => item.enabled &= modal_allows(item),
            Node::Submenu { children, .. } => inert(children),
            Node::Separator | Node::Header(_) | Node::Standard(_) => {}
        }
    }
}

// ----------------------------------------------------------------------------- the app's side

/// What the native menu reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An item chosen with the mouse (or the keyboard inside an open menu).
    Click(Item),
    /// A key equivalent pressed: the item's key with the modifiers held, to be handled as the key
    /// press it was.
    Key(KeyboardShortcut),
}

/// A platform's native menu bar (the desktop app implements it for macOS with muda).
pub trait Backend {
    /// Bring the menu in step with `bar`: update labels, enabled and checked in place, or rebuild
    /// when its structure changed.
    fn sync(&mut self, bar: &MenuBar);
    /// What was chosen since the last call. Items the platform runs itself (Services, Zoom, Hide…)
    /// are handled there and not reported.
    fn drain(&mut self) -> Vec<Event>;
}

/// The native menu bar, while VectorCraft uses one (the in-window menus are hidden then).
pub struct NativeMenu {
    backend: Box<dyn Backend>,
    /// Hash of the state the menus were last read in ([`state_hash`]).
    state: Option<u64>,
    clicks: Vec<Item>,
    /// Something was chosen (AppKit may have toggled a check mark itself): read the menus again.
    dirty: bool,
    logged: bool,
}

impl NativeMenu {
    pub fn new(backend: Box<dyn Backend>) -> NativeMenu {
        NativeMenu { backend, state: None, clicks: Vec::new(), dirty: true, logged: false }
    }

    /// The key equivalents pressed since the last frame, for [`key_events`]; clicks wait for
    /// [`run`].
    pub fn take_keys(&mut self) -> Vec<KeyboardShortcut> {
        let mut keys = Vec::new();
        for e in self.backend.drain() {
            match e {
                Event::Key(k) => keys.push(k),
                Event::Click(it) => self.clicks.push(it),
            }
            self.dirty = true;
        }
        keys
    }
}

/// Run the items clicked in the native menu since the last frame, like in-window menu clicks.
pub fn run(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let picking = app.file_dialog_open();
    let Some(menu) = app.services.native_menu.as_mut() else { return };
    for it in std::mem::take(&mut menu.clicks) {
        // A click that raced the menu going inert under a file dialog.
        if picking && !modal_allows(&it) {
            continue;
        }
        match it.command {
            Some(MINIMIZE) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Some(command) => {
                let (id, p) = menus::click_target(it.source, command, &it.params);
                menus::invoke_from_system_menu(app, ctx, &id, p);
            }
            None => {}
        }
    }
}

/// Bring the native menu up to date when anything it shows may have changed: a command ran, the
/// document, its history or selection changed, the user clicked or pressed a key. Reading the rows
/// asks every command whether it is enabled, so frames that only animate or scroll skip it.
pub fn sync(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(menu) = app.services.native_menu.as_ref() else { return };
    let input = ctx
        .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: false, .. })));
    let state = state_hash(app, input.then_some(app.frame));
    if menu.state == Some(state) && !menu.dirty {
        return;
    }
    let layout = layout(app);
    let Some(menu) = app.services.native_menu.as_mut() else { return };
    if !menu.logged {
        for c in &layout.clashes {
            log::info!("menu: {} is {} on macOS and {} in VectorCraft, which keeps it", c.shortcut, c.system, c.command);
        }
        menu.logged = true;
    }
    menu.backend.sync(&layout.bar);
    menu.state = Some(state);
    menu.dirty = false;
}

/// What the menus' rows depend on, cheaply (no allocation): commands run, the active document,
/// its revision (edits, selection and view changes), history and clipboard, the language,
/// shortcuts, plug-ins and an open file dialog. `input` forces a change on frames with a click or
/// a key press, which covers what the hash doesn't list (panels shown, a tool picked).
fn state_hash(app: &VectorcraftApp, input: Option<u64>) -> u64 {
    let mut h = DefaultHasher::new();
    input.hash(&mut h);
    app.run_count.hash(&mut h);
    let s = &app.session;
    (s.journal.len(), s.active_index(), s.clipboard.is_empty(), s.tool_composing()).hash(&mut h);
    if let Some(d) = s.active() {
        (d.uid, d.revision, d.history.undo.len(), d.history.redo.len(), d.transparency_grid, d.print_tiling).hash(&mut h);
    }
    (app.system_paste, app.last_effect.is_some(), app.ui.recent_files.len(), app.ui.recent_files.first(), app.recent_fonts().len()).hash(&mut h);
    app.file_dialog_open().hash(&mut h);
    (crate::i18n::current().code(), menus::plugin_revision()).hash(&mut h);
    crate::shortcut_editor::GENERATION.load(std::sync::atomic::Ordering::Relaxed).hash(&mut h);
    h.finish()
}
