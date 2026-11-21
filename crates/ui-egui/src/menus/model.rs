//! The menu model: the tree built into a `craft::menu::MenuBar`, the commands' states and the
//! catalog the framework resolves menu rows, shortcuts and the palette from. It is rebuilt only
//! when what the rows depend on changes ([`state_key`]).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;

use craft::keymap::{CommandDef, CommandRef, Mods, Scope};
use craft::ui::menu::Alternate;
use craft::menu::{Item, Labeled, LayoutOptions, Menu, MenuBar, MenuRole, Node, Target, layout};
use craft::tracked::Tracked;
use craft::ui::runtime::commands::{AppCatalog, CommandGate, CommandInput, CommandState, CommandStates};
use craft::ui::runtime::windows::WindowHost;
use serde_json::{Value, json};

use super::labels::holds_plugin_commands;
use super::{TODO, display_label, menu_tree_named, plugin_revision, shown_label, shown_state, why_off};
use crate::VectorcraftApp;

/// The application's name in the application menu and in the layout's own rows.
pub(crate) const APP_NAME: &str = "VectorCraft";

/// The English labels of the rows a Mac layout writes itself, which the catalogs translate.
pub(crate) const MAC_LABELS: &[&str] =
    &["Settings", "Appearance", "Services", "Hide VectorCraft", "Hide Others", "Show All", "Minimize", "Zoom", "Bring All to Front"];

/// The tooltip of a row that is not built yet.
const TODO_TOOLTIP: &str = "Coming soon — tracked in the parity plan";

/// The menus as the frame reads them.
#[derive(Default)]
pub(crate) struct Menus {
    key: Option<u64>,
    target: Option<Target>,
    /// Shown title or command key to its English text, for the bar's mnemonics.
    english: HashMap<String, String>,
    /// The menus as built, before a layout.
    pub(crate) bar: MenuBar,
    /// `bar` laid out for the shell's target: what the in-window bar shows.
    pub(crate) laid: MenuBar,
    /// The states of the catalogued commands and of every menu item's reference.
    pub(crate) states: Tracked<CommandStates>,
    catalog: Option<AppCatalog>,
}

impl Menus {
    /// The English text of a row or title the bar names, for its mnemonic hooks.
    pub(crate) fn english_of(&self, labeled: Labeled<'_>) -> Option<String> {
        let key = if let Labeled::Command(command) = labeled {
            command.key()
        } else if let Labeled::Title(title) = labeled {
            title.to_string()
        } else {
            return None;
        };
        self.english.get(&key).cloned()
    }
}

/// What the Home screen remembers when it opens (`app.home`): the active document and the
/// document count. When either changes, the Home screen gives way to the document.
pub(crate) fn home_key(app: &VectorcraftApp) -> (Option<u64>, usize) {
    (app.session.active().map(|d| d.uid), app.session.documents().len())
}

/// Whether the Home screen is up: chosen with the Home button (`app.home`), or no document is open
/// and Preferences › General › Show The Home Screen When No Documents Are Open is on (#394). Off,
/// an app with no document shows an empty window, and the Home button still opens the screen.
pub(crate) fn home_showing(app: &VectorcraftApp) -> bool {
    app.ui.home.is_some() || (app.session.active().is_none() && app.session.prefs.show_home_screen)
}

/// A label the layout writes itself, in the UI language: `{app}` is filled in before the lookup
/// because the catalog's rows read `Hide VectorCraft`, and the layout fills it in only afterwards.
fn translate_label(english: &'static str) -> String {
    let source = english.replace("{app}", APP_NAME);
    crate::i18n::tr(crate::i18n::current(), &source).to_owned()
}

/// What the menus' rows depend on, cheaply (no allocation): commands run, the active document,
/// its revision (edits, selection and view changes), history and clipboard, the language,
/// shortcuts, plug-ins, the Scripts folder's listing, an open file dialog and the fonts (Type › Font lists the loaded fonts
/// until the installed ones are scanned, then all of them, #5483). `input` forces a change on frames with a click or
/// a key press, which covers what the hash doesn't list (panels shown, a tool picked). While an
/// interaction is open (for example a move, scale or shape drag, a dialog's or the font menu's
/// preview, or a typing session), the revision, which each of its previews changes, is left out:
/// the hash changes as the interaction begins and as it ends.
pub(crate) fn state_key(app: &VectorcraftApp, input: Option<u64>) -> u64 {
    let mut h = DefaultHasher::new();
    input.hash(&mut h);
    app.run_count.hash(&mut h);
    let s = &app.session;
    (s.journal.len(), s.active_index(), s.clipboard.is_empty(), s.tool_composing()).hash(&mut h);
    if let Some(d) = s.active() {
        let revision = d.interaction.is_none().then_some(d.revision);
        (d.uid, revision, d.history.undo.len(), d.history.redo.len(), d.transparency_grid, d.print_tiling).hash(&mut h);
    }
    (app.system_paste, app.last_effect.is_some(), app.ui.recent_files.len(), app.ui.recent_files.first(), app.recent_fonts().len()).hash(&mut h);
    app.file_dialog_open().hash(&mut h);
    (crate::i18n::current().code(), plugin_revision(), crate::scripts_menu::revision(), crate::dialogs::proof_setup::saved_revision()).hash(&mut h);
    crate::shortcut_editor::GENERATION.load(Ordering::Relaxed).hash(&mut h);
    let fonts = vectorcraft_text::FontDb::global();
    (fonts.generation(), fonts.installed_fonts_ready()).hash(&mut h);
    // What the in-window labels read.
    let (prefs, v) = (&s.prefs, &app.ui.view);
    (prefs.font_preview, prefs.font_samples_on_white, prefs.font_names_in_english, crate::workspaces::current_name(&app.ui)).hash(&mut h);
    (v.edges, v.templates, v.corner_widgets, v.shape_widgets, v.text_threads, v.hidden_chars, v.gradient_annotator).hash(&mut h);
    (v.live_paint_gaps, v.artboards, v.rulers, v.video_rulers, v.bounding_box, v.guides, v.grid).hash(&mut h);
    (app.outline_view(), s.guides_locked(), s.slices_hidden()).hash(&mut h);
    crate::panels::actions::menu::key(app).hash(&mut h);
    crate::panels::script_console::key(app).hash(&mut h);
    h.finish()
}

/// Brings the menus up to date once per frame, before the frame's input: the model is built again
/// when the rows' state ([`state_key`]) or the shell's target changed.
pub(crate) fn refresh(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let target = Target::of_shell(&WindowHost::of(ctx).shell());
    let pressed = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { pressed: true, .. })));
    let key = state_key(app, pressed.then(|| ctx.cumulative_pass_nr()));
    if app.menus.key == Some(key) && app.menus.target == Some(target) {
        return;
    }
    rebuild(app, target);
    app.menus.key = Some(key);
    app.menus.target = Some(target);
}

/// The command input of the frame: the catalog, the states and the gate that refuses commands
/// while a file dialog is open or the shortcuts dialog records a chord.
pub(crate) fn command_input(app: &VectorcraftApp) -> CommandInput {
    let blocked = app.file_dialog_open() || crate::shortcut_editor::is_recording(app);
    let mut input = CommandInput::new().states(&app.menus.states);
    if let Some(catalog) = &app.menus.catalog {
        input = input.catalog(catalog.clone());
    }
    input.gate(CommandGate::new(blocked, |blocked: &bool, _: &CommandRef| !*blocked))
}

fn role(title: &str) -> MenuRole {
    match title {
        "VectorCraft" => MenuRole::App,
        "File" => MenuRole::File,
        "Edit" => MenuRole::Edit,
        "Window" => MenuRole::Window,
        "Help" => MenuRole::Help,
        _ => MenuRole::Other,
    }
}

/// The hash of what the catalog's rows depend on, which moves its generation.
fn generation() -> u64 {
    let mut h = DefaultHasher::new();
    let fonts = vectorcraft_text::FontDb::global();
    (crate::i18n::current().code(), plugin_revision(), crate::scripts_menu::revision(), crate::shortcut_editor::GENERATION.load(Ordering::Relaxed), fonts.generation())
        .hash(&mut h);
    h.finish()
}

fn rebuild(app: &mut VectorcraftApp, target: Target) {
    let mut build = Build { app, lang: crate::i18n::current(), states: CommandStates::new(), english: HashMap::new(), refs: Vec::new() };
    let menus: Vec<Menu> = menu_tree_named(app.session.prefs.font_names_in_english)
        .into_iter()
        .map(|(title, nodes)| {
            let shown = build.title(title, true);
            Menu::new(shown, build.nodes(nodes)).role(role(title))
        })
        .collect();
    let Build { mut states, english, refs, .. } = build;

    let defs = catalog_defs(app, &refs, &mut states);
    crate::panels::actions::menu::states(app, &mut states);
    crate::panels::script_console::states(app, &mut states);
    crate::panels::panel_menu_states(app, &mut states);

    let bar = MenuBar::new(menus);
    let options = LayoutOptions::new(APP_NAME).label(&translate_label);
    let laid = layout(&bar, target, &options).bar;
    match AppCatalog::new(defs, generation()) {
        Ok(catalog) => app.menus.catalog = Some(catalog),
        // A refused catalog is a bug in the rows; the last good one keeps the commands working.
        Err(e) => eprintln!("menu catalog refused: {e:?}"),
    }
    app.menus.english = english;
    app.menus.bar = bar;
    app.menus.laid = laid;
    app.menus.states.edit(|s| *s = states);
}

/// The catalog's rows: the keymap's, with icons, then a row for every menu reference the keymap
/// does not name (by its key or its bare id), and the state of every row not set by the tree.
// PLAN: the Shift+Space row for MomentaryCommands::RotateView (S-CV8) is the canvas area's: that
// command is not in the app scope, which `AppCatalog` accepts.
// PLAN: ActionsPanelCommands' defs (area plan X6) are menus-w18's; their signature is not visible here.
fn catalog_defs(app: &VectorcraftApp, refs: &[(CommandRef, String, bool)], states: &mut CommandStates) -> Vec<CommandDef> {
    let snapshot = crate::shortcut_editor::snapshot();
    let mut defs: Vec<CommandDef> = crate::keymap::catalog_defs(crate::keymap::platform(), &snapshot)
        .into_iter()
        .map(|d| match crate::panels::command_icon(&d.command.id) {
            Some(icon) => d.with_icon(icon),
            None => d,
        })
        .collect();
    let mut known: HashSet<String> = defs.iter().map(|d| d.command.key()).collect();
    for (command, label, checkable) in refs {
        if known.contains(&command.key()) || known.contains(&command.id) {
            continue;
        }
        known.insert(command.key());
        let def = CommandDef::new(command.clone(), label.clone(), Scope::app());
        defs.push(if *checkable { def.toggle() } else { def });
    }
    for d in &defs {
        if states.get(&d.command).is_some() {
            continue;
        }
        let (id, params) = (&d.command.id, &d.command.params);
        let (enabled, checked) = shown_state(app, id, params).unwrap_or((false, None));
        let mut state = CommandState::new().enabled(enabled);
        if let Some(on) = checked {
            state = state.checked(on);
        }
        let shown = display_label(app, id, &d.label);
        if shown != d.label {
            state = state.title(shown);
        }
        if let Some(reason) = Some(enabled).filter(|e| !e).and_then(|_| why_off(app, id).flatten()) {
            state = state.hint(reason);
        }
        states.set(d.command.clone(), state);
    }
    defs
}

/// The tree turned into the bar's nodes, with the state of every item it keeps.
struct Build<'a> {
    app: &'a VectorcraftApp,
    lang: crate::i18n::Lang,
    states: CommandStates,
    english: HashMap<String, String>,
    /// Every command reference the tree holds, its English label, and whether it is checkable.
    refs: Vec<(CommandRef, String, bool)>,
}

impl Build<'_> {
    /// `source` as shown: translated when `translate` (a plug-in's or folder's name is not), with
    /// the shown text remembered against its English.
    fn title(&mut self, source: &str, translate: bool) -> String {
        if !translate {
            return source.to_string();
        }
        let shown = crate::i18n::tr(self.lang, source).to_string();
        if shown != source {
            self.english.insert(shown.clone(), source.to_string());
        }
        shown
    }

    fn nodes(&mut self, nodes: Vec<Node>) -> Vec<Node> {
        nodes.into_iter().filter_map(|n| self.node(n)).collect()
    }

    /// `node` as the bar shows it, or `None` while the menus leave it out ([`shown_state`]).
    fn node(&mut self, node: Node) -> Option<Node> {
        Some(match node {
            Node::Item(item) => Node::Item(self.item(item)?),
            Node::Heading(text) => Node::Heading(self.title(&text, true)),
            // A plug-in's submenu shows its name as it is, as its items do, and so does a subfolder of
            // the Scripts folder.
            Node::Submenu { label, children, role, platforms } => {
                let named = holds_plugin_commands(&children) || crate::scripts_menu::is_folder(&children);
                let label = self.title(&label, !named);
                Node::Submenu { label, children: self.nodes(children), role, platforms }
            }
            other => other,
        })
    }

    fn item(&mut self, item: Item) -> Option<Item> {
        let command = item.command.clone();
        if command.id == TODO {
            self.states.set(command, CommandState::new().enabled(false));
            self.refs.push((CommandRef::new(TODO), "Coming Soon".into(), false));
            let tip = crate::i18n::tr(self.lang, TODO_TOOLTIP).to_string();
            return Some(item.tooltip(tip));
        }
        let (id, params) = (command.id.as_str(), &command.params);
        let (enabled, checked) = shown_state(self.app, id, params)?;
        let mut state = CommandState::new().enabled(enabled).title(self.item_title(id, params, &item.label));
        if let Some(on) = checked {
            state = state.checked(on);
        }
        let mut item = item;
        let tip = if id == "ui.expandDialog" && vectorcraft_engine::cmd::expand::has_appearance(&self.app.session) {
            Some(crate::i18n::tr(self.lang, crate::dialogs::expand::appearance_first()).to_string())
        } else if enabled {
            None
        } else {
            why_off(self.app, id).flatten()
        };
        if let Some(tip) = tip {
            state = state.hint(tip.clone());
            if id == "ui.expandDialog" {
                item = item.tooltip(tip);
            }
        }
        self.states.set(command.clone(), state);
        self.refs.push((command.clone(), item.label.clone(), checked.is_some()));
        Some(self.decorate(item, id, params))
    }

    /// The title a row shows: the Settings pages read `Page…`, the other rows their label as the
    /// menus draw it.
    fn item_title(&self, id: &str, params: &Value, label: &str) -> String {
        if id == "edit.preferences"
            && let Some(page) = params.get("category").and_then(Value::as_str)
        {
            return format!("{}…", crate::i18n::tr(self.lang, page));
        }
        shown_label(self.app, id, params, label)
    }

    /// The alternates (a modifier held as the row is chosen) and the font sample of a row.
    fn decorate(&self, item: Item, id: &str, params: &Value) -> Item {
        let run_with = |mods: Mods, p: Value| Alternate::new(mods, CommandRef::with_params(id, p));
        let held = |shift: bool, alt: bool| Mods { cmd: false, ctrl: false, alt, shift };
        match id {
            "object.unlockAll" | "object.showAll" => item.alternate(run_with(held(true, true), json!({ "scope": "group" }))),
            "ui.expandDialog" => item.alternate(run_with(held(false, true), json!({ "immediate": true }))),
            crate::addons::select::link_status::COMMAND => {
                item.alternate(run_with(held(true, false), crate::addons::select::link_status::with_shift(params.clone())))
            }
            "text.setStyle" if self.app.session.prefs.font_preview => match params.get("font").and_then(Value::as_str) {
                Some(font) => item.sample(crate::font_menu::menu_sample(font, self.app.session.prefs.font_samples_on_white)),
                None => item,
            },
            _ => item,
        }
    }
}
